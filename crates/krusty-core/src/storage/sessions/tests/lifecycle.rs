use super::{create_test_db, create_test_user};
use crate::agent::pinch_context::{PinchContext, PinchContextInput};
use crate::agent::summarizer::SummarizationResult;
use crate::ai::models::{ApiFormat, ModelAuthScope, ModelKey};
use crate::ai::providers::ProviderId;
use crate::storage::sessions::SessionManager;
use crate::storage::{SessionType, WorkspaceMode};
use crate::tools::registry::PermissionMode;

fn empty_pinch_context(parent_session_id: &str) -> PinchContext {
    PinchContext::from_input(PinchContextInput {
        source_session_id: parent_session_id.to_string(),
        source_session_title: "Parent Session".to_string(),
        summary: SummarizationResult::default(),
        ranked_files: vec![],
        preservation_hints: None,
        direction: None,
        project_context: None,
        key_file_contents: vec![],
        active_plan: None,
    })
}

#[test]
fn session_creation_supports_legacy_required_provider_columns() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let db_path = temp.path().join("legacy.db");
    let conn = rusqlite::Connection::open(&db_path).expect("legacy db");
    conn.execute_batch(
        "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);
         INSERT INTO schema_version (version, applied_at) VALUES (33, CURRENT_TIMESTAMP);
         CREATE TABLE sessions (
             id TEXT PRIMARY KEY,
             title TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             provider TEXT NOT NULL,
             model TEXT NOT NULL,
             metadata TEXT,
             working_dir TEXT,
             project_dir TEXT,
             workspace_mode TEXT NOT NULL DEFAULT 'neutral',
             session_type TEXT NOT NULL DEFAULT 'code',
             user_id TEXT,
             target_branch TEXT,
             permission_mode TEXT NOT NULL DEFAULT 'autonomous'
         );",
    )
    .expect("legacy schema");
    drop(conn);

    let manager =
        SessionManager::new(crate::storage::Database::new(&db_path).expect("open legacy database"));
    let session_id = manager
        .create_session("Legacy Session", None, Some("/tmp"))
        .expect("legacy-compatible session insert");
    let (provider, model): (String, String) = manager
        .db()
        .conn()
        .query_row(
            "SELECT provider, model FROM sessions WHERE id = ?1",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy row");

    assert_eq!(provider, "krusty");
    assert!(model.is_empty());
}

#[test]
fn session_model_selection_preserves_provider_auth_and_transport() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);
    let session_id = manager
        .create_session("Exact model", Some("gpt-shared"), Some("/tmp"))
        .unwrap();
    let key = ModelKey::new(ProviderId::OpenAI, "gpt-shared", ApiFormat::OpenAIResponses)
        .with_auth_scope(ModelAuthScope::OAuth);

    manager
        .update_session_model_selection(&session_id, Some(&key), Some("catalog-a"))
        .unwrap();
    let session = manager.get_session(&session_id).unwrap().unwrap();

    assert_eq!(session.model.as_deref(), Some("gpt-shared"));
    assert_eq!(session.model_key, Some(key));
    assert_eq!(session.model_catalog_revision.as_deref(), Some("catalog-a"));

    manager
        .update_session_model(&session_id, Some("legacy-only"))
        .unwrap();
    let legacy = manager.get_session(&session_id).unwrap().unwrap();
    assert_eq!(legacy.model.as_deref(), Some("legacy-only"));
    assert!(legacy.model_key.is_none());
    assert!(legacy.model_catalog_revision.is_none());
}

