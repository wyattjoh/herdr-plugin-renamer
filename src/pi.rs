//! Naming through Pi's configured model and existing authentication.

use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(45);
const PROMPT_LIMIT: usize = 2000;

fn pi_bin() -> PathBuf {
    if let Some(bin) = env::var_os("HERDR_NAMING_PI_BIN") {
        return bin.into();
    }
    if let Some(home) = env::var_os("HOME") {
        let local = PathBuf::from(home).join(".local/bin/pi");
        if local.is_file() {
            return local;
        }
    }
    "pi".into()
}

pub fn generate_slug(prompt: &str, slug_file: &Path) -> Option<String> {
    let bin = pi_bin();
    let truncated: String = prompt.chars().take(PROMPT_LIMIT).collect();
    let full_prompt = format!(
        "Output only a short kebab-case git branch slug (2-4 words, lowercase, \
         hyphens only, no prose, no quotes, no surrounding text) summarizing \
         this coding task:\n\n{truncated}"
    );
    let _ = std::fs::remove_file(slug_file);
    let output = File::create(slug_file).ok()?;
    let mut child = Command::new(bin)
        .args([
            "--print",
            "--no-session",
            "--no-tools",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-context-files",
        ])
        .arg(full_prompt)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let Some(status) = crate::process::wait_with_timeout(&mut child, TIMEOUT) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    if !status.success() {
        return None;
    }

    let raw = std::fs::read_to_string(slug_file).ok()?;
    let slug = crate::slug::sanitize(&raw);
    (!slug.is_empty()).then_some(slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn uses_pi_without_extensions_and_sanitizes_its_name() {
        let dir = std::env::temp_dir().join(format!("herdr-pi-namer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("pi");
        let args = dir.join("args");
        let slug_file = dir.join("slug");
        std::fs::write(
            &bin,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >\"$PI_TEST_ARGS\"\nprintf 'Fix Quote Builder\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("HERDR_NAMING_PI_BIN", &bin);
        std::env::set_var("PI_TEST_ARGS", &args);

        let result = generate_slug("Explain the quote builder", &slug_file);

        std::env::remove_var("HERDR_NAMING_PI_BIN");
        std::env::remove_var("PI_TEST_ARGS");
        assert_eq!(result.as_deref(), Some("fix-quote-builder"));
        let args = std::fs::read_to_string(args).unwrap();
        assert!(args.contains("--no-extensions"));
        assert!(args.contains("Explain the quote builder"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
