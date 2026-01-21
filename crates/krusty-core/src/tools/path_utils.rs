//! Shared path validation utilities for tool implementations

use std::path::{Path, PathBuf};

use crate::tools::registry::ToolResult;

/// Resolve and validate a path against the sandbox root
/// Returns the canonicalized path if valid, or a ToolResult error
pub fn validate_path(
    path: &str,
    sandbox_root: Option<&Path>,
    working_dir: &Path,
) -> Result<PathBuf, ToolResult> {
    let resolved = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        working_dir.join(path)
    };

    let canonical = resolved
        .canonicalize()
        .map_err(|e| ToolResult::error(format!("Cannot resolve path '{}': {}", path, e)))?;

    if let Some(sandbox) = sandbox_root {
        if !canonical.starts_with(sandbox) {
            return Err(ToolResult::error(format!(
                "Access denied: path '{}' is outside workspace",
                path
            )));
        }
    }

    Ok(canonical)
}

/// Validate a path that may not exist yet (for write operations)
/// Checks the parent directory is within sandbox
pub fn validate_new_path(
    path: &str,
    sandbox_root: Option<&Path>,
    working_dir: &Path,
) -> Result<PathBuf, ToolResult> {
    let resolved = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        working_dir.join(path)
    };

    if let Some(parent) = resolved.parent() {
        if parent.exists() {
            let canonical_parent = parent.canonicalize().map_err(|e| {
                ToolResult::error(format!("Cannot resolve parent of '{}': {}", path, e))
            })?;

            if let Some(sandbox) = sandbox_root {
                if !canonical_parent.starts_with(sandbox) {
                    return Err(ToolResult::error(format!(
                        "Access denied: path '{}' is outside workspace",
                        path
                    )));
                }
            }
        } else if let Some(sandbox) = sandbox_root {
            if !resolved.starts_with(sandbox) {
                return Err(ToolResult::error(format!(
                    "Access denied: path '{}' is outside workspace",
                    path
                )));
            }
        }
    }

    Ok(resolved)
}
