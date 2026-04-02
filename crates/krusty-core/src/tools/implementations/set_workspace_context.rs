//! Set Workspace Context tool - explicit neutral/project transitions.
//!
//! This allows the agent to promote a neutral session into project mode after
//! the user selects or creates a concrete project directory.

use async_trait::async_trait;
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

Use this when transitioning from general chat to project-focused work, for example after the user says "let's work on /home/user/myapp" or after you run `mkdir` plus `cargo init`. Always provide the absolute project_dir path for selected/created modes.

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

        match params.mode {
            WorkspaceMode::Neutral if normalized_project_dir.is_some() => {
                return ToolResult::error("workspace mode 'neutral' cannot include a project_dir");
            }
            WorkspaceMode::Selected | WorkspaceMode::Created
                if normalized_project_dir.is_none() =>
            {
                return ToolResult::error(
                    "workspace modes 'selected' and 'created' require a project_dir",
                );
            }
            _ => {}
        }

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(err) => {
                return ToolResult::error(format!(
                    "failed to open session database for workspace update: {err}"
                ));
            }
        };
        let manager = SessionManager::new(db);
        let working_dir = match params.mode {
            WorkspaceMode::Neutral => None,
            WorkspaceMode::Selected | WorkspaceMode::Created => normalized_project_dir,
        };
        if let Err(err) = manager.update_session_working_dir(session_id, working_dir) {
            return ToolResult::error(format!("failed to update workspace context: {err}"));
        }

        ToolResult::success_data(json!({
            "session_id": session_id,
            "workspace_mode": params.mode,
            "project_dir": normalized_project_dir,
            "reason": params.reason,
        }))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::tools::registry::ToolContext;

    fn create_test_session() -> (TempDir, String) {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let db_path = temp_dir.path().join("krusty.db");
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
        let db_path = temp_dir.path().join("krusty.db");
        let ctx = ToolContext::default().with_session_metadata(session_id.clone(), db_path.clone());

        let result = SetWorkspaceContextTool
            .execute(
                json!({
                    "mode": "created",
                    "project_dir": "/tmp/demo-app"
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error);

        let db = Database::new(&db_path).expect("database should open");
        let manager = SessionManager::new(db);
        let session = manager
            .get_session(&session_id)
            .expect("session should load")
            .expect("session should exist");
        assert_eq!(session.working_dir.as_deref(), Some("/tmp/demo-app"));
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
