//! herdr-plugin-renamer
//!
//! A herdr event hook (`pane.agent_status_changed`) that renames an
//! auto-generated worktree branch and its workspace from the agent's first
//! prompt.
//!
//! The binary runs in two phases:
//!   - HOT  (default): fires on every event. Bails in microseconds from env
//!     vars alone unless an auto-worktree agent just started working, then
//!     forks the cold phase detached and exits.
//!   - COLD (`HERDR_NAMING_PHASE=cold`): does the slow work (poll for the
//!     session, read the first prompt, generate a slug via the engine chain,
//!     rename branch + workspace).
//!
//! Every path exits 0 (fail open) so the hook never wedges herdr.

mod codex;
mod context;
mod engine;
#[cfg(target_os = "macos")]
mod foundation;
mod git;
mod herdr;
mod slug;
mod transcript;

use std::env;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A claim marker younger than this is treated as "cold phase in flight" so we
/// don't launch a second Codex call for the same workspace. Older markers are
/// considered stale (e.g. a crashed cold phase) and may be reclaimed.
const CLAIM_TTL: Duration = Duration::from_secs(120);
const SESSION_POLL_ATTEMPTS: u32 = 12;
const SESSION_POLL_DELAY: Duration = Duration::from_millis(500);
const PROMPT_POLL_ATTEMPTS: u32 = 20;
const PROMPT_POLL_DELAY: Duration = Duration::from_millis(750);

fn main() {
    if env::var("HERDR_NAMING_PHASE").as_deref() == Ok("cold") {
        cold_phase();
    } else {
        hot_phase();
    }
}

/// Cheap gate on every event. Reads only env vars; no subprocess, no socket.
fn hot_phase() {
    let event_json = match env::var("HERDR_PLUGIN_EVENT_JSON") {
        Ok(value) => value,
        Err(_) => return,
    };
    let context_json = env::var("HERDR_PLUGIN_CONTEXT_JSON").unwrap_or_default();

    let eligible = match context::evaluate(&event_json, &context_json) {
        Some(eligible) => eligible,
        None => return,
    };

    let state_dir = state_dir();
    let marker = marker_path(&state_dir, &eligible.workspace_id);
    if claim_is_fresh(&marker) {
        debug_log(&format!(
            "hot: claim fresh, bail ws={}",
            eligible.workspace_id
        ));
        return;
    }

    let _ = std::fs::create_dir_all(&state_dir);
    let _ = std::fs::write(&marker, now_secs().to_string());

    debug_log(&format!(
        "hot: eligible ws={} pane={} label={} -> fork cold",
        eligible.workspace_id, eligible.pane_id, eligible.workspace_label
    ));
    spawn_cold_phase(&eligible);
}

