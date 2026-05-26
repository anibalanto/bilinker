use std::path::Path;
use anyhow::{bail, Result};

/// Returns the SHA-1 of the most recent commit that touched `file` (relative to `root`).
/// Returns an error if the file has no git history (uncommitted or not tracked).
pub fn head_commit_for_file(root: &Path, file: &str) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["log", "--format=%H", "-n", "1", "--", file])
        .current_dir(root)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let commit = stdout.trim().to_string();

    if commit.is_empty() {
        bail!(
            "file '{}' has no git history — commit the file before creating a bilink",
            file
        );
    }
    Ok(commit)
}

/// Returns the SHA-1 of the most recent commit that touched `file`, or `None` if none exists.
/// Use `head_commit_for_file` when history is required.
pub fn try_head_commit_for_file(root: &Path, file: &str) -> Option<String> {
    head_commit_for_file(root, file).ok()
}
