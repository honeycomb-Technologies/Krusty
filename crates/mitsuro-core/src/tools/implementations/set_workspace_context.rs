//! Set Workspace Context tool - explicit neutral/project transitions.
//!
//! This allows the agent to promote a neutral session into project mode after
//! the user selects or creates a concrete project directory.

use async_trait::async_trait;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{Database, SessionManager, WorkspaceMode};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct SetWorkspaceContextTool;

#[derive(Deserialize)]
struct Params {
    mode: WorkspaceMode,
    #[serde(default)]
    project_dir: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[async_trait]
impl Tool for SetWorkspaceContextTool {
    fn name(&self) -> &str {
        "set_workspace_context"
    }

    fn description(&self) -> &str {
        "Update the session workspace state. Use this after the user explicitly selects a folder, or after you create a new project directory and want future turns to treat it as the active project."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Modes: "neutral" means no project is active (general conversation). "selected" means the user picked an existing directory. "created" means you just scaffolded a new project.

Use this when transitioning from general chat to project-focused work, for example after the user says "let's work on /home/user/myapp" or after you run `mkdir` plus `cargo init`. Provide the absolute project_dir path for selected/created modes.

Workspace context orients future work; it is not a default filesystem permission boundary. If this tool rejects a path, treat that as an explicit runtime access policy.

Do not switch to neutral unless the user explicitly wants to leave project context."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["neutral", "selected", "created"],
                    "description": "Target workspace mode"
                },
                "project_dir": {
                    "type": ["string", "null"],
                    "description": "Explicit project directory. Required for selected/created; must be null for neutral."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional short explanation for logs and result metadata"
                }
            },
            "required": ["mode"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let Some(session_id) = ctx.session_id.as_deref() else {
            return ToolResult::error(
                "set_workspace_context requires a session-bound tool context",
            );
        };
        let Some(db_path) = ctx.db_path.as_deref() else {
            return ToolResult::error("set_workspace_context requires database access");
        };

        let normalized_project_dir = params
            .project_dir
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());

        let validated_project_dir = match params.mode {
            WorkspaceMode::Neutral => {
                if normalized_project_dir.is_some() {
                    return ToolResult::error(
                        "workspace mode 'neutral' cannot include a project_dir",
                    );
                }
                None
            }
            WorkspaceMode::Selected | WorkspaceMode::Created => {
                let Some(project_dir) = normalized_project_dir else {
                    return ToolResult::error(
                        "workspace modes 'selected' and 'created' require a project_dir",
                    );
                };

                match validate_project_dir(project_dir, ctx) {
                    Ok(project_dir) => Some(project_dir),
                    Err(err) => return ToolResult::error(err),
                }
            }
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(err) => {
                return ToolResult::error(format!(
                    "failed to open session database for workspace update: {err}"
                ));
            }
        };
        let manager = SessionManager::new(db);
        if let Err(err) = manager.update_session_workspace(
            session_id,
            validated_project_dir.as_deref(),
            params.mode,
        ) {
            return ToolResult::error(format!("failed to update workspace context: {err}"));
        }

        ToolResult::success_data(json!({
            "session_id": session_id,
            "workspace_mode": params.mode,
            "project_dir": validated_project_dir,
            "reason": params.reason,
        }))
    }
}

fn validate_project_dir(project_dir: &str, ctx: &ToolContext) -> Result<String, String> {
    let path = Path::new(project_dir);
    if !path.is_absolute() {
        return Err("workspace project_dir must be an absolute path".to_string());
    }

    let canonical_project_dir = path
        .canonicalize()
        .map_err(|err| format!("workspace project_dir must exist and be accessible: {err}"))?;

    if !canonical_project_dir.is_dir() {
        return Err("workspace project_dir must be an existing directory".to_string());
    }

    if let Some(access_root) = ctx.filesystem_access_root() {
        let canonical_access_root = access_root
            .canonicalize()
            .map_err(|err| format!("filesystem access root is not accessible: {err}"))?;
        if !canonical_project_dir.starts_with(&canonical_access_root) {
            return Err(format!(
                "Access denied: workspace project_dir '{}' is outside the configured filesystem access root",
                project_dir
            ));
        }
    }

    path_to_string(canonical_project_dir)
}

