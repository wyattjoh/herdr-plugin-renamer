//! Bounded, fail-open enrichment of naming input with Claude skill context and
//! safe excerpts from `@relative/file` references.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_SOURCE_FILE_BYTES: u64 = 256 * 1024;
const MAX_FILE_REFERENCES: usize = 16;
const MAX_REFERENCE_TOKENS: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct EnrichmentPolicy {
    pub include_expanded_context: bool,
    pub max_skill_chars: usize,
    pub max_file_chars: usize,
    pub max_total_context_chars: usize,
}

impl Default for EnrichmentPolicy {
    fn default() -> Self {
        Self {
            include_expanded_context: true,
            max_skill_chars: 1_000,
            max_file_chars: 1_200,
            max_total_context_chars: 2_400,
        }
    }
}

/// Retain the literal prompt and append bounded context when policy permits it.
pub fn expand_naming_input(
    raw_prompt: &str,
    skill_expansion: Option<&str>,
    checkout_path: Option<&Path>,
    policy: &EnrichmentPolicy,
) -> String {
    if !policy.include_expanded_context {
        return raw_prompt.to_string();
    }

    let mut result = raw_prompt.to_string();
    let mut remaining = policy.max_total_context_chars;

    if let Some(skill) = skill_expansion
        .map(str::trim)
        .filter(|skill| !skill.is_empty())
    {
        let opening = "\n\n<skill-context>\n";
        let closing = "\n</skill-context>";
        let overhead = opening.chars().count() + closing.chars().count();
        let limit = policy
            .max_skill_chars
            .min(remaining.saturating_sub(overhead));
        let excerpt = bounded_excerpt(skill, limit, "skill context");
        if !excerpt.is_empty() {
            result.push_str(opening);
            result.push_str(&excerpt);
            result.push_str(closing);
            remaining = remaining.saturating_sub(overhead + excerpt.chars().count());
        }
    }

    if let Some(root) = checkout_path.and_then(|path| path.canonicalize().ok()) {
        let mut seen = HashSet::new();
        for reference in file_references(raw_prompt).take(MAX_REFERENCE_TOKENS) {
            if remaining == 0 {
                break;
            }
            if !seen.insert(reference.clone()) {
                continue;
            }
            if seen.len() > MAX_FILE_REFERENCES {
                break;
            }
            let escaped_reference = escape_attribute(&reference);
            let opening = format!("\n\n<file-context path=\"{escaped_reference}\">\n");
            let closing = "\n</file-context>";
            let overhead = opening.chars().count() + closing.chars().count();
            let limit = policy
                .max_file_chars
                .min(remaining.saturating_sub(overhead));
            let Some(contents) = read_safe_file_excerpt(&root, &reference, limit) else {
                continue;
            };
            if contents.is_empty() {
                continue;
            }
            result.push_str(&opening);
            result.push_str(&contents);
            result.push_str(closing);
            remaining = remaining.saturating_sub(overhead + contents.chars().count());
        }
    }

    result
}

fn file_references(prompt: &str) -> impl Iterator<Item = String> + '_ {
    prompt.split_whitespace().filter_map(|token| {
        let path = token
            .strip_prefix('@')?
            .trim_start_matches(['\'', '"', '(', '[', '{'])
            .trim_end_matches(['\'', '"', ',', '.', ';', ':', '!', '?', ')', ']', '}']);
        if path.is_empty() {
            None
        } else {
            Some(path.to_string())
        }
    })
}

fn read_safe_file_excerpt(root: &Path, reference: &str, limit: usize) -> Option<String> {
    let relative = PathBuf::from(reference);
    if relative.is_absolute() {
        return None;
    }

    let resolved = root.join(relative).canonicalize().ok()?;
    let metadata = resolved.metadata().ok()?;
    if !resolved.starts_with(root)
        || !metadata.file_type().is_file()
        || metadata.len() > MAX_SOURCE_FILE_BYTES
    {
        return None;
    }

    let text = std::fs::read_to_string(resolved).ok()?;
    if looks_binary(&text) {
        return None;
    }
    Some(bounded_excerpt(&text, limit, "file content"))
}

fn looks_binary(text: &str) -> bool {
    text.chars()
        .any(|ch| ch == '\0' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
}

fn escape_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
}