/// The slow path, run detached so herdr is never blocked.
fn cold_phase() {
    let pane_id = env::var("HN_PANE_ID").unwrap_or_default();
    let workspace_id = env::var("HN_WORKSPACE_ID").unwrap_or_default();
    let checkout_path = env::var("HN_CHECKOUT_PATH").unwrap_or_default();
    let marker = marker_path(&state_dir(), &workspace_id);
    debug_log(&format!("cold: start ws={workspace_id} pane={pane_id}"));

    // Resolve the native session (with the timing-race poll), then the prompt.
    // On a transient miss, drop the claim so a later event retries.
    let (agent, session_id) =
        match herdr::poll_agent_session(&pane_id, SESSION_POLL_ATTEMPTS, SESSION_POLL_DELAY) {
            Some(session) => session,
            None => {
                debug_log("cold: no agent_session after poll, removing claim");
                let _ = std::fs::remove_file(&marker);
                return;
            }
        };
    debug_log(&format!("cold: session agent={agent} id={session_id}"));

    // Poll for the first prompt, not just read once. Claude reports its session
    // id at SessionStart (before the prompt is submitted) and flushes the user
    // line a beat after the pane flips to `working`, so a single read can miss
    // it. Since the agent then stays `working` with no new event to retry on, we
    // must wait here rather than bail.
    let prompt = match poll_first_prompt(&agent, &session_id) {
        Some(prompt) => prompt,
        None => {
            debug_log("cold: no first prompt after poll, removing claim");
            let _ = std::fs::remove_file(&marker);
            return;
        }
    };
    debug_log(&format!(
        "cold: first prompt ({} chars): {}",
        prompt.chars().count(),
        prompt.chars().take(80).collect::<String>()
    ));

    // Name it: walk the engine chain (on-device first by default, Codex
    // fallback), then a deterministic local slug if every engine fails.
    let slug_file = format!("{}/{}.slug", state_dir(), workspace_id);
    let slug = generate_slug(&prompt, Path::new(&slug_file)).unwrap_or_else(|| {
        let slug = slug::fallback_from_prompt(&prompt);
        debug_log(&format!("cold: all engines failed, fallback slug={slug}"));
        slug
    });

    // Safety re-check: only rename a branch still on the auto `worktree/` name,
    // so a manual rename racing us is never clobbered.
    let branch = compose_branch(resolve_branch_prefix().as_deref(), &slug);
    match git::current_branch(&checkout_path) {
        Some(current) if current.starts_with("worktree/") => {
            let ok = git::rename_current_branch(&checkout_path, &branch);
            debug_log(&format!("cold: branch {current} -> {branch} ok={ok}"));
        }
        other => debug_log(&format!("cold: skip branch rename, current={other:?}")),
    }
    let ok = herdr::workspace_rename(&workspace_id, &slug);
    debug_log(&format!(
        "cold: workspace rename ws={workspace_id} -> {slug} ok={ok}"
    ));

    // Keep the marker as a "done" record (now older than CLAIM_TTL over time,
    // but the eligibility gate already bails once the label is renamed).
    let _ = std::fs::write(&marker, now_secs().to_string());
}

/// Walk the engine chain selected by `HERDR_NAMING_ENGINE`, returning the first
/// slug an engine produces. `None` means every engine in the chain failed (so
/// the caller uses the deterministic local fallback).
fn generate_slug(prompt: &str, slug_file: &Path) -> Option<String> {
    let selection = env::var("HERDR_NAMING_ENGINE").ok();
    for eng in engine::engine_chain(selection.as_deref()) {
        let result = match eng {
            #[cfg(target_os = "macos")]
            engine::Engine::Foundation => foundation::generate_slug(prompt),
            engine::Engine::Codex => codex::generate_slug(prompt, slug_file),
        };
        match result {
            Some(slug) => {
                debug_log(&format!("cold: {eng:?} slug={slug}"));
                return Some(slug);
            }
            None => debug_log(&format!("cold: {eng:?} produced no slug")),
        }
    }
    None
}

/// Join an optional branch prefix and the slug into the final branch name.
/// Trailing/leading slashes and surrounding whitespace on the prefix are
/// trimmed; an empty or whitespace-only prefix yields the bare slug.
fn compose_branch(prefix: Option<&str>, slug: &str) -> String {
    match prefix
        .map(|p| p.trim().trim_matches('/'))
        .filter(|p| !p.is_empty())
    {
        Some(prefix) => format!("{prefix}/{slug}"),
        None => slug.to_string(),
    }
}

/// Resolve the branch prefix, in priority order: the `HERDR_NAMING_BRANCH_PREFIX`
/// env var (override, incl. set-empty to force no prefix), then a `branch-prefix`
/// file in the per-plugin config dir (`HERDR_PLUGIN_CONFIG_DIR`), else `None` for
/// no prefix. The config file is the install-friendly path: it does not depend on
/// the environment herdr was launched with.
fn resolve_branch_prefix() -> Option<String> {
    if let Ok(prefix) = env::var("HERDR_NAMING_BRANCH_PREFIX") {
        return Some(prefix);
    }
    let dir = env::var("HERDR_PLUGIN_CONFIG_DIR").ok()?;
    std::fs::read_to_string(format!("{dir}/branch-prefix")).ok()
}

