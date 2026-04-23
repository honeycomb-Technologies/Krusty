use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    Json,
};
use tokio::fs;

use super::super::session_access::request_workspace_scope;
use super::policy::FilesPolicy;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{
    FileQuery, FileResponse, FileWriteRequest, FileWriteResponse, TreeEntry, TreeQuery,
    TreeResponse,
};
use crate::utils::workspace::resolve_scoped_workspace_path;
use crate::AppState;

pub(super) async fn read_file(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<FileQuery>,
) -> Result<Json<FileResponse>, AppError> {
    let policy = FilesPolicy::default();
    let path = resolve_file_path(&state, user.as_ref(), Some(query.path.as_str()))?;

    let metadata = fs::metadata(&path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::NotFound(format!("File not found: {}", query.path))
        } else {
            AppError::Internal(format!("Failed to read file metadata: {}", e))
        }
    })?;

    if metadata.is_dir() {
        return Err(AppError::BadRequest(format!(
            "Path is a directory: {}",
            query.path
        )));
    }

    if policy.exceeds_text_file_limit(metadata.len()) {
        return Err(AppError::BadRequest(policy.read_limit_error()));
    }

    let bytes = fs::read(&path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read file: {}", e)))?;
    let content = String::from_utf8(bytes)
        .map_err(|_| AppError::BadRequest(policy.non_utf8_error().to_string()))?;

    Ok(Json(FileResponse {
        path: query.path,
        content,
        size: metadata.len(),
    }))
}

pub(super) async fn write_file(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<FileQuery>,
    Json(req): Json<FileWriteRequest>,
) -> Result<Json<FileWriteResponse>, AppError> {
    let policy = FilesPolicy::default();
    if policy.exceeds_text_file_limit(req.content.len() as u64) {
        return Err(AppError::BadRequest(policy.write_limit_error()));
    }

    let path = resolve_file_path(&state, user.as_ref(), Some(query.path.as_str()))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create directories: {}", e)))?;
    }

    let bytes = req.content.as_bytes();
    fs::write(&path, bytes)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {}", e)))?;

    Ok(Json(FileWriteResponse {
        path: query.path,
        bytes_written: bytes.len(),
    }))
}

pub(super) async fn get_tree(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, AppError> {
    let root_path = resolve_file_path(&state, user.as_ref(), query.root.as_deref())?;

    if !root_path.is_dir() {
        return Err(AppError::BadRequest(format!(
            "Path is not a directory: {}",
            root_path.display()
        )));
    }

    let policy = FilesPolicy::default();
    let counter = Arc::new(AtomicUsize::new(0));
    let entries = build_tree(&root_path, query.depth, &counter, policy).await?;

    Ok(Json(TreeResponse {
        root: root_path.display().to_string(),
        entries,
    }))
}

pub(super) fn resolve_file_path(
    state: &AppState,
    user: Option<&CurrentUser>,
    requested: Option<&str>,
) -> Result<PathBuf, AppError> {
    let workspace_scope = request_workspace_scope(state, user);
    resolve_scoped_workspace_path(
        requested,
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )
}

async fn build_tree(
    path: &Path,
    depth: usize,
    counter: &Arc<AtomicUsize>,
    policy: FilesPolicy,
) -> Result<Vec<TreeEntry>, AppError> {
    if depth == 0 || counter.load(Ordering::Relaxed) >= policy.max_tree_entries() {
        return Ok(vec![]);
    }

    let mut entries = Vec::new();
    let mut read_dir = fs::read_dir(path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read directory entry: {}", e)))?
    {
        if counter.load(Ordering::Relaxed) >= policy.max_tree_entries() {
            break;
        }

        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if policy.workspace_tree_hides(&name) {
            continue;
        }

        counter.fetch_add(1, Ordering::Relaxed);

        let is_dir = entry_path.is_dir();
        let children = if is_dir && depth > 1 {
            Some(Box::pin(build_tree(&entry_path, depth - 1, counter, policy)).await?)
        } else {
            None
        };

        entries.push(TreeEntry {
            name,
            path: entry_path.display().to_string(),
            is_dir,
            children,
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });

    Ok(entries)
}
