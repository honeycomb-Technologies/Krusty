use std::path::PathBuf;

use super::pr::extract_pr_from_branch_name;
use super::status::{parse_numstat, parse_status_output, parse_worktree_output};

#[test]
fn parses_porcelain_v2_status_counts() {
    let output = "\
# branch.oid 0123456789abcdef\n\
# branch.head feature-x\n\
# branch.upstream origin/feature-x\n\
# branch.ab +2 -1\n\
1 M. N... 100644 100644 100644 abcdef0 abcdef0 src/main.rs\n\
1 .M N... 100644 100644 100644 abcdef0 abcdef0 README.md\n\
2 R. N... 100644 100644 100644 abcdef0 abcdef0 R100 old new\n\
u UU N... 100644 100644 100644 100644 abcdef0 abcdef0 abcdef0 conflicted.txt\n\
? new_file.rs\n";

    let status = parse_status_output(PathBuf::from("/tmp/repo"), output);
    assert_eq!(status.branch.as_deref(), Some("feature-x"));
    assert_eq!(status.upstream.as_deref(), Some("origin/feature-x"));
    assert_eq!(status.head.as_deref(), Some("01234567"));
    assert_eq!(status.branch_files, 0);
    assert_eq!(status.branch_additions, 0);
    assert_eq!(status.branch_deletions, 0);
    assert_eq!(status.pr_number, None);
    assert_eq!(status.ahead, 2);
    assert_eq!(status.behind, 1);
    assert_eq!(status.staged, 2);
    assert_eq!(status.modified, 1);
    assert_eq!(status.untracked, 1);
    assert_eq!(status.conflicted, 1);
    assert_eq!(status.total_changes(), 5);
}

#[test]
fn parses_worktree_porcelain_output() {
    let output = "\
worktree /repo\n\
HEAD 0123456789abcdef\n\
branch refs/heads/main\n\
\n\
worktree /repo-feature\n\
HEAD fedcba9876543210\n\
branch refs/heads/feature-x\n";

    let worktrees = parse_worktree_output(output);
    assert_eq!(worktrees.len(), 2);
    assert_eq!(worktrees[0].path, PathBuf::from("/repo"));
    assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
    assert_eq!(worktrees[0].head.as_deref(), Some("01234567"));
    assert_eq!(worktrees[1].branch.as_deref(), Some("feature-x"));
    assert_eq!(worktrees[1].head.as_deref(), Some("fedcba98"));
}

#[test]
fn parses_numstat_branch_diff_summary() {
    let output = "\
10\t2\tsrc/main.rs\n\
4\t0\tsrc/lib.rs\n\
-\t-\tbinary.file\n";

    let summary = parse_numstat(output);
    assert_eq!(summary.files, 3);
    assert_eq!(summary.additions, 14);
    assert_eq!(summary.deletions, 2);
}

#[test]
fn extracts_pr_number_from_branch_name() {
    assert_eq!(extract_pr_from_branch_name("pr-29"), Some(29));
    assert_eq!(extract_pr_from_branch_name("feature/pull/104"), Some(104));
    assert_eq!(extract_pr_from_branch_name("feature/new-ui"), None);
}
