use std::path::{Path, PathBuf};

use axum::{extract::Query, Json};
use tokio::fs;

use super::policy::FilesPolicy;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{BrowseEntry, BrowseQuery, BrowseResponse};
use crate::utils::workspace::resolve_scoped_workspace_path;

pub(super) async fn browse_directories(
    user: Option<CurrentUser>,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, AppError> {
    let canonical_home = resolve_browse_home(user.as_ref())?;
    let canonical_current = resolve_browse_root(&canonical_home, query.path.as_deref())?;

    if !canonical_current.is_dir() {
        return Err(AppError::BadRequest(format!(
            "Path is not a directory: {}",
            canonical_current.display()
        )));
    }

    let parent = if canonical_current != canonical_home {
        canonical_current.parent().map(|p| p.display().to_string())
    } else {
        None
    };

    let directories = list_visible_directories(&canonical_current, FilesPolicy::default()).await?;

    Ok(Json(BrowseResponse {
        current: canonical_current.display().to_string(),
        parent,
        directories,
    }))
}

fn resolve_browse_home(user: Option<&CurrentUser>) -> Result<PathBuf, AppError> {
    let home = user
        .and_then(|u| u.0.home_dir.clone())
        .or_else(dirs::home_dir)
        .ok_or_else(|| AppError::Internal("Could not determine home directory".to_string()))?;

    home.canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to canonicalize home: {}", e)))
}

fn resolve_browse_root(home: &Path, requested: Option<&str>) -> Result<PathBuf, AppError> {
    let current_path = resolve_scoped_workspace_path(requested, home, home)?;
    let canonical_current = current_path.canonicalize().map_err(|_| {
        AppError::NotFound(format!("Directory not found: {}", current_path.display()))
    })?;

    if !canonical_current.starts_with(home) {
        return Err(AppError::BadRequest(
            "Path must be within home directory".to_string(),
        ));
    }

    Ok(canonical_current)
}

async fn list_visible_directories(
    path: &Path,
    policy: FilesPolicy,
) -> Result<Vec<BrowseEntry>, AppError> {
    let mut directories = Vec::new();
    let mut read_dir = fs::read_dir(path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read entry: {}", e)))?
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if policy.browse_hides(&name) {
            continue;
        }

        directories.push(BrowseEntry {
            name,
            path: path.display().to_string(),
        });
    }

    directories.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(directories)
}
