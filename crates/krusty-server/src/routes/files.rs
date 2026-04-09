//! File operations endpoints

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use tokio::fs;

use super::session_access::request_workspace_scope;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{
    BrowseEntry, BrowseQuery, BrowseResponse, FileQuery, FileResponse, FileWriteRequest,
    FileWriteResponse, TreeEntry, TreeQuery, TreeResponse,
};
use crate::utils::workspace::resolve_scoped_workspace_path;
use crate::AppState;

/// Build the files router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(read_file).put(write_file))
        .route("/tree", get(get_tree))
        .route("/browse", get(browse_directories))
}

/// Read a file's contents
async fn read_file(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<FileQuery>,
) -> Result<Json<FileResponse>, AppError> {
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

    let content = fs::read_to_string(&path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read file: {}", e)))?;

    Ok(Json(FileResponse {
        path: query.path,
        content,
        size: metadata.len(),
    }))
}

/// Write content to a file
async fn write_file(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<FileQuery>,
    Json(req): Json<FileWriteRequest>,
) -> Result<Json<FileWriteResponse>, AppError> {
    const MAX_FILE_WRITE_SIZE: usize = 100 * 1024 * 1024; // 100MB
    if req.content.len() > MAX_FILE_WRITE_SIZE {
        return Err(AppError::BadRequest(
            "File content exceeds maximum size of 100MB".to_string(),
        ));
    }

    let path = resolve_file_path(&state, user.as_ref(), Some(query.path.as_str()))?;

    // Create parent directories if needed
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

/// Get directory tree
async fn get_tree(
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

    let counter = Arc::new(AtomicUsize::new(0));
    let entries = build_tree(&root_path, query.depth, &counter).await?;

    Ok(Json(TreeResponse {
        root: root_path.display().to_string(),
        entries,
    }))
}

const MAX_TREE_ENTRIES: usize = 10_000;

fn resolve_file_path(
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

/// Recursively build directory tree
async fn build_tree(
    path: &Path,
    depth: usize,
    counter: &Arc<AtomicUsize>,
) -> Result<Vec<TreeEntry>, AppError> {
    if depth == 0 || counter.load(Ordering::Relaxed) >= MAX_TREE_ENTRIES {
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
        if counter.load(Ordering::Relaxed) >= MAX_TREE_ENTRIES {
            break;
        }

        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files and common ignore patterns
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }

        counter.fetch_add(1, Ordering::Relaxed);

        let is_dir = entry_path.is_dir();
        let children = if is_dir && depth > 1 {
            Some(Box::pin(build_tree(&entry_path, depth - 1, counter)).await?)
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

    // Sort: directories first, then alphabetically
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });

    Ok(entries)
}

/// Browse directories for project selection (not restricted to working dir)
async fn browse_directories(
    user: Option<CurrentUser>,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, AppError> {
    // In multi-tenant mode, scope to user's workspace; otherwise use home dir
    let home = user
        .as_ref()
        .and_then(|u| u.0.home_dir.clone())
        .or_else(dirs::home_dir)
        .ok_or_else(|| AppError::Internal("Could not determine home directory".to_string()))?;

    let canonical_home = home
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to canonicalize home: {}", e)))?;
    let current_path =
        resolve_scoped_workspace_path(query.path.as_deref(), &canonical_home, &canonical_home)?;

    let canonical_current = current_path.canonicalize().map_err(|_| {
        AppError::NotFound(format!("Directory not found: {}", current_path.display()))
    })?;

    if !canonical_current.starts_with(&canonical_home) {
        return Err(AppError::BadRequest(
            "Path must be within home directory".to_string(),
        ));
    }

    if !canonical_current.is_dir() {
        return Err(AppError::BadRequest(format!(
            "Path is not a directory: {}",
            current_path.display()
        )));
    }

    // Get parent (if not at home)
    let parent = if canonical_current != canonical_home {
        canonical_current.parent().map(|p| p.display().to_string())
    } else {
        None
    };

    // List directories only
    let mut directories = Vec::new();
    let mut read_dir = fs::read_dir(&canonical_current)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read entry: {}", e)))?
    {
        let path = entry.path();

        // Only directories
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();

        // Skip hidden directories
        if name.starts_with('.') {
            continue;
        }

        directories.push(BrowseEntry {
            name,
            path: path.display().to_string(),
        });
    }

    // Sort alphabetically
    directories.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(Json(BrowseResponse {
        current: canonical_current.display().to_string(),
        parent,
        directories,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::{extract::Query, Json};
    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::Database;
    use krusty_core::tools::registry::ToolRegistry;

    use super::{browse_directories, resolve_file_path};
    use crate::auth::{AuthenticatedUser, CurrentUser};
    use crate::error::AppError;
    use crate::types::BrowseQuery;
    use crate::AppState;

    fn create_test_state() -> (AppState, PathBuf) {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-server-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("krusty.db");
        Database::new(&db_path).expect("database should initialize");
        let working_dir = temp_dir.join("workspace");
        std::fs::create_dir_all(&working_dir).expect("workspace should exist");

        (
            AppState {
                server_port: 3000,
                db_path: Arc::new(db_path),
                working_dir: Arc::new(working_dir.clone()),
                ai_client: None,
                tool_registry: Arc::new(ToolRegistry::new()),
                process_registry: Arc::new(ProcessRegistry::new()),
                model_registry: create_model_registry(),
                credential_store: Arc::new(RwLock::new(CredentialStore::default())),
                mcp_manager: Arc::new(McpManager::new(working_dir.clone())),
                hook_manager: Arc::new(RwLock::new(UserHookManager::new())),
                skills_manager: Arc::new(RwLock::new(SkillsManager::with_defaults(&working_dir))),
                cancellation: AgentCancellation::new(),
                session_locks: Arc::new(RwLock::new(HashMap::new())),
                session_inputs: Arc::new(RwLock::new(HashMap::new())),
                session_presence: Arc::new(RwLock::new(HashMap::new())),
                delegated_state: Arc::new(RwLock::new(HashMap::new())),
                remote_access: Arc::new(RwLock::new(crate::remote_access::RemoteAccessConfig {
                    enabled: true,
                    token: "test-token".to_string(),
                })),
                active_agent_streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                peak_rss_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                peak_virtual_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                push_service: None,
                apns_service: None,
                oauth_flows: Arc::new(Mutex::new(HashMap::new())),
                mako_runtime: crate::mako_runtime::MakoRuntimeManager::new(),
            },
            temp_dir,
        )
    }

    fn current_user(user_id: &str, home_dir: &std::path::Path) -> CurrentUser {
        CurrentUser(AuthenticatedUser {
            user_id: Some(user_id.to_string()),
            home_dir: Some(home_dir.to_path_buf()),
        })
    }

    #[tokio::test]
    async fn resolve_file_path_rejects_paths_outside_user_root() {
        let (state, temp_dir) = create_test_state();
        let user_root = temp_dir.join("user");
        let outside_root = temp_dir.join("outside");
        std::fs::create_dir_all(&user_root).expect("user root should exist");
        std::fs::create_dir_all(&outside_root).expect("outside root should exist");

        let result = resolve_file_path(
            &state,
            Some(&current_user("alice", &user_root)),
            Some(outside_root.to_string_lossy().as_ref()),
        );

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn resolve_file_path_allows_relative_paths_within_user_root() {
        let (state, temp_dir) = create_test_state();
        let user_root = temp_dir.join("user");
        let repo_dir = user_root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo dir should exist");

        let result = resolve_file_path(
            &state,
            Some(&current_user("alice", &user_root)),
            Some("repo"),
        );
        let path = match result {
            Ok(path) => path,
            Err(_) => panic!("file path should resolve"),
        };

        assert_eq!(path, repo_dir);
    }

    #[tokio::test]
    async fn browse_directories_resolves_relative_paths_within_user_root() {
        let (_state, temp_dir) = create_test_state();
        let user_root = temp_dir.join("user");
        let repo_dir = user_root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo dir should exist");

        let Json(response) = browse_directories(
            Some(current_user("alice", &user_root)),
            Query(BrowseQuery {
                path: Some("repo".to_string()),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("browse should succeed"));

        assert_eq!(response.current, repo_dir.to_string_lossy());
    }

    #[tokio::test]
    async fn browse_directories_rejects_paths_outside_user_root() {
        let (_state, temp_dir) = create_test_state();
        let user_root = temp_dir.join("user");
        let outside_root = temp_dir.join("outside");
        std::fs::create_dir_all(&user_root).expect("user root should exist");
        std::fs::create_dir_all(&outside_root).expect("outside root should exist");

        let result = browse_directories(
            Some(current_user("alice", &user_root)),
            Query(BrowseQuery {
                path: Some(outside_root.to_string_lossy().to_string()),
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
