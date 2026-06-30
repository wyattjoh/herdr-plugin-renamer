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

/// A pane that passed the cheap eligibility gate: an agent just transitioned to
/// `working`. Worktree details are optional because tab renaming also applies
/// to main checkouts.
#[derive(Debug, PartialEq, Eq)]
pub struct Eligible {
    pub pane_id: String,
    pub workspace_id: String,
    pub workspace_label: Option<String>,
    pub checkout_path: Option<String>,
    pub is_linked_worktree: bool,
}

/// The fast bail. Returns `Some(Eligible)` only when:
///  - the new agent status is `working`.
///
/// The cold path decides which side effects are allowed: tab rename for numeric
/// tabs, and branch/workspace rename only for linked worktrees still on an auto
/// `worktree/` branch.
pub fn evaluate(event_json: &str, context_json: &str) -> Option<Eligible> {
    let event: EventEnvelope = serde_json::from_str(event_json).ok()?;
    if event.data.agent_status != "working" {
        return None;
    }

    let context: Context = serde_json::from_str(context_json).ok()?;
    let (checkout_path, is_linked_worktree) = context
        .worktree
        .map(|worktree| (worktree.checkout_path, worktree.is_linked_worktree))
        .unwrap_or((None, false));

    Some(Eligible {
        pane_id: event.data.pane_id,
        workspace_id: event.data.workspace_id,
        workspace_label: context.workspace_label,
        checkout_path,
        is_linked_worktree,
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
        assert_eq!(
            e.workspace_label.as_deref(),
            Some("worktree-silver-field-3fd7")
        );
        assert_eq!(e.checkout_path.as_deref(), Some("/tmp/wt"));
        assert!(e.is_linked_worktree);
    }

    #[test]
    fn non_working_status_bails() {
        assert!(evaluate(&event("idle"), CTX_ELIGIBLE).is_none());
        assert!(evaluate(&event("blocked"), CTX_ELIGIBLE).is_none());
    }

    #[test]
    fn working_without_worktree_is_eligible_for_tab_rename() {
        let ctx = r#"{"workspace_id":"w4B","workspace_label":"main-checkout"}"#;
        let e = evaluate(&event("working"), ctx).expect("eligible");
        assert_eq!(e.pane_id, "w4B:p1");
        assert_eq!(e.workspace_id, "w4B");
    }

    #[test]
    fn already_renamed_workspace_is_still_eligible_for_tab_rename() {
        let ctx = r#"{
            "workspace_id":"w4B",
            "workspace_label":"oauth-login",
            "worktree":{"checkout_path":"/tmp/wt","is_linked_worktree":true}
        }"#;
        let e = evaluate(&event("working"), ctx).expect("eligible");
        assert_eq!(e.workspace_label.as_deref(), Some("oauth-login"));
    }

    #[test]
    fn non_linked_worktree_is_eligible_but_not_linked() {
        let ctx = r#"{
            "workspace_id":"w4B",
            "workspace_label":"worktree-x-y-1234",
            "worktree":{"checkout_path":"/tmp/wt","is_linked_worktree":false}
        }"#;
        let e = evaluate(&event("working"), ctx).expect("eligible");
        assert!(!e.is_linked_worktree);
        assert_eq!(e.checkout_path.as_deref(), Some("/tmp/wt"));
    }

    #[test]
    fn garbage_json_bails() {
        assert!(evaluate("not json", CTX_ELIGIBLE).is_none());
        assert!(evaluate(&event("working"), "not json").is_none());
    }
}
