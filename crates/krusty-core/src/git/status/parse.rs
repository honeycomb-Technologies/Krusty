use std::path::PathBuf;

use super::super::command::short_sha;
use super::super::model::{
    BranchDiffSummary, GitBranchSummary, GitStatusSummary, GitWorktreeSummary,
};

pub(crate) fn parse_branch_summary_line(line: &str) -> GitBranchSummary {
    let mut parts = line.split('\t');
    let name = parts.next().unwrap_or_default().to_string();
    let is_current = parts.next().unwrap_or_default().trim() == "*";
    let upstream = parts.next().and_then(|u| {
        let trimmed = u.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    GitBranchSummary {
        name,
        is_current,
        upstream,
        is_remote: false,
    }
}

pub(crate) fn parse_status_output(repo_root: PathBuf, output: &str) -> GitStatusSummary {
    let mut status = GitStatusSummary {
        repo_root,
        branch: None,
        head: None,
        upstream: None,
        branch_files: 0,
        branch_additions: 0,
        branch_deletions: 0,
        pr_number: None,
        ahead: 0,
        behind: 0,
        staged: 0,
        modified: 0,
        untracked: 0,
        conflicted: 0,
    };

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(head) = rest.strip_prefix("branch.head ") {
                let head = head.trim();
                if head != "(detached)" && head != "(unknown)" {
                    status.branch = Some(head.to_string());
                }
                continue;
            }

            if let Some(oid) = rest.strip_prefix("branch.oid ") {
                let oid = oid.trim();
                if oid != "(initial)" && !oid.is_empty() {
                    status.head = Some(short_sha(oid));
                }
                continue;
            }

            if let Some(upstream) = rest.strip_prefix("branch.upstream ") {
                let upstream = upstream.trim();
                if !upstream.is_empty() {
                    status.upstream = Some(upstream.to_string());
                }
                continue;
            }

            if let Some(ab) = rest.strip_prefix("branch.ab ") {
                for part in ab.split_whitespace() {
                    if let Some(ahead) = part.strip_prefix('+') {
                        status.ahead = ahead.parse::<usize>().unwrap_or(0);
                    } else if let Some(behind) = part.strip_prefix('-') {
                        status.behind = behind.parse::<usize>().unwrap_or(0);
                    }
                }
            }
            continue;
        }

        if line.starts_with("1 ") || line.starts_with("2 ") {
            let xy = line.split_whitespace().nth(1).unwrap_or("..");
            let mut chars = xy.chars();
            let x = chars.next().unwrap_or('.');
            let y = chars.next().unwrap_or('.');

            if x != '.' {
                status.staged += 1;
            }
            if y != '.' {
                status.modified += 1;
            }
            continue;
        }

        if line.starts_with("u ") {
            status.conflicted += 1;
            continue;
        }

        if line.starts_with("? ") {
            status.untracked += 1;
        }
    }

    status
}

pub(crate) fn parse_worktree_output(output: &str) -> Vec<GitWorktreeSummary> {
    let mut result = Vec::new();
    let mut current: Option<GitWorktreeSummary> = None;

    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(prev) = current.take() {
                result.push(prev);
            }
            current = Some(GitWorktreeSummary {
                path: PathBuf::from(path.trim()),
                branch: None,
                head: None,
                is_current: false,
            });
            continue;
        }

        if line.is_empty() {
            if let Some(prev) = current.take() {
                result.push(prev);
            }
            continue;
        }

        let Some(ref mut wt) = current else {
            continue;
        };

        if let Some(head) = line.strip_prefix("HEAD ") {
            wt.head = Some(short_sha(head.trim()));
            continue;
        }

        if let Some(branch_ref) = line.strip_prefix("branch ") {
            let short = branch_ref
                .trim()
                .strip_prefix("refs/heads/")
                .unwrap_or(branch_ref.trim());
            wt.branch = Some(short.to_string());
            continue;
        }

        if line == "detached" {
            wt.branch = None;
        }
    }

    if let Some(prev) = current {
        result.push(prev);
    }

    result
}

pub(crate) fn parse_numstat(output: &str) -> BranchDiffSummary {
    let mut summary = BranchDiffSummary::default();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split('\t');
        let added = parts.next().unwrap_or_default().trim();
        let deleted = parts.next().unwrap_or_default().trim();
        let path = parts.next().unwrap_or_default().trim();
        if path.is_empty() {
            continue;
        }

        summary.files += 1;
        if let Ok(v) = added.parse::<usize>() {
            summary.additions += v;
        }
        if let Ok(v) = deleted.parse::<usize>() {
            summary.deletions += v;
        }
    }
    summary
}