#[test]
fn create_linked_session_preserves_workspace_contract() {
    let (db, _temp) = create_test_db();
    let user_id = "workspace-user";
    create_test_user(&db, user_id);

    let manager = SessionManager::new(db);
    let parent_session_id = manager
        .create_session_for_user_with_config(
            "Parent Session",
            Some("gpt-5"),
            Some("/tmp/worktree"),
            Some("/tmp/worktree/apps/mobile"),
            WorkspaceMode::Created,
            Some(user_id),
            Some("feature/mobile-intent"),
            SessionType::Mako,
        )
        .expect("Failed to create parent session");
    let pinch_ctx = empty_pinch_context(&parent_session_id);

    let child_session_id = manager
        .create_linked_session(
            "Child Session",
            &parent_session_id,
            &pinch_ctx,
            Some("gpt-5"),
            Some("/tmp/incorrect-runtime-fallback"),
            None,
            PermissionMode::default(),
        )
        .expect("Failed to create child session");

    let child_session = manager
        .get_session(&child_session_id)
        .expect("Failed to load child session")
        .expect("Child session should exist");

    assert_eq!(child_session.session_type, SessionType::Mako);
    assert_eq!(child_session.working_dir.as_deref(), Some("/tmp/worktree"));
    assert_eq!(
        child_session.project_dir.as_deref(),
        Some("/tmp/worktree/apps/mobile")
    );
    assert_eq!(child_session.workspace_mode, WorkspaceMode::Created);
    assert_eq!(
        child_session.target_branch.as_deref(),
        Some("feature/mobile-intent")
    );
    assert_eq!(child_session.user_id.as_deref(), Some(user_id));
    assert_eq!(
        child_session.parent_session_id.as_deref(),
        Some(parent_session_id.as_str())
    );
    assert_eq!(child_session.permission_mode, PermissionMode::default());
}

#[test]
fn create_linked_session_preserves_explicit_permission_mode() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);
    let parent_session_id = manager
        .create_session_for_user_with_config_and_permission(
            "Supervised Parent",
            Some("gpt-5"),
            Some("/tmp/worktree"),
            Some("/tmp/worktree"),
            WorkspaceMode::Selected,
            None,
            None,
            SessionType::Code,
            PermissionMode::Supervised,
        )
        .expect("Failed to create parent session");
    let pinch_ctx = empty_pinch_context(&parent_session_id);

    let child_session_id = manager
        .create_linked_session(
            "Supervised Child",
            &parent_session_id,
            &pinch_ctx,
            Some("gpt-5"),
            Some("/tmp/worktree"),
            None,
            PermissionMode::Supervised,
        )
        .expect("Failed to create child session");

    let child_session = manager
        .get_session(&child_session_id)
        .expect("Failed to load child session")
        .expect("Child session should exist");

    assert_eq!(child_session.permission_mode, PermissionMode::Supervised);
}

#[test]
fn create_linked_session_preserves_neutral_workspace_without_project() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);
    let parent_session_id = manager
        .create_session_for_user_with_config(
            "Neutral Parent",
            Some("gpt-5"),
            None,
            None,
            WorkspaceMode::Neutral,
            None,
            None,
            SessionType::Chat,
        )
        .expect("Failed to create neutral parent session");
    let pinch_ctx = empty_pinch_context(&parent_session_id);

    let child_session_id = manager
        .create_linked_session(
            "Neutral Child",
            &parent_session_id,
            &pinch_ctx,
            Some("gpt-5"),
            Some("/tmp/server-default-should-not-leak"),
            Some("feature/should-not-leak"),
            PermissionMode::default(),
        )
        .expect("Failed to create neutral child session");

    let child_session = manager
        .get_session(&child_session_id)
        .expect("Failed to load child session")
        .expect("Child session should exist");

    assert_eq!(child_session.session_type, SessionType::Chat);
    assert_eq!(child_session.workspace_mode, WorkspaceMode::Neutral);
    assert_eq!(child_session.working_dir, None);
    assert_eq!(child_session.project_dir, None);
    assert_eq!(child_session.target_branch, None);
    assert_eq!(
        child_session.parent_session_id.as_deref(),
        Some(parent_session_id.as_str())
    );
}

