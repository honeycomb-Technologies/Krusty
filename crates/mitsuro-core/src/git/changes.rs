use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path};

use anyhow::{anyhow, bail, Result};

use super::command::{resolve_repo_root, run_git, should_suppress_display};
use super::model::{GitChangedFileSummary, GitChangesSummary, GitFileDiff};
use super::status::diff::resolve_base_ref;

const MAX_PATCH_BYTES: usize = 240_000;

/// List files changed from the current branch base through the working tree.
pub fn changes(path: &Path) -> Result<Option<GitChangesSummary>> {
    let repo_root = match resolve_repo_root(path)? {
        Some(root) => root,
        None => return Ok(None),
    };
    if should_suppress_display(&repo_root) {
        return Ok(None);
    }

    let base = diff_base(&repo_root)?;
    let base_arg = base.as_deref().unwrap_or("HEAD");
    let name_output = run_git(
        &[
            "diff",
            "--no-ext-diff",
            "--no-renames",
            "--name-status",
            "-z",
            base_arg,
            "--",
        ],
        &repo_root,
    )?;
    let numstat_output = run_git(
        &[
            "diff",
            "--no-ext-diff",
            "--no-renames",
            "--numstat",
            "-z",
            base_arg,
            "--",
        ],
        &repo_root,
    )?;

    let counts = parse_numstat(&numstat_output.stdout);
    let mut files = parse_name_status(&name_output.stdout)
        .into_iter()
        .map(|(path, status)| {
            let (additions, deletions) = counts.get(&path).copied().unwrap_or_default();
            GitChangedFileSummary {
                path,
                status,
                additions,
                deletions,
            }
        })
        .collect::<Vec<_>>();

    let untracked_output = run_git(
        &["ls-files", "--others", "--exclude-standard", "-z"],
        &repo_root,
    )?;
    for path in nul_fields(&untracked_output.stdout) {
        if files.iter().any(|file| file.path == path) {
            continue;
        }
        let additions = count_text_lines(&repo_root.join(&path)).unwrap_or_default();
        files.push(GitChangedFileSummary {
            path,
            status: "untracked".to_string(),
            additions,
            deletions: 0,
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(Some(GitChangesSummary { repo_root, files }))
}

/// Load a bounded unified patch for one changed file.
pub fn file_diff(path: &Path, file: &str) -> Result<Option<GitFileDiff>> {
    validate_relative_file(file)?;
    let repo_root = match resolve_repo_root(path)? {
        Some(root) => root,
        None => return Ok(None),
    };
    if should_suppress_display(&repo_root) {
        return Ok(None);
    }

    let changed = changes(&repo_root)?
        .ok_or_else(|| anyhow!("Path is not inside a displayable git repository"))?;
    let entry = changed
        .files
        .iter()
        .find(|entry| entry.path == file)
        .ok_or_else(|| anyhow!("File is not part of the current repository changes"))?;

    if entry.status == "untracked" {
        return Ok(Some(untracked_diff(&repo_root, file)?));
    }

    let base = diff_base(&repo_root)?;
    let base_arg = base.as_deref().unwrap_or("HEAD");
    let output = run_git(
        &[
            "diff",
            "--no-ext-diff",
            "--no-renames",
            "--unified=3",
            base_arg,
            "--",
            file,
        ],
        &repo_root,
    )?;
    let binary = output
        .stdout
        .windows(b"Binary files".len())
        .any(|window| window == b"Binary files");
    let (patch, truncated) = bounded_utf8(&output.stdout);
    Ok(Some(GitFileDiff {
        path: file.to_string(),
        patch,
        truncated,
        binary,
    }))
}

fn diff_base(repo_root: &Path) -> Result<Option<String>> {
    let upstream = run_git(
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        repo_root,
    )
    .ok()
    .and_then(|output| {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!value.is_empty()).then_some(value)
    });
    let Some(base_ref) = resolve_base_ref(repo_root, upstream.as_deref()) else {
        return Ok(None);
    };
    let output = run_git(&["merge-base", "HEAD", base_ref.as_str()], repo_root)?;
    let merge_base = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!merge_base.is_empty()).then_some(merge_base))
}

fn validate_relative_file(file: &str) -> Result<()> {
    let path = Path::new(file);
    if file.trim().is_empty() || path.is_absolute() {
        bail!("File path must be a non-empty repository-relative path");
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        bail!("File path must stay inside the repository");
    }
    Ok(())
}

fn parse_name_status(bytes: &[u8]) -> Vec<(String, String)> {
    let fields = nul_fields(bytes);
    fields
        .chunks_exact(2)
        .map(|pair| {
            let status = match pair[0].chars().next().unwrap_or('M') {
                'A' => "added",
                'D' => "deleted",
                'U' => "conflicted",
                'T' => "type changed",
                _ => "modified",
            };
            (pair[1].clone(), status.to_string())
        })
        .collect()
}

fn parse_numstat(bytes: &[u8]) -> HashMap<String, (usize, usize)> {
    nul_fields(bytes)
        .into_iter()
        .filter_map(|field| {
            let mut parts = field.splitn(3, '\t');
            let additions = parts.next()?.parse().unwrap_or(0);
            let deletions = parts.next()?.parse().unwrap_or(0);
            let path = parts.next()?.to_string();
            Some((path, (additions, deletions)))
        })
        .collect()
}

fn nul_fields(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect()
}

fn count_text_lines(path: &Path) -> Option<usize> {
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    Some(text.lines().count())
}

fn untracked_diff(repo_root: &Path, file: &str) -> Result<GitFileDiff> {
    let bytes = fs::read(repo_root.join(file))?;
    if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        return Ok(GitFileDiff {
            path: file.to_string(),
            patch: String::new(),
            truncated: false,
            binary: true,
        });
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut patch = format!(
        "diff --git a/{file} b/{file}\nnew file mode 100644\n--- /dev/null\n+++ b/{file}\n@@ -0,0 +1,{} @@\n",
        text.lines().count()
    );
    for line in text.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    let (patch, truncated) = bounded_utf8(patch.as_bytes());
    Ok(GitFileDiff {
        path: file.to_string(),
        patch,
        truncated,
        binary: false,
    })
}

fn bounded_utf8(bytes: &[u8]) -> (String, bool) {
    if bytes.len() <= MAX_PATCH_BYTES {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    let mut end = MAX_PATCH_BYTES;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    let mut patch = String::from_utf8_lossy(&bytes[..end]).into_owned();
    patch.push_str("\n... diff truncated ...\n");
    (patch, true)
}

#[cfg(test)]
mod tests {
    use super::{parse_name_status, parse_numstat, validate_relative_file};

    #[test]
    fn parses_nul_delimited_change_metadata() {
        let names = parse_name_status(b"M\0src/main.rs\0A\0new file.ts\0");
        assert_eq!(
            names,
            vec![
                ("src/main.rs".to_string(), "modified".to_string()),
                ("new file.ts".to_string(), "added".to_string()),
            ]
        );
        let counts = parse_numstat(b"4\t2\tsrc/main.rs\0-\t-\tasset.png\0");
        assert_eq!(counts.get("src/main.rs"), Some(&(4, 2)));
        assert_eq!(counts.get("asset.png"), Some(&(0, 0)));
    }

    #[test]
    fn rejects_paths_that_escape_the_repository() {
        assert!(validate_relative_file("../secret").is_err());
        assert!(validate_relative_file("/tmp/secret").is_err());
        assert!(validate_relative_file("src/main.rs").is_ok());
    }
}
