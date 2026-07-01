//! The on-device naming engine: shells out to the `herdr-namer` Swift helper,
//! which asks Apple's FoundationModels for a kebab-case slug. Mirrors
//! `codex::generate_slug`: returns `None` on any failure (model unavailable,
//! helper missing, timeout, non-zero exit) so the caller falls back to Codex
//! and then a deterministic local slug. No network, no auth.

use std::env;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

// On-device generation is sub-second warm and ~1-2s cold; this is a generous
// ceiling that still guards against a wedged model call.
const TIMEOUT: Duration = Duration::from_secs(15);
const PROMPT_LIMIT: usize = 2000;

/// Run the Swift helper to produce a slug candidate. The helper prints a bare
/// candidate to stdout and exits 0 on success, or writes a reason to stderr and
/// exits non-zero when Apple Intelligence is unavailable. We sanitize stdout.
pub fn generate_slug(prompt: &str) -> Option<String> {
    let bin = helper_bin()?;
    let truncated: String = prompt.chars().take(PROMPT_LIMIT).collect();

    let mut child = Command::new(&bin)
        .arg(&truncated)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let status = match wait_with_timeout(&mut child, TIMEOUT) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    if !status.success() {
        return None;
    }

    // The child has exited; its tiny stdout is buffered in the pipe.
    let mut raw = String::new();
    child.stdout.take()?.read_to_string(&mut raw).ok()?;

    let slug = crate::slug::sanitize(&raw);
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

/// Resolve the helper binary path. Honors `HERDR_NAMING_FOUNDATION_BIN`, else
/// defaults to the SwiftPM release build next to this plugin's own binary
/// (`<root>/target/release/<bin>` -> `<root>/naming-helper/.build/release/herdr-namer`).
fn helper_bin() -> Option<PathBuf> {
    if let Ok(path) = env::var("HERDR_NAMING_FOUNDATION_BIN") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let exe = env::current_exe().ok()?;
    let root = exe.parent()?.parent()?.parent()?; // release -> target -> root
    Some(root.join("naming-helper/.build/release/herdr-namer"))
}

/// Poll `try_wait` until the child exits or the timeout elapses, returning the
/// exit status if it finished on its own. The child's stdout pipe stays
/// readable after exit (the slug is far smaller than the pipe buffer, so the
/// child never blocks on a full pipe while we poll).
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Live end-to-end check of the Rust wrapper against the real Swift helper
    // and on-device model. Ignored by default: it needs the helper built
    // (`swift build -c release` under naming-helper/) and Apple Intelligence
    // available. Run: cargo test foundation -- --ignored --nocapture
    #[test]
    #[ignore]
    fn helper_produces_a_sane_slug() {
        let helper = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/naming-helper/.build/release/herdr-namer"
        );
        env::set_var("HERDR_NAMING_FOUNDATION_BIN", helper);
        let slug = generate_slug("Add a dark mode toggle to the settings page")
            .expect("expected a slug from the on-device helper");
        assert!(!slug.is_empty());
        assert!(
            slug.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "slug had unexpected chars: {slug}"
        );
    }

    #[test]
    #[ignore]
    fn helper_prefers_compact_noun_topic_labels() {
        let helper = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/naming-helper/.build/release/herdr-namer"
        );
        env::set_var("HERDR_NAMING_FOUNDATION_BIN", helper);

        let slug = generate_slug("Change selected file to current")
            .expect("expected a slug from the on-device helper");

        assert_eq!(slug, "current-file");
    }

    #[test]
    #[ignore]
    fn helper_uses_the_actual_prompt_topic_instead_of_examples() {
        let helper = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/naming-helper/.build/release/herdr-namer"
        );
        env::set_var("HERDR_NAMING_FOUNDATION_BIN", helper);

        let slug = generate_slug("tell me about the commits on this branch")
            .expect("expected a slug from the on-device helper");

        assert_eq!(slug, "branch-commits");
    }
}
