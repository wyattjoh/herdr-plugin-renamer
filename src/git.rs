//! Minimal git interaction against the worktree checkout: read the current
//! branch (for a safety re-check), inspect local refs, and rename it.

use std::process::Command;

/// `git symbolic-ref --short HEAD` in the worktree. `None` on detached HEAD or
/// any error.
pub fn current_branch(checkout_path: &str) -> Option<String> {
    let output = Command::new("git")
        .current_dir(checkout_path)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

/// Check whether a local branch ref already exists.
pub fn branch_exists(checkout_path: &str, branch: &str) -> Option<bool> {
    let reference = format!("refs/heads/{branch}");
    let output = Command::new("git")
        .current_dir(checkout_path)
        .args(["show-ref", "--verify", "--quiet", &reference])
        .output()
        .ok()?;
    match output.status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// `git branch -m <new_branch>` renames the currently checked-out branch.
pub fn rename_current_branch(checkout_path: &str, new_branch: &str) -> bool {
    Command::new("git")
        .current_dir(checkout_path)
        .args(["branch", "-m", new_branch])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
