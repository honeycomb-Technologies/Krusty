use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::debug;

use crate::updater::checker::types::UpdateInfo;

pub(super) fn check_for_updates_dev(repo_path: &Path) -> Result<Option<UpdateInfo>> {
    debug!("Fetching from origin...");

    let fetch_status = Command::new("git")
        .args(["fetch", "origin", "main", "--quiet"])
        .current_dir(repo_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if !fetch_status.success() {
        return Err(anyhow!("Failed to fetch from origin"));
    }

    let current_commit = git_stdout(repo_path, &["rev-parse", "--short", "HEAD"])?;
    let new_commit = git_stdout(repo_path, &["rev-parse", "--short", "origin/main"])?;

    debug!("Current: {}, Remote: {}", current_commit, new_commit);

    if current_commit == new_commit {
        return Ok(None);
    }

    let commit_message = git_stdout(repo_path, &["log", "-1", "--format=%s", "origin/main"])?;

    Ok(Some(UpdateInfo {
        current_version: current_commit,
        new_version: new_commit,
        release_notes: commit_message,
        is_dev_mode: true,
    }))
}

pub(crate) fn is_newer_version(new: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut parts = s.split('.');
        (
            parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        )
    };

    let new_v = parse(new);
    let curr_v = parse(current);

    new_v > curr_v
}

fn git_stdout(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            "unknown error".to_string()
        } else {
            stderr
        };
        return Err(anyhow!("git {} failed: {}", args.join(" "), detail));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
