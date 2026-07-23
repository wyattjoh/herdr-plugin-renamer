//! Resolving an agent transcript from a native session id or Pi-reported path and extracting
//! genuine user prompts. Supports Claude Code, Codex, and Pi, which use different on-disk formats.

use std::env;
use std::path::PathBuf;

/// Resolve the transcript file, read it, and return the first real user prompt.
pub fn read_first_prompt(agent: &str, session_id: &str) -> Option<String> {
    let contents = read_transcript(agent, session_id)?;
    first_prompt(agent, &contents)
}

/// Build the explicit `/rename` model context and local fallback from real Pi prompts.
pub fn read_rename_prompt(agent: &str, session_id: &str) -> Option<(String, String)> {
    let contents = read_transcript(agent, session_id)?;
    match agent {
        "pi" => rename_prompt_pi(&contents),
        _ => None,
    }
}

fn read_transcript(agent: &str, session_id: &str) -> Option<String> {
    let path = resolve_path(agent, session_id)?;
    std::fs::read_to_string(path).ok()
}

/// Resolve the agent's `.jsonl` file; Pi reports its path directly.
fn resolve_path(agent: &str, session_id: &str) -> Option<PathBuf> {
    let home = env::var("HOME").ok();
    resolve_path_with_home(agent, session_id, home.as_deref())
}

fn resolve_path_with_home(agent: &str, session_id: &str, home: Option<&str>) -> Option<PathBuf> {
    if agent == "pi" {
        return Some(PathBuf::from(session_id));
    }
    let home = home?;
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
        "pi" => first_prompt_pi(contents),
        _ => None,
    }
}

/// Claude Code JSONL: the first `type=="user"` line that is not meta, carries
/// genuine text (string or `text` blocks), and is not a slash/local-command
/// wrapper. If no such line exists, falls back to the first non-ignored
/// slash-command invocation via `claude_command_prompt`.
fn first_prompt_claude(contents: &str) -> Option<String> {
    let mut command_fallback = None;

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
        if text.is_empty() {
            continue;
        }
        if is_claude_command(text) {
            if command_fallback.is_none() {
                command_fallback = claude_command_prompt(text);
            }
            continue;
        }
        return Some(text.to_string());
    }

    command_fallback
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

fn claude_command_prompt(text: &str) -> Option<String> {
    if text.starts_with("<local-command") {
        return None;
    }

    // `command-message` is a display label, not the raw slash invocation, but
    // for the builtins in `is_ignored_command` it equals the canonical name
    // (e.g. "clear", "model"). Falls back to `command-name` when absent.
    let command = extract_tag(text, "command-message")
        .or_else(|| extract_tag(text, "command-name"))?
        .trim()
        .trim_start_matches('/')
        .to_string();
    if command.is_empty() {
        return None;
    }
    if is_ignored_command(&command) {
        return None;
    }

    let args = extract_tag(text, "command-args").unwrap_or_default();
    let args = args.trim();

    if args.is_empty() {
        Some(command)
    } else {
        Some(format!("{command} {args}"))
    }
}

fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].to_string())
}

