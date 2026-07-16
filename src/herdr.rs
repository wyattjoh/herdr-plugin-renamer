//! Calls back into herdr over its CLI: reading a pane's native agent session
//! (with a short poll for the documented timing race) and renaming Herdr labels.

use std::env;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

const METADATA_SOURCE: &str = "plugin:herdr-plugin-renamer";

fn herdr_bin() -> String {
    env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_string())
}

/// `herdr pane get <pane_id>` returns JSON by default. Extract the agent label
/// and native session id from `agent_session`. Returns `None` when the session
/// has not been reported yet.
fn pane_agent_session(pane_id: &str) -> Option<(String, String)> {
    let output = Command::new(herdr_bin())
        .args(["pane", "get", pane_id])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    // The CLI wraps the pane in a `{"result":{"pane":{...}}}` envelope. Accept
    // the wrapped shape first, then fall back to unwrapped variants.
    let session = value
        .pointer("/result/pane/agent_session")
        .or_else(|| value.pointer("/pane/agent_session"))
        .or_else(|| value.get("agent_session"))?;

    let agent = match session.get("agent").and_then(|a| a.as_str()) {
        Some(a) => a.to_string(),
        // Older builds emit only `source` (e.g. "herdr:claude").
        None => session
            .get("source")
            .and_then(|s| s.as_str())
            .map(|s| s.trim_start_matches("herdr:").to_string())?,
    };
    let value = session.get("value").and_then(|v| v.as_str())?.to_string();
    Some((agent, value))
}

/// Poll `pane get` for the session id. `pane.agent_status_changed` can fire
/// before herdr has received the session from the integration hook, so we retry
/// briefly before giving up.
pub fn poll_agent_session(
    pane_id: &str,
    attempts: u32,
    delay: Duration,
) -> Option<(String, String)> {
    for attempt in 0..attempts {
        if let Some(session) = pane_agent_session(pane_id) {
            return Some(session);
        }
        if attempt + 1 < attempts {
            sleep(delay);
        }
    }
    None
}

/// `herdr workspace rename <workspace_id> <label>`.
pub fn workspace_rename(workspace_id: &str, label: &str) -> bool {
    Command::new(herdr_bin())
        .args(["workspace", "rename", workspace_id, label])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `herdr pane rename <pane_id> <label>`.
pub fn pane_rename(pane_id: &str, label: &str) -> bool {
    Command::new(herdr_bin())
        .args(["pane", "rename", pane_id, label])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Publish the generated task name for custom Agent sidebar rows via `$task`.
pub fn pane_report_task(pane_id: &str, task: &str) -> bool {
    report_task_metadata("pane", pane_id, task)
}

/// Publish the generated task name for custom Space sidebar rows via `$task`.
pub fn workspace_report_task(workspace_id: &str, task: &str) -> bool {
    report_task_metadata("workspace", workspace_id, task)
}

fn report_task_metadata(resource: &str, resource_id: &str, task: &str) -> bool {
    let token = format!("task={task}");
    Command::new(herdr_bin())
        .args([
            resource,
            "report-metadata",
            resource_id,
            "--source",
            METADATA_SOURCE,
            "--token",
            &token,
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
