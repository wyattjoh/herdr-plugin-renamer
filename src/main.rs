//! herdr-plugin-renamer
//!
//! A herdr event hook (`pane.agent_status_changed`) that renames a numeric tab
//! from the agent's first prompt, and also renames an auto-generated worktree
//! branch/workspace when the pane is in a linked worktree.
//!
//! The binary runs in two phases:
//!   - HOT  (default): fires on every event. Bails in microseconds from env
//!     vars alone unless an agent just started working, then forks the cold
//!     phase detached and exits.
//!   - COLD (`HERDR_NAMING_PHASE=cold`): does the slow work (poll for the
//!     session, read the first prompt, generate a slug via the engine chain,
//!     rename a numeric tab, and maybe rename branch + workspace).
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
    let tab_id = env::var("HERDR_TAB_ID").unwrap_or_else(|_| eligible.pane_id.clone());
    let marker_key = marker_key_for_tab(&tab_id);

    let state_dir = state_dir();
    let done_marker = done_marker_path(&state_dir, &marker_key);
    if Path::new(&done_marker).exists() {
        debug_log(&format!(
            "hot: done marker exists, bail tab={tab_id} ws={}",
            eligible.workspace_id
        ));
        return;
    }
    let claim_marker = claim_marker_path(&state_dir, &marker_key);
    if claim_is_fresh(&claim_marker) {
        debug_log(&format!(
            "hot: claim fresh, bail tab={tab_id} ws={}",
            eligible.workspace_id
        ));
        return;
    }

    let _ = std::fs::create_dir_all(&state_dir);
    let _ = std::fs::write(&claim_marker, now_secs().to_string());

    debug_log(&format!(
        "hot: eligible ws={} tab={} pane={} label={:?} linked={} -> fork cold",
        eligible.workspace_id,
        tab_id,
        eligible.pane_id,
        eligible.workspace_label,
        eligible.is_linked_worktree
    ));
    spawn_cold_phase(&eligible, &tab_id, &marker_key);
}

/// The slow path, run detached so herdr is never blocked.
fn cold_phase() {
    let pane_id = env::var("HN_PANE_ID").unwrap_or_default();
    let tab_id = env::var("HN_TAB_ID").unwrap_or_default();
    let workspace_id = env::var("HN_WORKSPACE_ID").unwrap_or_default();
    let marker_key = env::var("HN_MARKER_KEY").unwrap_or_else(|_| marker_key_for_tab(&tab_id));
    let checkout_path = env::var("HN_CHECKOUT_PATH")
        .ok()
        .filter(|path| !path.is_empty());
    let is_linked_worktree = env::var("HN_IS_LINKED_WORKTREE").as_deref() == Ok("true");
    let state_dir = state_dir();
    let claim_marker = claim_marker_path(&state_dir, &marker_key);
    let done_marker = done_marker_path(&state_dir, &marker_key);
    debug_log(&format!(
        "cold: start ws={workspace_id} tab={tab_id} pane={pane_id}"
    ));

    // Resolve the native session (with the timing-race poll), then the prompt.
    // On a transient miss, drop the claim so a later event retries.
    let (agent, session_id) =
        match herdr::poll_agent_session(&pane_id, SESSION_POLL_ATTEMPTS, SESSION_POLL_DELAY) {
            Some(session) => session,
            None => {
                debug_log("cold: no agent_session after poll, removing claim");
                let _ = std::fs::remove_file(&claim_marker);
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
            let _ = std::fs::remove_file(&claim_marker);
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
    let slug_file = format!("{state_dir}/{marker_key}.slug");
    let slug = generate_slug(&prompt, Path::new(&slug_file)).unwrap_or_else(|| {
        let slug = slug::fallback_from_prompt(&prompt);
        debug_log(&format!("cold: all engines failed, fallback slug={slug}"));
        slug
    });

    match herdr::tab_label(&tab_id) {
        Some(label) if is_numeric_label(&label) => {
            let ok = herdr::tab_rename(&tab_id, &slug);
            debug_log(&format!("cold: tab {tab_id} {label} -> {slug} ok={ok}"));
        }
        Some(label) => debug_log(&format!("cold: skip tab rename, label={label:?}")),
        None => debug_log(&format!(
            "cold: skip tab rename, label unavailable tab={tab_id}"
        )),
    }

    // Safety re-check: only rename a branch still on the auto `worktree/` name.
    // Only after a successful branch rename do we rename the workspace.
    if is_linked_worktree {
        if let Some(checkout_path) = checkout_path.as_deref() {
            match git::current_branch(checkout_path) {
                Some(current) if current.starts_with("worktree/") => {
                    let branch = compose_branch(resolve_branch_prefix().as_deref(), &slug);
                    let branch_ok = git::rename_current_branch(checkout_path, &branch);
                    debug_log(&format!(
                        "cold: branch {current} -> {branch} ok={branch_ok}"
                    ));
                    if branch_ok {
                        let ok = herdr::workspace_rename(&workspace_id, &slug);
                        debug_log(&format!(
                            "cold: workspace rename ws={workspace_id} -> {slug} ok={ok}"
                        ));
                    } else {
                        debug_log("cold: skip workspace rename, branch rename failed");
                    }
                }
                other => debug_log(&format!(
                    "cold: skip branch/workspace rename, current={other:?}"
                )),
            }
        } else {
            debug_log("cold: skip branch/workspace rename, checkout path unavailable");
        }
    } else {
        debug_log("cold: skip branch/workspace rename, not a linked worktree");
    }

    let _ = std::fs::remove_file(&claim_marker);
    let _ = std::fs::write(&done_marker, now_secs().to_string());
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
fn spawn_cold_phase(eligible: &context::Eligible, tab_id: &str, marker_key: &str) {
    let exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return,
    };

    let mut command = Command::new(exe);
    command
        .env("HERDR_NAMING_PHASE", "cold")
        .env("HN_PANE_ID", &eligible.pane_id)
        .env("HN_TAB_ID", tab_id)
        .env("HN_WORKSPACE_ID", &eligible.workspace_id)
        .env(
            "HN_WORKSPACE_LABEL",
            eligible.workspace_label.as_deref().unwrap_or(""),
        )
        .env(
            "HN_CHECKOUT_PATH",
            eligible.checkout_path.as_deref().unwrap_or(""),
        )
        .env(
            "HN_IS_LINKED_WORKTREE",
            eligible.is_linked_worktree.to_string(),
        )
        .env("HN_MARKER_KEY", marker_key)
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

fn marker_key_for_tab(tab_id: &str) -> String {
    let safe = tab_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("tab-{safe}")
}

fn claim_marker_path(state_dir: &str, marker_key: &str) -> String {
    format!("{state_dir}/{marker_key}.claim")
}

fn done_marker_path(state_dir: &str, marker_key: &str) -> String {
    format!("{state_dir}/{marker_key}.done")
}

fn is_numeric_label(label: &str) -> bool {
    let label = label.trim();
    !label.is_empty() && label.chars().all(|ch| ch.is_ascii_digit())
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
    use super::{compose_branch, is_numeric_label, marker_key_for_tab};

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

    #[test]
    fn numeric_label_requires_digits() {
        assert!(is_numeric_label("1"));
        assert!(is_numeric_label("42"));
        assert!(is_numeric_label(" 7 "));
        assert!(!is_numeric_label(""));
        assert!(!is_numeric_label("tab-1"));
        assert!(!is_numeric_label("renamer-test"));
    }

    #[test]
    fn marker_key_is_safe_for_tab_ids() {
        assert_eq!(marker_key_for_tab("w5V:t1"), "tab-w5V_t1");
    }
}