/// Retry `read_first_prompt` until the transcript has the user's first message
/// or we exhaust the attempts. Covers the lag between the pane reporting
/// `working` and the agent flushing the first user line to its transcript.
fn poll_first_prompt(agent: &str, session_id: &str) -> Option<String> {
    for attempt in 0..PROMPT_POLL_ATTEMPTS {
        if let Some(prompt) = transcript::read_first_prompt(agent, session_id) {
            return Some(prompt);
        }
        if attempt + 1 < PROMPT_POLL_ATTEMPTS {
            std::thread::sleep(PROMPT_POLL_DELAY);
        }
    }
    None
}

/// Re-exec ourselves in the cold phase, detached into a new session so it
/// survives the hot process exiting and any herdr process-group cleanup.
fn spawn_cold_phase(eligible: &context::Eligible) {
    let exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return,
    };

    let mut command = Command::new(exe);
    command
        .env("HERDR_NAMING_PHASE", "cold")
        .env("HN_PANE_ID", &eligible.pane_id)
        .env("HN_WORKSPACE_ID", &eligible.workspace_id)
        .env("HN_WORKSPACE_LABEL", &eligible.workspace_label)
        .env("HN_CHECKOUT_PATH", &eligible.checkout_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // SAFETY: setsid only detaches the child into a new session; it does not
    // touch this process's memory and is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    let _ = command.spawn();
}

fn state_dir() -> String {
    env::var("HERDR_PLUGIN_STATE_DIR").unwrap_or_else(|_| "/tmp".to_string())
}

fn marker_path(state_dir: &str, workspace_id: &str) -> String {
    format!("{state_dir}/{workspace_id}.claim")
}

/// True when a claim marker exists and is younger than `CLAIM_TTL`.
fn claim_is_fresh(marker: &str) -> bool {
    let metadata = match std::fs::metadata(marker) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    let modified = match metadata.modified() {
        Ok(modified) => modified,
        Err(_) => return true,
    };
    match modified.elapsed() {
        Ok(age) => age < CLAIM_TTL,
        // Can't read the age: assume fresh and bail rather than double-fire.
        Err(_) => true,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append a diagnostic line to `<state_dir>/debug.log`. Only called on the rare
/// eligible/cold paths, so it never costs the hot-path bail anything. The cold
/// phase runs detached with stderr to /dev/null, so a file is the only way to
/// see what it did.
fn debug_log(message: &str) {
    let dir = state_dir();
    let _ = std::fs::create_dir_all(&dir);
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{dir}/debug.log"))
    {
        let _ = writeln!(
            file,
            "{} [pid {}] {}",
            now_secs(),
            std::process::id(),
            message
        );
    }
}

#[cfg(test)]
mod tests {
    use super::compose_branch;

    #[test]
    fn no_prefix_is_bare_slug() {
        assert_eq!(compose_branch(None, "add-dark-mode"), "add-dark-mode");
        assert_eq!(compose_branch(Some(""), "add-dark-mode"), "add-dark-mode");
        assert_eq!(
            compose_branch(Some("   "), "add-dark-mode"),
            "add-dark-mode"
        );
    }

    #[test]
    fn prefix_is_joined_with_a_slash() {
        assert_eq!(
            compose_branch(Some("wyattjoh"), "add-dark-mode"),
            "wyattjoh/add-dark-mode"
        );
    }

    #[test]
    fn surrounding_slashes_and_whitespace_are_trimmed() {
        assert_eq!(
            compose_branch(Some("  /wyattjoh/  "), "add-dark-mode"),
            "wyattjoh/add-dark-mode"
        );
    }

    #[test]
    fn internal_slashes_in_prefix_are_kept() {
        assert_eq!(
            compose_branch(Some("team/wyatt"), "add-dark-mode"),
            "team/wyatt/add-dark-mode"
        );
    }
}
