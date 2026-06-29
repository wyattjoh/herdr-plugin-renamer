//! herdr-plugin-naming
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
//!     session, read the first prompt, call Codex, rename branch + workspace).
//!
//! Every path exits 0 (fail open) so the hook never wedges herdr.

mod codex;
mod context;
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
        return;
    }

    let _ = std::fs::create_dir_all(&state_dir);
    let _ = std::fs::write(&marker, now_secs().to_string());

    spawn_cold_phase(&eligible);
}

/// The slow path, run detached so herdr is never blocked.
fn cold_phase() {
    let pane_id = env::var("HN_PANE_ID").unwrap_or_default();
    let workspace_id = env::var("HN_WORKSPACE_ID").unwrap_or_default();
    let checkout_path = env::var("HN_CHECKOUT_PATH").unwrap_or_default();
    let marker = marker_path(&state_dir(), &workspace_id);

    // Resolve the native session (with the timing-race poll), then the prompt.
    // On a transient miss, drop the claim so a later event retries.
    let (agent, session_id) =
        match herdr::poll_agent_session(&pane_id, SESSION_POLL_ATTEMPTS, SESSION_POLL_DELAY) {
            Some(session) => session,
            None => {
                let _ = std::fs::remove_file(&marker);
                return;
            }
        };

    let prompt = match transcript::read_first_prompt(&agent, &session_id) {
        Some(prompt) => prompt,
        None => {
            let _ = std::fs::remove_file(&marker);
            return;
        }
    };

    // Name it: Codex first, deterministic local fallback otherwise.
    let slug_file = format!("{}/{}.slug", state_dir(), workspace_id);
    let slug = codex::generate_slug(&prompt, Path::new(&slug_file))
        .unwrap_or_else(|| slug::fallback_from_prompt(&prompt));

    // Safety re-check: only rename a branch still on the auto `worktree/` name,
    // so a manual rename racing us is never clobbered.
    if let Some(current) = git::current_branch(&checkout_path) {
        if current.starts_with("worktree/") {
            git::rename_current_branch(&checkout_path, &format!("wyattjoh/{slug}"));
        }
    }
    herdr::workspace_rename(&workspace_id, &slug);

    // Keep the marker as a "done" record (now older than CLAIM_TTL over time,
    // but the eligibility gate already bails once the label is renamed).
    let _ = std::fs::write(&marker, now_secs().to_string());
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
