use std::path::{Path, PathBuf};

use tracing::warn;

/// Instruction files to search for in the working directory (priority order).
const PROJECT_FILES: &[&str] = &[
    "KRAB.md",
    "krab.md",
    "AGENTS.md",
    "agents.md",
    "CLAUDE.md",
    "claude.md",
    ".cursorrules",
    ".windsurfrules",
    ".clinerules",
    ".github/copilot-instructions.md",
    "JULES.md",
    "gemini.md",
];

/// Build project context from instruction files in the working directory.
///
/// Searches from the project root down to the working directory and
/// concatenates the closest instruction file from each directory.
pub fn build_project_context(working_dir: &Path) -> String {
    let instruction_files = discover_instruction_files(working_dir);
    if instruction_files.is_empty() {
        return String::new();
    }

    let mut sections = Vec::new();
    for path in instruction_files {
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                warn!(path = %path.display(), error = %error, "Failed to read project instruction file");
                continue;
            }
        };
        if content.trim().is_empty() {
            continue;
        }

        let label = path
            .strip_prefix(working_dir)
            .ok()
            .map(|p| p.display().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| path.display().to_string());

        sections.push(format!(
            "[PROJECT INSTRUCTIONS - {}]\n\n{}\n\n[END PROJECT INSTRUCTIONS]",
            label, content
        ));
    }

    sections.join("\n\n")
}

fn discover_instruction_files(working_dir: &Path) -> Vec<PathBuf> {
    let start = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    let root = discover_project_root(&start);
    let mut dirs = Vec::new();
    let mut current = start.as_path();

    loop {
        dirs.push(current.to_path_buf());
        if current == root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }

    dirs.reverse();

    let mut files = Vec::new();
    for dir in dirs {
        if let Some(path) = PROJECT_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
        {
            files.push(path);
        }
    }

    files
}

pub(super) fn discover_named_file(base_dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|name| base_dir.join(name))
        .find(|path| path.is_file())
}

fn discover_project_root(working_dir: &Path) -> &Path {
    for ancestor in working_dir.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor;
        }
    }
    working_dir
}
