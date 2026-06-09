use std::path::PathBuf;

/// Condensed repository status for UI surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusSummary {
    pub repo_root: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    /// Files changed in branch diff (typically merge-base..HEAD).
    pub branch_files: usize,
    /// Added lines in branch diff.
    pub branch_additions: usize,
    /// Deleted lines in branch diff.
    pub branch_deletions: usize,
    /// Added lines in the current uncommitted tracked diff.
    pub worktree_additions: usize,
    /// Deleted lines in the current uncommitted tracked diff.
    pub worktree_deletions: usize,
    /// Current branch PR number (if discoverable).
    pub pr_number: Option<u64>,
    pub ahead: usize,
    pub behind: usize,
    pub staged: usize,
    pub modified: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl GitStatusSummary {
    pub fn total_changes(&self) -> usize {
        self.staged + self.modified + self.untracked + self.conflicted
    }
}

/// Branch metadata (local or remote-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitBranchSummary {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub is_remote: bool,
}

/// Worktree metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitWorktreeSummary {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_current: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct BranchDiffSummary {
    pub(super) files: usize,
    pub(super) additions: usize,
    pub(super) deletions: usize,
}