fn path_to_string(path: PathBuf) -> Result<String, String> {
    path.into_os_string()
        .into_string()
        .map_err(|_| "workspace project_dir must be valid UTF-8".to_string())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::tools::registry::ToolContext;

    fn create_test_session() -> (TempDir, String) {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let db_path = temp_dir.path().join("mitsuro.db");
        let db = Database::new(&db_path).expect("database should create");
        let manager = SessionManager::new(db);
        let session_id = manager
            .create_session("Neutral Session", Some("test-model"), None)
            .expect("session should create");
        (temp_dir, session_id)
    }

    #[tokio::test]
    async fn promotes_session_into_created_workspace_mode() {
        let (temp_dir, session_id) = create_test_session();
        let db_path = temp_dir.path().join("mitsuro.db");
        let workspace = temp_dir.path().join("workspace");
        let project_dir = workspace.join("demo-app");
        std::fs::create_dir_all(&project_dir).expect("project dir should create");
        let expected_project_dir = project_dir
            .canonicalize()
            .expect("project dir should canonicalize")
            .to_string_lossy()
            .to_string();
        let ctx = ToolContext::default()
            .with_session_metadata(session_id.clone(), db_path.clone())
            .with_sandbox(workspace);

        let result = SetWorkspaceContextTool
            .execute(
                json!({
                    "mode": "created",
                    "project_dir": project_dir
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error, "{}", result.output);

        let db = Database::new(&db_path).expect("database should open");
        let manager = SessionManager::new(db);
        let session = manager
            .get_session(&session_id)
            .expect("session should load")
            .expect("session should exist");
        assert_eq!(
            session.project_dir.as_deref(),
            Some(expected_project_dir.as_str())
        );
        assert_eq!(
            session.working_dir.as_deref(),
            Some(expected_project_dir.as_str())
        );
        assert_eq!(session.workspace_mode, WorkspaceMode::Created);
    }

    #[tokio::test]
    async fn unrestricted_context_allows_selected_project_outside_working_dir() {
        let (temp_dir, session_id) = create_test_session();
        let db_path = temp_dir.path().join("mitsuro.db");
        let working_dir = temp_dir.path().join("workspace");
        let sibling_repo = temp_dir.path().join("sibling-repo");
        std::fs::create_dir_all(&working_dir).expect("workspace should create");
        std::fs::create_dir_all(&sibling_repo).expect("sibling repo should create");
        let expected_project_dir = sibling_repo
            .canonicalize()
            .expect("sibling repo should canonicalize")
            .to_string_lossy()
            .to_string();
        let ctx = ToolContext {
            working_dir,
            ..Default::default()
        }
        .with_session_metadata(session_id.clone(), db_path.clone());

        let result = SetWorkspaceContextTool
            .execute(
                json!({
                    "mode": "selected",
                    "project_dir": sibling_repo
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error, "{}", result.output);

        let db = Database::new(&db_path).expect("database should open");
        let manager = SessionManager::new(db);
        let session = manager
            .get_session(&session_id)
            .expect("session should load")
            .expect("session should exist");
        assert_eq!(
            session.project_dir.as_deref(),
            Some(expected_project_dir.as_str())
        );
        assert_eq!(
            session.working_dir.as_deref(),
            Some(expected_project_dir.as_str())
        );
        assert_eq!(session.workspace_mode, WorkspaceMode::Selected);
    }

    #[tokio::test]
    async fn rejects_project_dir_outside_configured_filesystem_access_root() {
        let (temp_dir, session_id) = create_test_session();
        let db_path = temp_dir.path().join("mitsuro.db");
        let workspace = temp_dir.path().join("workspace");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&workspace).expect("workspace should create");
        std::fs::create_dir_all(&outside).expect("outside dir should create");
        let ctx = ToolContext::default()
            .with_session_metadata(session_id.clone(), db_path.clone())
            .with_sandbox(workspace);

        let result = SetWorkspaceContextTool
            .execute(
                json!({
                    "mode": "selected",
                    "project_dir": outside
                }),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("filesystem access root"));

        let db = Database::new(&db_path).expect("database should open");
        let manager = SessionManager::new(db);
        let session = manager
            .get_session(&session_id)
            .expect("session should load")
            .expect("session should exist");
        assert_eq!(session.project_dir, None);
        assert_eq!(session.working_dir, None);
        assert_eq!(session.workspace_mode, WorkspaceMode::Neutral);
    }

    #[tokio::test]
    async fn rejects_relative_project_dir() {
        let (temp_dir, session_id) = create_test_session();
        let db_path = temp_dir.path().join("mitsuro.db");
        let ctx = ToolContext::default().with_session_metadata(session_id, db_path);

        let result = SetWorkspaceContextTool
            .execute(
                json!({
                    "mode": "selected",
                    "project_dir": "relative-project"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("absolute path"));
    }

    #[tokio::test]
    async fn rejects_invalid_neutral_payload() {
        let (_temp_dir, session_id) = create_test_session();
        let ctx = ToolContext::default()
            .with_session_metadata(session_id, std::path::PathBuf::from("/tmp/missing.db"));

        let result = SetWorkspaceContextTool
            .execute(
                json!({
                    "mode": "neutral",
                    "project_dir": "/tmp/demo-app"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("cannot include a project_dir"));
    }
}