#[test]
fn test_create_linked_session_preserves_parent_user_id() {
    let (db, _temp) = create_test_db();
    let user_id = "user-123";
    create_test_user(&db, user_id);

    let manager = SessionManager::new(db);
    let parent_session_id = manager
        .create_session_for_user(
            "Parent Session",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user_id),
        )
        .expect("Failed to create parent session");
    let pinch_ctx = PinchContext::from_input(PinchContextInput {
        source_session_id: parent_session_id.clone(),
        source_session_title: "Parent Session".to_string(),
        summary: SummarizationResult::default(),
        ranked_files: vec![],
        preservation_hints: None,
        direction: None,
        project_context: None,
        key_file_contents: vec![],
        active_plan: None,
    });

    let child_session_id = manager
        .create_linked_session(
            "Child Session",
            &parent_session_id,
            &pinch_ctx,
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            None,
            PermissionMode::default(),
        )
        .expect("Failed to create child session");

    let child_session = manager
        .get_session(&child_session_id)
        .expect("Failed to load child session")
        .expect("Child session should exist");

    assert_eq!(child_session.user_id.as_deref(), Some(user_id));
    assert_eq!(
        child_session.parent_session_id.as_deref(),
        Some(parent_session_id.as_str())
    );
}

#[test]
fn test_get_session() {
    // Test retrieving a session by ID
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    // Retrieve the session
    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session");

    assert!(session.is_some(), "Session should exist");
    let session = session.unwrap();
    assert_eq!(session.id, session_id);
    assert_eq!(session.title, "Test Session");
    assert_eq!(session.working_dir, Some("/tmp".to_string()));
    assert_eq!(session.session_type, SessionType::Code);
    assert_eq!(session.target_branch, None);
    assert_eq!(session.permission_mode, PermissionMode::default());
}

#[test]
fn test_create_session_with_explicit_type_and_workspace() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session_for_user_with_config(
            "Chat Session",
            Some("gpt-5"),
            None,
            None,
            WorkspaceMode::Neutral,
            None,
            None,
            SessionType::Chat,
        )
        .expect("Failed to create session");

    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session")
        .expect("Session should exist");

    assert_eq!(session.session_type, SessionType::Chat);
    assert_eq!(session.workspace_mode, WorkspaceMode::Neutral);
    assert_eq!(session.working_dir, None);
    assert_eq!(session.project_dir, None);
}

#[test]
fn test_update_session_title() {
    // Test updating session title
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session("Original Title", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    // Update the title
    manager
        .update_session_title(&session_id, "Updated Title")
        .expect("Failed to update title");

    // Verify the update
    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session")
        .expect("Session should exist");

    assert_eq!(session.title, "Updated Title");
}

#[test]
fn test_update_session_working_dir() {
    // Test updating session working directory
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session("Session", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    manager
        .update_session_working_dir(&session_id, Some("/home/user/project"))
        .expect("Failed to update working dir");

    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session")
        .expect("Session should exist");
    assert_eq!(session.working_dir, Some("/home/user/project".to_string()));
}

#[test]
fn test_update_session_target_branch() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session("Session", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    manager
        .update_session_target_branch(&session_id, Some("feature/test"))
        .expect("Failed to update target branch");

    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session")
        .expect("Session should exist");
    assert_eq!(session.target_branch.as_deref(), Some("feature/test"));

    manager
        .update_session_target_branch(&session_id, None)
        .expect("Failed to clear target branch");

    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session")
        .expect("Session should exist");
    assert_eq!(session.target_branch, None);
}

#[test]
fn test_update_session_workspace_neutral_clears_working_dir() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session_for_user_with_config(
            "Workspace Session",
            Some("gpt-5"),
            Some("/tmp/demo-app"),
            Some("/tmp/demo-app"),
            WorkspaceMode::Created,
            None,
            None,
            SessionType::Code,
        )
        .expect("Failed to create session");

    manager
        .update_session_workspace(&session_id, None, WorkspaceMode::Neutral)
        .expect("Failed to clear workspace");

    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session")
        .expect("Session should exist");
    assert_eq!(session.workspace_mode, WorkspaceMode::Neutral);
    assert_eq!(session.project_dir, None);
    assert_eq!(session.working_dir, None);
}

#[test]
fn test_delete_session() {
    // Test deleting a session
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    // Delete the session
    manager
        .delete_session(&session_id)
        .expect("Failed to delete session");

    // Session should be gone
    let session = manager
        .get_session(&session_id)
        .expect("Failed to get session");

    assert!(session.is_none(), "Session should be deleted");
}
