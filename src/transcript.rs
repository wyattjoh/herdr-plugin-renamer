//! Resolving an agent transcript from a native session id and extracting the
//! first genuine user prompt. Supports Claude Code and Codex, which use
//! different on-disk formats.

use std::env;
use std::path::PathBuf;

/// Resolve the transcript file, read it, and return the first real user prompt.
pub fn read_first_prompt(agent: &str, session_id: &str) -> Option<String> {
    let path = resolve_path(agent, session_id)?;
    let contents = std::fs::read_to_string(&path).ok()?;
    first_prompt(agent, &contents)
}

/// Glob the agent's transcript directory for the session's `.jsonl` file.
fn resolve_path(agent: &str, session_id: &str) -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let pattern = match agent {
        "claude" => {
            let base = env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| format!("{home}/.claude"));
            format!("{base}/projects/**/{session_id}.jsonl")
        }
        "codex" => {
            let base = env::var("CODEX_HOME").unwrap_or_else(|_| format!("{home}/.codex"));
            format!("{base}/sessions/**/rollout-*{session_id}.jsonl")
        }
        _ => return None,
    };
    glob::glob(&pattern).ok()?.flatten().next()
}

/// Dispatch to the per-agent transcript parser.
pub fn first_prompt(agent: &str, contents: &str) -> Option<String> {
    match agent {
        "claude" => first_prompt_claude(contents),
        "codex" => first_prompt_codex(contents),
        _ => None,
    }
}

/// Claude Code JSONL: the first `type=="user"` line that is not meta, carries
/// genuine text (string or `text` blocks), and is not a slash/local-command
/// wrapper.
fn first_prompt_claude(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        if value.get("isMeta").and_then(|m| m.as_bool()) == Some(true) {
            continue;
        }
        let content = match value.get("message").and_then(|m| m.get("content")) {
            Some(c) => c,
            None => continue,
        };
        let text = extract_claude_text(content);
        let text = text.trim();
        if text.is_empty() || is_claude_command(text) {
            continue;
        }
        return Some(text.to_string());
    }
    None
}

/// `message.content` is usually a string, sometimes an array of blocks. Only
/// `text` blocks count (tool_result blocks are skipped).
fn extract_claude_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

fn is_claude_command(text: &str) -> bool {
    text.starts_with("<command-name")
        || text.starts_with("<command-message")
        || text.starts_with("<local-command")
}

/// Codex rollout JSONL: the first `response_item` user `message` whose
/// `input_text` is a real prompt, skipping the developer preamble, the AGENTS.md
/// instruction block, and the `<user_instructions>`/`<environment_context>`
/// wrappers.
fn first_prompt_codex(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let payload = match value.get("payload") {
            Some(p) => p,
            None => continue,
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        if payload.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let text = payload
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("input_text"))
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let text = text.trim();
        if text.is_empty() || is_codex_preamble(text) {
            continue;
        }
        return Some(text.to_string());
    }
    None
}

fn is_codex_preamble(text: &str) -> bool {
    text.starts_with("# AGENTS.md")
        || text.starts_with("<INSTRUCTIONS>")
        || text.starts_with("<user_instructions>")
        || text.starts_with("<environment_context>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_string_content() {
        let jsonl = concat!(
            r#"{"type":"summary","summary":"x"}"#,
            "\n",
            r#"{"type":"user","isMeta":true,"message":{"content":"<command-name>/clear</command-name>"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"Add OAuth login to the dashboard"}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("Add OAuth login to the dashboard")
        );
    }

    #[test]
    fn claude_skips_command_wrapper_with_meta_false() {
        let jsonl = concat!(
            r#"{"type":"user","isMeta":false,"message":{"content":"<command-name>/clear</command-name>"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"Refactor the parser"}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("Refactor the parser")
        );
    }

    #[test]
    fn claude_array_content_skips_tool_result() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"text","text":"Fix the failing test"}]}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("Fix the failing test")
        );
    }

    #[test]
    fn codex_skips_preamble_and_instructions() {
        let jsonl = concat!(
            r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>"}]}}"#,
            "\n",
            r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions\n<INSTRUCTIONS>"}]}}"##,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd=/x</environment_context>"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Implement rate limiting on the API"}]}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("codex", jsonl).as_deref(),
            Some("Implement rate limiting on the API")
        );
    }

    #[test]
    fn unknown_agent_returns_none() {
        assert!(first_prompt("gemini", "{}").is_none());
    }

    #[test]
    fn no_real_prompt_returns_none() {
        let jsonl = r#"{"type":"user","isMeta":true,"message":{"content":"meta"}}"#;
        assert!(first_prompt("claude", jsonl).is_none());
    }
}