/// Denylist of Claude Code builtin session-control commands that carry no
/// task intent (settings, housekeeping, meta), so they should never win the
/// command-fallback slot over a later task-bearing command.
fn is_ignored_command(command: &str) -> bool {
    matches!(
        command,
        "add-dir"
            | "bug"
            | "clear"
            | "color"
            | "compact"
            | "config"
            | "cost"
            | "doctor"
            | "export"
            | "help"
            | "login"
            | "logout"
            | "memory"
            | "model"
            | "permissions"
            | "plugin"
            | "plugins"
            | "reload-plugins"
            | "reload-skills"
            | "resume"
            | "skills"
            | "status"
    )
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

/// Pi JSONL: the first user `message` entry with text content. Session and
/// model-change records have no `message.role` and are skipped.
fn first_prompt_pi(contents: &str) -> Option<String> {
    contents.lines().find_map(pi_prompt)
}

fn rename_prompt_pi(contents: &str) -> Option<(String, String)> {
    let messages: Vec<_> = contents.lines().filter_map(pi_prompt).collect();
    let first = messages.first()?;
    let recent = messages
        .iter()
        .enumerate()
        .skip(messages.len().saturating_sub(3))
        .filter(|(index, _)| *index != 0)
        .enumerate()
        .map(|(index, (_, message))| format!("{}. {message}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let recent = if recent.is_empty() { "none" } else { &recent };
    let prompt = format!(
        "## Naming context\n\nFirst user message:\n{first}\n\nRecent user messages:\n{recent}"
    );
    Some((prompt, messages.last()?.clone()))
}

fn pi_prompt(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(|t| t.as_str()) != Some("message")
        || value.pointer("/message/role").and_then(|r| r.as_str()) != Some("user")
    {
        return None;
    }
    let text = value
        .pointer("/message/content")
        .map(extract_claude_text)
        .unwrap_or_default();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
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
    fn claude_command_wrapper_falls_back_when_no_prompt() {
        let jsonl = concat!(
            r#"{"type":"user","isMeta":false,"message":{"content":"<command-message>improve-codebase-architecture</command-message>\n<command-name>/improve-codebase-architecture</command-name>"}}"#,
            "\n",
            r#"{"type":"user","isMeta":true,"message":{"content":[{"type":"text","text":"Base directory for this skill: /tmp/skills/improve-codebase-architecture\n\n# Improve Codebase Architecture"}]}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("improve-codebase-architecture")
        );
    }

    #[test]
    fn claude_command_wrapper_includes_args() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"<command-message>improve-codebase-architecture</command-message>\n<command-name>/improve-codebase-architecture</command-name>\n<command-args>focus on persistence and snapshots</command-args>"}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("improve-codebase-architecture focus on persistence and snapshots")
        );
    }

    #[test]
    fn claude_command_fallback_skips_clear_before_task_command() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>\n<command-message>clear</command-message>\n<command-args></command-args>"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"<command-message>handoff</command-message>\n<command-name>/handoff</command-name>\n<command-args>the next agent should finish the visual pass</command-args>"}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("handoff the next agent should finish the visual pass")
        );
    }

    #[test]
    fn claude_command_fallback_skips_session_control_commands() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args>claude-opus-4-7</command-args>"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"<command-message>improve-codebase-architecture</command-message>\n<command-name>/improve-codebase-architecture</command-name>\n<command-args>focus on persistence and snapshots</command-args>"}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("improve-codebase-architecture focus on persistence and snapshots")
        );
    }

    #[test]
    fn claude_command_fallback_skips_logout() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"<command-name>/logout</command-name>\n<command-message>logout</command-message>\n<command-args></command-args>"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"<command-message>handoff</command-message>\n<command-name>/handoff</command-name>\n<command-args>the next agent should finish the visual pass</command-args>"}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("handoff the next agent should finish the visual pass")
        );
    }

    #[test]
    fn claude_local_command_wrapper_excluded_from_fallback() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"<local-command-stdout>ok</local-command-stdout>"}}"#,
            "\n",
        );
        assert!(first_prompt("claude", jsonl).is_none());
    }

    #[test]
    fn claude_command_fallback_uses_command_name_without_message() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"<command-name>/handoff</command-name>\n<command-args>finish the visual pass</command-args>"}}"#,
            "\n",
        );
        assert_eq!(
            first_prompt("claude", jsonl).as_deref(),
            Some("handoff finish the visual pass")
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
    fn pi_path_does_not_require_home() {
        assert_eq!(
            resolve_path_with_home("pi", "/tmp/pi-session.jsonl", None),
            Some(PathBuf::from("/tmp/pi-session.jsonl"))
        );
    }

    #[test]
    fn pi_reads_first_prompt_and_builds_rename_context() {
        let jsonl = concat!(
            r#"{"type":"session","id":"session"}"#,
            "\n",
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}]}}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"First task"}]}}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Second task"}]}}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Third task"}]}}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Fourth task"}]}}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Latest task"}]}}"#,
            "\n",
        );
        assert_eq!(first_prompt("pi", jsonl).as_deref(), Some("First task"));
        assert_eq!(
            rename_prompt_pi(jsonl),
            Some((
                "## Naming context\n\nFirst user message:\nFirst task\n\nRecent user messages:\n1. Third task\n2. Fourth task\n3. Latest task".into(),
                "Latest task".into(),
            ))
        );
        assert_eq!(
            rename_prompt_pi(
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Only task"}]}}"#
            ),
            Some((
                "## Naming context\n\nFirst user message:\nOnly task\n\nRecent user messages:\nnone".into(),
                "Only task".into(),
            ))
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
