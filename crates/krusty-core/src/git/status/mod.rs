use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;

use super::command::{resolve_repo_root, run_git, should_suppress_display};
use super::model::{GitBranchSummary, GitStatusSummary, GitWorktreeSummary};
use super::pr::resolve_pr_number;

mod diff;
mod parse;

use diff::{compute_branch_diff_summary, compute_worktree_diff_summary};
#[cfg(test)]
pub(super) use parse::{parse_numstat, parse_status_output, parse_worktree_output};

/// Get repository status for a given path.
pub fn status(path: &Path) -> Result<Option<GitStatusSummary>> {
    let repo_root = match resolve_repo_root(path)? {
        Some(root) => root,
        None => return Ok(None),
    };

    if should_suppress_display(&repo_root) {
        return Ok(None);
    }

    let output = run_git(
        &[
            "status",
            "--porcelain=2",
            "--branch",
            "--untracked-files=all",
        ],
        &repo_root,
    )?;
    let mut status =
        parse::parse_status_output(repo_root, &String::from_utf8_lossy(&output.stdout));
    if let Some(diff) = compute_branch_diff_summary(&status.repo_root, status.upstream.as_deref()) {
        status.branch_files = diff.files;
        status.branch_additions = diff.additions;
        status.branch_deletions = diff.deletions;
    }
    if status.total_changes() > 0 {
        if let Some(diff) = compute_worktree_diff_summary(&status.repo_root) {
            status.worktree_additions = diff.additions;
            status.worktree_deletions = diff.deletions;
        }
    }
    status.pr_number = resolve_pr_number(&status.repo_root, status.branch.as_deref());

    Ok(Some(status))
}

/// List local and remote branches for a repository path.
pub fn branches(path: &Path) -> Result<Option<Vec<GitBranchSummary>>> {
    let repo_root = match resolve_repo_root(path)? {
        Some(root) => root,
        None => return Ok(None),
    };

    let output = run_git(
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(HEAD)%09%(upstream:short)",
            "refs/heads",
        ],
        &repo_root,
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut branches = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        branches.push(parse::parse_branch_summary_line(line));
    }

    let local_names: HashSet<String> = branches.iter().map(|b| b.name.clone()).collect();

    if let Ok(remote_output) = run_git(
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/remotes/origin",
        ],
        &repo_root,
    ) {
        let remote_stdout = String::from_utf8_lossy(&remote_output.stdout);
        for line in remote_stdout.lines().filter(|l| !l.trim().is_empty()) {
            let stripped = line.trim().strip_prefix("origin/").unwrap_or(line.trim());
            if stripped == "HEAD" || local_names.contains(stripped) {
                continue;
            }
            branches.push(GitBranchSummary {
                name: stripped.to_string(),
                is_current: false,
                upstream: Some(format!("origin/{stripped}")),
                is_remote: true,
            });
        }
    }

    branches.sort_by(|a, b| {
        b.is_current
            .cmp(&a.is_current)
            .then(a.is_remote.cmp(&b.is_remote))
            .then(a.name.cmp(&b.name))
    });
    Ok(Some(branches))
}

/// List worktrees for a repository path.
pub fn worktrees(path: &Path) -> Result<Option<Vec<GitWorktreeSummary>>> {
    let repo_root = match resolve_repo_root(path)? {
        Some(root) => root,
        None => return Ok(None),
    };

    let output = run_git(&["worktree", "list", "--porcelain"], &repo_root)?;
    let mut worktrees = parse::parse_worktree_output(&String::from_utf8_lossy(&output.stdout));

    let current_root = repo_root.canonicalize().unwrap_or(repo_root);
    for wt in &mut worktrees {
        let wt_path = wt.path.canonicalize().unwrap_or_else(|_| wt.path.clone());
        wt.is_current = wt_path == current_root;
    }

    worktrees.sort_by(|a, b| b.is_current.cmp(&a.is_current).then(a.path.cmp(&b.path)));
    Ok(Some(worktrees))
}
