use regex::Regex;
use std::sync::LazyLock;

/// Matches `git commit` but not git plumbing commands like `git commit-tree`
static GIT_COMMIT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"git\s+commit\b").unwrap());

static CO_AUTHORED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)co-authored-by\s*:").unwrap());

/// Returns true if `git commit` at this position is the porcelain command,
/// not a plumbing command like `git commit-tree` or `git commit-graph`.
fn is_porcelain_commit(command: &str, match_end: usize) -> bool {
    let rest = &command[match_end..];
    // If the next char is '-', it's a plumbing command (commit-tree, commit-graph)
    !rest.starts_with('-')
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitIdentity {
    pub name: String,
    pub email: String,
    pub mode: GitIdentityMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitIdentityMode {
    CoAuthor,
    Author,
    Disabled,
}

impl Default for GitIdentity {
    fn default() -> Self {
        Self {
            name: "Mitsuro".to_string(),
            email: "krusty@users.noreply.github.com".to_string(),
            mode: GitIdentityMode::CoAuthor,
        }
    }
}

impl GitIdentity {
    pub fn trailer_line(&self) -> String {
        format!("Co-Authored-By: {} <{}>", self.name, self.email)
    }

    /// For CoAuthor mode, detect `git commit` commands and inject `--trailer`.
    /// For Author/Disabled, return the command unchanged.
    pub fn apply_to_command(&self, command: &str) -> String {
        if self.mode != GitIdentityMode::CoAuthor {
            return command.to_string();
        }

        // Skip if command already contains a Co-Authored-By reference
        if CO_AUTHORED_RE.is_match(command) {
            return command.to_string();
        }

        // Check if there's a porcelain `git commit` (not `git commit-tree` etc.)
        let has_porcelain_commit = GIT_COMMIT_RE
            .find_iter(command)
            .any(|m| is_porcelain_commit(command, m.end()));

        if !has_porcelain_commit {
            return command.to_string();
        }

        let trailer = self.trailer_line();
        let trailer_flag = format!(" --trailer \"{}\"", trailer);

        // Build result by processing matches and only injecting trailer for porcelain commits
        let mut result = String::with_capacity(command.len() + trailer_flag.len());
        let mut last_end = 0;
        for m in GIT_COMMIT_RE.find_iter(command) {
            result.push_str(&command[last_end..m.end()]);
            if is_porcelain_commit(command, m.end()) {
                result.push_str(&trailer_flag);
            }
            last_end = m.end();
        }
        result.push_str(&command[last_end..]);
        result
    }

    /// For Author mode, return env var pairs for GIT_AUTHOR/COMMITTER.
    /// For CoAuthor/Disabled, return empty.
    pub fn env_vars(&self) -> Vec<(&str, &str)> {
        if self.mode != GitIdentityMode::Author {
            return Vec::new();
        }

        vec![
            ("GIT_AUTHOR_NAME", self.name.as_str()),
            ("GIT_AUTHOR_EMAIL", self.email.as_str()),
            ("GIT_COMMITTER_NAME", self.name.as_str()),
            ("GIT_COMMITTER_EMAIL", self.email.as_str()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_identity() {
        let id = GitIdentity::default();
        assert_eq!(id.name, "Mitsuro");
        assert_eq!(id.email, "krusty@users.noreply.github.com");
        assert_eq!(id.mode, GitIdentityMode::CoAuthor);
    }

    #[test]
    fn trailer_line_format() {
        let id = GitIdentity::default();
        assert_eq!(
            id.trailer_line(),
            "Co-Authored-By: Mitsuro <krusty@users.noreply.github.com>"
        );
    }

    #[test]
    fn apply_simple_commit() {
        let id = GitIdentity::default();
        let cmd = r#"git commit -m "fix: something""#;
        let result = id.apply_to_command(cmd);
        assert!(result.contains("--trailer"));
        assert!(result.contains("Co-Authored-By: Mitsuro"));
    }

    #[test]
    fn apply_chained_commands() {
        let id = GitIdentity::default();
        let cmd = r#"git add . && git commit -m "feat: new thing""#;
        let result = id.apply_to_command(cmd);
        assert!(result.contains("--trailer"));
        assert!(result.contains("git add ."));
    }

    #[test]
    fn skip_when_already_has_coauthor() {
        let id = GitIdentity::default();
        let cmd = r#"git commit -m "fix: x" --trailer "Co-Authored-By: Someone <a@b.com>""#;
        let result = id.apply_to_command(cmd);
        assert_eq!(result, cmd);
    }

    #[test]
    fn skip_git_commit_tree() {
        let id = GitIdentity::default();
        let cmd = "git commit-tree abc123";
        let result = id.apply_to_command(cmd);
        assert_eq!(result, cmd);
    }

    #[test]
    fn no_modification_in_author_mode() {
        let id = GitIdentity {
            mode: GitIdentityMode::Author,
            ..Default::default()
        };
        let cmd = r#"git commit -m "test""#;
        assert_eq!(id.apply_to_command(cmd), cmd);
    }

    #[test]
    fn no_modification_in_disabled_mode() {
        let id = GitIdentity {
            mode: GitIdentityMode::Disabled,
            ..Default::default()
        };
        let cmd = r#"git commit -m "test""#;
        assert_eq!(id.apply_to_command(cmd), cmd);
    }

    #[test]
    fn env_vars_author_mode() {
        let id = GitIdentity {
            mode: GitIdentityMode::Author,
            ..Default::default()
        };
        let vars = id.env_vars();
        assert_eq!(vars.len(), 4);
        assert!(vars.contains(&("GIT_AUTHOR_NAME", "Mitsuro")));
        assert!(vars.contains(&("GIT_AUTHOR_EMAIL", "krusty@users.noreply.github.com")));
    }

    #[test]
    fn env_vars_empty_for_coauthor() {
        let id = GitIdentity::default();
        assert!(id.env_vars().is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let id = GitIdentity::default();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: GitIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, id.name);
        assert_eq!(parsed.email, id.email);
        assert_eq!(parsed.mode, id.mode);
    }

    #[test]
    fn serde_mode_snake_case() {
        let json = r#"{"name":"K","email":"k@k.com","mode":"co_author"}"#;
        let id: GitIdentity = serde_json::from_str(json).unwrap();
        assert_eq!(id.mode, GitIdentityMode::CoAuthor);
    }

    #[test]
    fn apply_heredoc_commit() {
        let id = GitIdentity::default();
        let cmd = r#"git commit -m "$(cat <<'EOF'
feat: add feature

Some description
EOF
)""#;
        let result = id.apply_to_command(cmd);
        assert!(result.contains("--trailer"));
    }

    #[test]
    fn no_modification_for_non_git_commands() {
        let id = GitIdentity::default();
        let cmd = "cargo build --release";
        assert_eq!(id.apply_to_command(cmd), cmd);
    }
}
