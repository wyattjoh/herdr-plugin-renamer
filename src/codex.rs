//! The naming engine: a single headless Codex call that turns the captured
//! prompt into a kebab-case slug. Bounded by a hard timeout; returns `None` on
//! any failure so the caller can fall back to a deterministic local slug.

use std::env;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(30);
const PROMPT_LIMIT: usize = 2000;

/// Run `codex exec` non-interactively to produce a slug. The model's final
/// message is written to `slug_file` via `-o`; we read and sanitize it.
///
/// `--ignore-user-config` is load-bearing: it disables the user's Codex hooks
/// (SessionStart/UserPromptSubmit, including herdr's own), giving a
/// deterministic, recursion-free run. Auth still resolves from CODEX_HOME.
pub fn generate_slug(prompt: &str, slug_file: &Path) -> Option<String> {
    let bin = env::var("HERDR_NAMING_CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
    let truncated: String = prompt.chars().take(PROMPT_LIMIT).collect();
    let full_prompt = format!(
        "Output only a short kebab-case git branch slug (2-4 words, lowercase, \
         hyphens only, no prose, no quotes, no surrounding text) summarizing \
         this coding task:\n\n{truncated}"
    );

    let _ = std::fs::remove_file(slug_file);

    let mut child = Command::new(bin)
        .args([
            "exec",
            "--skip-git-repo-check",
            "--ignore-user-config",
            "-s",
            "read-only",
            "-m",
            "gpt-5.5",
            "-c",
            "model_reasoning_effort=low",
            "-o",
        ])
        .arg(slug_file)
        .arg(&full_prompt)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    if !wait_with_timeout(&mut child, TIMEOUT) {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }

    let raw = std::fs::read_to_string(slug_file).ok()?;
    let slug = crate::slug::sanitize(&raw);
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

/// Poll `try_wait` until the child exits or the timeout elapses. Returns true if
/// the child finished on its own.
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return false,
        }
    }
}
