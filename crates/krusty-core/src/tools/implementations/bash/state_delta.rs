//! Bounded positive state evidence for foreground shell calls.
//!
//! Free-form shell cannot safely produce a negative `changed=false` proof:
//! PATH, control flow, shell dialects, redirects, symlinks, and external
//! configuration all make complete effect inference intractable. This module
//! therefore tracks only literal workspace-scoped output-redirection targets
//! and reports `changed=true` when bounded metadata demonstrably differs.
//! Equality and every unparsed command remain opaque; the semantic progress
//! ledger owns repeated-intent convergence.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::tools::registry::progress_change_key_for_paths;

const MAX_TARGETS: usize = 64;
const SNAPSHOT_WALL_BUDGET: Duration = Duration::from_millis(250);
static SNAPSHOT_SLOT: Lazy<std::sync::Arc<Semaphore>> =
    Lazy::new(|| std::sync::Arc::new(Semaphore::new(1)));

pub(super) struct BashStateDeltaProbe {
    targets: Vec<PathBuf>,
    before: Vec<PathSnapshot>,
    change_key: String,
}

impl BashStateDeltaProbe {
    pub(super) async fn capture(
        command: &str,
        working_dir: &Path,
        sandbox_root: Option<&Path>,
    ) -> Option<Self> {
        // Krusty executes `cmd /C` on Windows. POSIX redirect parsing must not
        // make claims about cmd.exe quoting, backslashes, or `%VAR%` expansion.
        if cfg!(windows) {
            return None;
        }

        let permit = std::sync::Arc::clone(&SNAPSHOT_SLOT)
            .try_acquire_owned()
            .ok()?;
        let command = command.to_string();
        let working_dir = working_dir.to_path_buf();
        let sandbox_root = sandbox_root.map(Path::to_path_buf);
        let capture = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let working_dir = working_dir.canonicalize().ok()?;
            let scope_root = sandbox_root
                .and_then(|root| root.canonicalize().ok())
                .filter(|root| working_dir.starts_with(root))
                .unwrap_or_else(|| working_dir.clone());
            let targets = literal_output_targets(&command, &working_dir, &scope_root);
            if targets.is_empty() {
                return None;
            }
            let before = snapshot_paths(&targets);
            let change_key = target_change_key(&targets, &scope_root);
            Some(Self {
                targets,
                before,
                change_key,
            })
        });

        tokio::time::timeout(SNAPSHOT_WALL_BUDGET, capture)
            .await
            .ok()?
            .ok()?
    }

    pub(super) async fn changed(self) -> Option<String> {
        let permit = std::sync::Arc::clone(&SNAPSHOT_SLOT)
            .try_acquire_owned()
            .ok()?;
        let targets = self.targets;
        let capture = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            snapshot_paths(&targets)
        });
        let after = tokio::time::timeout(SNAPSHOT_WALL_BUDGET, capture)
            .await
            .ok()?
            .ok()?;

        self.before
            .iter()
            .zip(after.iter())
            .any(|(before, after)| before.differs_from(after))
            .then_some(self.change_key)
    }
}

fn target_change_key(targets: &[PathBuf], scope_root: &Path) -> String {
    progress_change_key_for_paths(targets, scope_root)
}

fn literal_output_targets(command: &str, working_dir: &Path, scope_root: &Path) -> Vec<PathBuf> {
    let characters = command.chars().collect::<Vec<_>>();
    let mut targets = BTreeSet::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut index = 0;

    while index < characters.len() && targets.len() < MAX_TARGETS {
        let character = characters[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match character {
            '\\' if !in_single => {
                escaped = true;
                index += 1;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                index += 1;
            }
            '"' if !in_single => {
                in_double = !in_double;
                index += 1;
            }
            '>' if !in_single && !in_double => {
                index += 1;
                if matches!(characters.get(index), Some('>' | '|')) {
                    index += 1;
                }
                // `>&1` and `>& file` have descriptor/dialect-dependent
                // semantics. Missing them is safe because evidence is positive
                // only; guessing a target would not be.
                if characters.get(index) == Some(&'&') {
                    index += 1;
                    continue;
                }
                while matches!(characters.get(index), Some(' ' | '\t')) {
                    index += 1;
                }
                let (raw_target, next_index) = parse_literal_shell_word(&characters, index);
                index = next_index.max(index + 1);
                let Some(raw_target) = raw_target else {
                    continue;
                };
                if let Some(path) =
                    resolve_scoped_literal_path(working_dir, scope_root, raw_target.as_str())
                {
                    targets.insert(path);
                }
            }
            _ => index += 1,
        }
    }

    targets.into_iter().collect()
}

fn parse_literal_shell_word(characters: &[char], mut index: usize) -> (Option<String>, usize) {
    let mut word = String::new();
    let mut in_single = false;
    let mut in_double = false;

    while index < characters.len() {
        let character = characters[index];
        if in_single {
            if character == '\'' {
                in_single = false;
            } else {
                word.push(character);
            }
            index += 1;
            continue;
        }
        if in_double {
            match character {
                '"' => in_double = false,
                '\\' => {
                    index += 1;
                    let Some(escaped) = characters.get(index).copied() else {
                        return (None, index);
                    };
                    if matches!(escaped, '$' | '`' | '"' | '\\') {
                        word.push(escaped);
                    } else if escaped != '\n' {
                        word.push('\\');
                        word.push(escaped);
                    }
                }
                _ => word.push(character),
            }
            index += 1;
            continue;
        }

        match character {
            '\'' => in_single = true,
            '"' => in_double = true,
            '\\' => {
                index += 1;
                let Some(escaped) = characters.get(index).copied() else {
                    return (None, index);
                };
                if escaped != '\n' {
                    word.push(escaped);
                }
            }
            ' ' | '\t' | '\n' | ';' | '|' | '&' | '<' | '>' => break,
            _ => word.push(character),
        }
        index += 1;
    }

    if in_single || in_double || word.is_empty() {
        return (None, index);
    }
    if word.starts_with('~')
        || word
            .chars()
            .any(|character| matches!(character, '$' | '`' | '*' | '?' | '[' | ']' | '{' | '}'))
    {
        return (None, index);
    }
    (Some(word), index)
}

fn resolve_scoped_literal_path(
    working_dir: &Path,
    scope_root: &Path,
    raw: &str,
) -> Option<PathBuf> {
    if matches!(raw, "/dev/null" | "/dev/stdout" | "/dev/stderr") {
        return None;
    }
    let raw_path = Path::new(raw);
    let joined = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        working_dir.join(raw_path)
    };
    let normalized = lexical_normalize(&joined)?;
    if !normalized.starts_with(scope_root)
        || !nearest_existing_ancestor_is_scoped(&normalized, scope_root)
    {
        return None;
    }
    Some(normalized)
}

fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Some(normalized)
}

fn nearest_existing_ancestor_is_scoped(path: &Path, scope_root: &Path) -> bool {
    path.ancestors()
        .find(|ancestor| ancestor.exists())
        .and_then(|ancestor| ancestor.canonicalize().ok())
        .is_some_and(|ancestor| ancestor.starts_with(scope_root))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSnapshot {
    Absent,
    Known([u8; 32]),
    Unknown,
}

impl PathSnapshot {
    fn differs_from(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Absent, Self::Known(_)) | (Self::Known(_), Self::Absent) => true,
            (Self::Known(left), Self::Known(right)) => left != right,
            _ => false,
        }
    }
}

fn snapshot_paths(paths: &[PathBuf]) -> Vec<PathSnapshot> {
    paths.iter().map(|path| snapshot_path(path)).collect()
}

fn snapshot_path(path: &Path) -> PathSnapshot {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return PathSnapshot::Absent,
        Err(_) => return PathSnapshot::Unknown,
    };
    let mut hasher = Sha256::new();
    let kind = if metadata.is_file() {
        b"file".as_slice()
    } else if metadata.is_dir() {
        b"dir".as_slice()
    } else if metadata.file_type().is_symlink() {
        b"symlink".as_slice()
    } else {
        return PathSnapshot::Unknown;
    };
    hasher.update(kind);
    hasher.update(metadata.len().to_le_bytes());
    hasher.update([u8::from(metadata.permissions().readonly())]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hasher.update(metadata.mode().to_le_bytes());
        hasher.update(metadata.uid().to_le_bytes());
        hasher.update(metadata.gid().to_le_bytes());
        hasher.update(metadata.dev().to_le_bytes());
        hasher.update(metadata.ino().to_le_bytes());
        hasher.update(metadata.nlink().to_le_bytes());
    }
    if metadata.file_type().is_symlink() {
        let Ok(target) = fs::read_link(path) else {
            return PathSnapshot::Unknown;
        };
        hasher.update(target.as_os_str().to_string_lossy().as_bytes());
    }
    PathSnapshot::Known(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_literal_scoped_output_redirects() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("root");
        let targets = literal_output_targets(
            "printf x>>log; printf y > './other'; printf z 2>&1; printf q > $DYNAMIC",
            &root,
            &root,
        );
        assert_eq!(targets, vec![root.join("log"), root.join("other")]);
    }

    #[test]
    fn posix_backslash_and_non_ascii_path_spelling_are_preserved() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("root");
        let targets = literal_output_targets(
            "printf x > \"foo\\q\"; printf y > foo\\\nbar; printf z > foo\u{a0}bar",
            &root,
            &root,
        );
        assert_eq!(
            targets,
            vec![
                root.join("foo\\q"),
                root.join("foobar"),
                root.join("foo\u{a0}bar")
            ]
        );
    }

    #[test]
    fn metadata_snapshot_detects_append_and_replace_without_opening_content() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("state");
        fs::write(&path, "a").expect("initial");
        let before = snapshot_path(&path);
        assert!(!before.differs_from(&snapshot_path(&path)));

        fs::write(&path, "longer").expect("append-sized change");
        assert!(before.differs_from(&snapshot_path(&path)));
    }
}
