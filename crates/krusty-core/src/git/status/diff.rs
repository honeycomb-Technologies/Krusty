use std::path::Path;

use super::super::command::{ref_exists, run_git};
use super::super::model::BranchDiffSummary;
use super::parse::parse_numstat;

pub(super) fn compute_branch_diff_summary(
    repo_root: &Path,
    upstream: Option<&str>,
) -> Option<BranchDiffSummary> {
    let base_ref = resolve_base_ref(repo_root, upstream)?;
    let merge_base_output = run_git(&["merge-base", "HEAD", base_ref.as_str()], repo_root).ok()?;
    let merge_base = String::from_utf8_lossy(&merge_base_output.stdout)
        .trim()
        .to_string();
    if merge_base.is_empty() {
        return None;
    }

    let range = format!("{merge_base}..HEAD");
    let diff_output = run_git(&["diff", "--numstat", range.as_str()], repo_root).ok()?;
    let stdout = String::from_utf8_lossy(&diff_output.stdout);
    Some(parse_numstat(&stdout))
}

pub(super) fn compute_worktree_diff_summary(repo_root: &Path) -> Option<BranchDiffSummary> {
    let diff_output = run_git(&["diff", "--numstat", "HEAD", "--"], repo_root).ok()?;
    let stdout = String::from_utf8_lossy(&diff_output.stdout);
    Some(parse_numstat(&stdout))
}

fn resolve_base_ref(repo_root: &Path, upstream: Option<&str>) -> Option<String> {
    if let Some(upstream) = upstream.filter(|u| !u.trim().is_empty()) {
        if ref_exists(repo_root, upstream) {
            return Some(upstream.to_string());
        }
    }

    for candidate in ["origin/main", "origin/master", "main", "master"] {
        if ref_exists(repo_root, candidate) {
            return Some(candidate.to_string());
        }
    }

    None
}