fn bounded_excerpt(input: &str, limit: usize, label: &str) -> String {
    if limit == 0 {
        return String::new();
    }
    let count = input.chars().count();
    if count <= limit {
        return input.to_string();
    }

    let marker = format!("\n[... {label} truncated for naming ...]\n");
    let marker_len = marker.chars().count();
    if limit <= marker_len {
        return input.chars().take(limit).collect();
    }
    let available = limit - marker_len;
    let head_len = available.div_ceil(2);
    let tail_len = available / 2;
    let head: String = input.chars().take(head_len).collect();
    let tail: String = input.chars().skip(count - tail_len).collect();
    format!("{head}{marker}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempCheckout(PathBuf);

    impl TempCheckout {
        fn new() -> Self {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("herdr-renamer-prompt-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).expect("create temp checkout");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempCheckout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn includes_bounded_skill_expansion() {
        let input = expand_naming_input(
            "wayfinder @docs/plan.md",
            Some("# Wayfinder\nDiscover the implementation route."),
            None,
            &EnrichmentPolicy::default(),
        );

        assert_eq!(
            input,
            "wayfinder @docs/plan.md\n\n<skill-context>\n# Wayfinder\nDiscover the implementation route.\n</skill-context>"
        );
    }

    #[test]
    fn includes_readable_relative_file() {
        let checkout = TempCheckout::new();
        fs::create_dir(checkout.path().join("docs")).expect("create docs");
        fs::write(
            checkout.path().join("docs/plan.md"),
            "# Rename plan\nUse workspace metadata.",
        )
        .expect("write reference");

        let input = expand_naming_input(
            "wayfinder @docs/plan.md",
            None,
            Some(checkout.path()),
            &EnrichmentPolicy::default(),
        );

        assert_eq!(
            input,
            "wayfinder @docs/plan.md\n\n<file-context path=\"docs/plan.md\">\n# Rename plan\nUse workspace metadata.\n</file-context>"
        );
    }

    #[test]
    fn no_reference_input_is_unchanged() {
        let checkout = TempCheckout::new();
        assert_eq!(
            expand_naming_input(
                "rename the workspace",
                None,
                Some(checkout.path()),
                &EnrichmentPolicy::default(),
            ),
            "rename the workspace"
        );
    }

    #[test]
    fn missing_reference_preserves_literal_input() {
        let checkout = TempCheckout::new();
        assert_eq!(
            expand_naming_input(
                "wayfinder @missing.md",
                None,
                Some(checkout.path()),
                &EnrichmentPolicy::default(),
            ),
            "wayfinder @missing.md"
        );
    }

    #[test]
    fn duplicate_reference_does_not_hide_later_files() {
        let checkout = TempCheckout::new();
        fs::write(checkout.path().join("one.md"), "first file").expect("write one");
        fs::write(checkout.path().join("two.md"), "second file").expect("write two");

        let input = expand_naming_input(
            "compare @one.md @one.md @two.md",
            None,
            Some(checkout.path()),
            &EnrichmentPolicy::default(),
        );

        assert_eq!(input.matches("path=\"one.md\"").count(), 1);
        assert!(input.contains("path=\"two.md\""));
    }

    #[test]
    fn multiple_references_respect_total_context_limit() {
        let checkout = TempCheckout::new();
        fs::write(checkout.path().join("one.md"), "aaaaaaaaaaaaaaaaaaaa").expect("write one");
        fs::write(checkout.path().join("two.md"), "bbbbbbbbbbbbbbbbbbbb").expect("write two");
        let policy = EnrichmentPolicy {
            max_file_chars: 20,
            max_total_context_chars: 120,
            ..EnrichmentPolicy::default()
        };
        let raw = "compare @one.md @two.md";

        let input = expand_naming_input(raw, None, Some(checkout.path()), &policy);

        assert!(input.contains("aaaaaaaaaaaaaaaaaaaa"));
        assert!(input.contains("<file-context path=\"two.md\">\nbbbbbb"));
        assert!(!input.contains("bbbbbbb"));
        assert!(input.chars().count() - raw.chars().count() <= 120);
    }

    #[test]
    fn traversal_and_symlink_escapes_are_rejected() {
        use std::os::unix::fs::symlink;

        let parent = TempCheckout::new();
        let checkout_path = parent.path().join("checkout");
        fs::create_dir(&checkout_path).expect("create checkout");
        fs::write(parent.path().join("outside.md"), "private outside text").expect("write outside");
        symlink(
            parent.path().join("outside.md"),
            checkout_path.join("escape.md"),
        )
        .expect("create symlink");

        let input = expand_naming_input(
            "inspect @../outside.md and @escape.md",
            None,
            Some(&checkout_path),
            &EnrichmentPolicy::default(),
        );

        assert_eq!(input, "inspect @../outside.md and @escape.md");
    }

    #[test]
    fn binary_files_are_skipped() {
        let checkout = TempCheckout::new();
        fs::write(checkout.path().join("invalid.dat"), [0xff, 0xfe, 0xfd])
            .expect("write invalid utf8");
        fs::write(checkout.path().join("control.dat"), [0, 1, 2]).expect("write control bytes");

        let input = expand_naming_input(
            "inspect @invalid.dat and @control.dat",
            None,
            Some(checkout.path()),
            &EnrichmentPolicy::default(),
        );

        assert_eq!(input, "inspect @invalid.dat and @control.dat");
    }

    #[test]
    fn files_over_the_source_safety_cap_are_skipped() {
        let checkout = TempCheckout::new();
        fs::write(
            checkout.path().join("huge.md"),
            vec![b'a'; MAX_SOURCE_FILE_BYTES as usize + 1],
        )
        .expect("write huge file");

        let input = expand_naming_input(
            "inspect @huge.md",
            None,
            Some(checkout.path()),
            &EnrichmentPolicy::default(),
        );

        assert_eq!(input, "inspect @huge.md");
    }

    #[test]
    fn oversized_file_uses_explicit_head_tail_excerpt() {
        let checkout = TempCheckout::new();
        fs::write(
            checkout.path().join("large.md"),
            format!("{}{}{}", "a".repeat(40), "x".repeat(40), "z".repeat(40)),
        )
        .expect("write large file");
        let policy = EnrichmentPolicy {
            max_file_chars: 80,
            ..EnrichmentPolicy::default()
        };

        let input = expand_naming_input("inspect @large.md", None, Some(checkout.path()), &policy);

        assert!(input.contains("[... file content truncated for naming ...]"));
        assert!(input.contains("aaaa"));
        assert!(input.contains("zzzz"));
        assert!(!input.contains(&"x".repeat(40)));
    }

    #[test]
    fn disabled_expansion_returns_literal_input() {
        let checkout = TempCheckout::new();
        fs::write(checkout.path().join("plan.md"), "private context").expect("write plan");
        let policy = EnrichmentPolicy {
            include_expanded_context: false,
            ..EnrichmentPolicy::default()
        };

        let input = expand_naming_input(
            "wayfinder @plan.md",
            Some("expanded skill instructions"),
            Some(checkout.path()),
            &policy,
        );

        assert_eq!(input, "wayfinder @plan.md");
    }
}
