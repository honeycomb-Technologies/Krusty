use std::path::{Path, PathBuf};

use crate::error::AppError;

pub fn validate_path_within(base: &Path, path: &Path) -> Result<(), AppError> {
    let canonical_base = base
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to canonicalize base dir: {}", e)))?;

    let check_path = find_existing_ancestor(path)
        .ok_or_else(|| AppError::BadRequest("Invalid path: no existing ancestor".to_string()))?;

    let canonical_check = check_path
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to canonicalize path: {}", e)))?;

    if !canonical_check.starts_with(&canonical_base) {
        return Err(AppError::BadRequest(
            "Path must be within allowed directory".to_string(),
        ));
    }

    Ok(())
}

pub fn find_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}
