//! Turning arbitrary text into a safe kebab-case slug, and a deterministic
//! fallback slug derived from the prompt when the Codex naming engine is
//! unavailable.

const MAX_WORDS: usize = 6;
const MAX_LEN: usize = 50;

/// Lowercase, collapse every run of non-alphanumeric characters into a single
/// hyphen, trim leading/trailing hyphens, then cap to `MAX_WORDS` words and
/// `MAX_LEN` characters. ASCII-only output suitable for a git branch name.
pub fn sanitize(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = true; // start true so leading separators are dropped
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }

    let capped = out
        .split('-')
        .filter(|w| !w.is_empty())
        .take(MAX_WORDS)
        .collect::<Vec<_>>()
        .join("-");

    let mut capped = if capped.len() > MAX_LEN {
        capped[..MAX_LEN].to_string()
    } else {
        capped
    };
    while capped.ends_with('-') {
        capped.pop();
    }
    capped
}

/// Build a slug from the first non-empty line of the prompt. Never returns an
/// empty string, so a rename always has something to use.
pub fn fallback_from_prompt(prompt: &str) -> String {
    let first_line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let slug = sanitize(first_line);
    if slug.is_empty() {
        "agent-task".to_string()
    } else {
        slug
    }
}

/// Return the base slug or its first free numeric suffix. `is_taken` returns
/// `None` when availability cannot be determined.
pub fn first_available(
    base: &str,
    mut is_taken: impl FnMut(&str) -> Option<bool>,
) -> Option<String> {
    if !is_taken(base)? {
        return Some(base.to_string());
    }
    for number in 2.. {
        let suffix = format!("-{number}");
        let stem: String = base.chars().take(MAX_LEN - suffix.len()).collect();
        let candidate = format!("{}{suffix}", stem.trim_end_matches('-'));
        if !is_taken(&candidate)? {
            return Some(candidate);
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_kebab() {
        assert_eq!(sanitize("OAuth Login Providers"), "oauth-login-providers");
    }

    #[test]
    fn collapses_punctuation_and_spaces() {
        assert_eq!(
            sanitize("Fix the bug!!! in   parser"),
            "fix-the-bug-in-parser"
        );
    }

    #[test]
    fn trims_edges() {
        assert_eq!(sanitize("  --Hello, World--  "), "hello-world");
    }

    #[test]
    fn caps_to_six_words() {
        assert_eq!(
            sanitize("one two three four five six seven eight"),
            "one-two-three-four-five-six"
        );
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(sanitize("   !!!   "), "");
    }

    #[test]
    fn fallback_never_empty() {
        assert_eq!(fallback_from_prompt("!!!"), "agent-task");
        assert_eq!(
            fallback_from_prompt("Add JWT auth to the API endpoints please"),
            "add-jwt-auth-to-the-api"
        );
    }

    #[test]
    fn fallback_uses_first_nonempty_line() {
        assert_eq!(
            fallback_from_prompt("\n\n  \nRefactor token validation"),
            "refactor-token-validation"
        );
    }

    #[test]
    fn adds_first_free_suffix_without_exceeding_slug_limit() {
        let taken = ["rename-task", "rename-task-2"];
        assert_eq!(
            first_available("rename-task", |candidate| Some(taken.contains(&candidate))),
            Some("rename-task-3".into())
        );
        let maxed = "a".repeat(MAX_LEN);
        assert_eq!(
            first_available(&maxed, |candidate| Some(candidate == maxed)).map(|slug| slug.len()),
            Some(MAX_LEN)
        );
    }
}
