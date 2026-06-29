//! Parsing and eligibility for the two env JSON blobs herdr passes to an event
//! hook. The whole hot-path bail decision is made here from env vars alone, with
//! no subprocess or socket call.

use serde::Deserialize;

/// `HERDR_PLUGIN_EVENT_JSON` for a `pane.agent_status_changed` event.
#[derive(Debug, Deserialize)]
struct EventEnvelope {
    data: EventData,
}

#[derive(Debug, Deserialize)]
struct EventData {
    pane_id: String,
    workspace_id: String,
    agent_status: String,
}

/// `HERDR_PLUGIN_CONTEXT_JSON`. Only the fields we need are modelled; the rest
/// are ignored by serde.
#[derive(Debug, Deserialize)]
struct Context {
    #[serde(default)]
    workspace_label: Option<String>,
    #[serde(default)]
    worktree: Option<Worktree>,
}

#[derive(Debug, Deserialize)]
struct Worktree {
    #[serde(default)]
    checkout_path: Option<String>,
    #[serde(default)]
    is_linked_worktree: bool,
}

/// A pane that passed the cheap eligibility gate: an auto-generated herdr
/// worktree whose agent just transitioned to `working`.
#[derive(Debug, PartialEq, Eq)]
pub struct Eligible {
    pub pane_id: String,
    pub workspace_id: String,
    pub workspace_label: String,
    pub checkout_path: String,
}

/// The fast bail. Returns `Some(Eligible)` only when:
///  - the new agent status is `working`, and
///  - the pane's workspace is a linked herdr worktree, and
///  - the workspace label still has the auto-generated `worktree-` prefix
///    (so we never clobber a name the user already set), and
///  - we have a checkout path to run git against.
///
/// Because a successful rename changes the label away from `worktree-`, this
/// gate is self-idempotent: re-fired events bail here once the rename is done.
pub fn evaluate(event_json: &str, context_json: &str) -> Option<Eligible> {
    let event: EventEnvelope = serde_json::from_str(event_json).ok()?;
    if event.data.agent_status != "working" {
        return None;
    }

    let context: Context = serde_json::from_str(context_json).ok()?;
    let worktree = context.worktree?;
    if !worktree.is_linked_worktree {
        return None;
    }

    let label = context.workspace_label?;
    if !label.starts_with("worktree-") {
        return None;
    }
    let checkout_path = worktree.checkout_path?;

    Some(Eligible {
        pane_id: event.data.pane_id,
        workspace_id: event.data.workspace_id,
        workspace_label: label,
        checkout_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX_ELIGIBLE: &str = r#"{
        "workspace_id":"w4B",
        "workspace_label":"worktree-silver-field-3fd7",
        "worktree":{"checkout_path":"/tmp/wt","is_linked_worktree":true}
    }"#;

    fn event(status: &str) -> String {
        format!(
            r#"{{"event":"pane_agent_status_changed","data":{{"type":"pane_agent_status_changed","pane_id":"w4B:p1","workspace_id":"w4B","agent_status":"{status}","agent":"codex"}}}}"#
        )
    }

    #[test]
    fn working_auto_worktree_is_eligible() {
        let e = evaluate(&event("working"), CTX_ELIGIBLE).expect("eligible");
        assert_eq!(e.pane_id, "w4B:p1");
        assert_eq!(e.workspace_id, "w4B");
        assert_eq!(e.workspace_label, "worktree-silver-field-3fd7");
        assert_eq!(e.checkout_path, "/tmp/wt");
    }

    #[test]
    fn non_working_status_bails() {
        assert!(evaluate(&event("idle"), CTX_ELIGIBLE).is_none());
        assert!(evaluate(&event("blocked"), CTX_ELIGIBLE).is_none());
    }

    #[test]
    fn missing_worktree_block_bails() {
        let ctx = r#"{"workspace_id":"w4B","workspace_label":"worktree-x-y-1234"}"#;
        assert!(evaluate(&event("working"), ctx).is_none());
    }

    #[test]
    fn already_renamed_label_bails() {
        let ctx = r#"{
            "workspace_id":"w4B",
            "workspace_label":"oauth-login",
            "worktree":{"checkout_path":"/tmp/wt","is_linked_worktree":true}
        }"#;
        assert!(evaluate(&event("working"), ctx).is_none());
    }

    #[test]
    fn non_linked_worktree_bails() {
        let ctx = r#"{
            "workspace_id":"w4B",
            "workspace_label":"worktree-x-y-1234",
            "worktree":{"checkout_path":"/tmp/wt","is_linked_worktree":false}
        }"#;
        assert!(evaluate(&event("working"), ctx).is_none());
    }

    #[test]
    fn garbage_json_bails() {
        assert!(evaluate("not json", CTX_ELIGIBLE).is_none());
        assert!(evaluate(&event("working"), "not json").is_none());
    }
}
