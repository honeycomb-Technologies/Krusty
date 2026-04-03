use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::team::{TeamManager, TeammateConfig, TeammateRole};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct TeammateTool {
    team_manager: Arc<TeamManager>,
}

impl TeammateTool {
    pub fn new(team_manager: Arc<TeamManager>) -> Self {
        Self { team_manager }
    }
}

#[derive(Deserialize)]
struct Params {
    action: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    _task_ids: Option<Vec<String>>,
}

const VALID_ACTIONS: &[&str] = &["spawn", "cancel", "list"];
const VALID_ROLES: &[&str] = &["builder", "reviewer", "tester"];

#[async_trait]
impl Tool for TeammateTool {
    fn name(&self) -> &str {
        "teammate"
    }

    fn description(&self) -> &str {
        "Manage teammate agents: spawn workers (builder, reviewer, tester), cancel stalled teammates, or list active teammates."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Manage your team of specialized agent workers.

Actions:
- "spawn": Create a new teammate. Requires name and role. The teammate starts idle and can be assigned tasks.
  - builder: Can read and write files. Use for implementation work.
  - reviewer: Read-only with bash. Use for code review and analysis.
  - tester: Read-only with bash. Use for running tests and validation.
- "cancel": Stop a teammate by name. Use when a teammate is stalled or no longer needed.
- "list": Show all active teammates and their current status.

Naming: Use descriptive names that indicate what the teammate is working on (e.g., "auth-builder", "api-tester").

Always create tasks (via CreateTask) before spawning teammates to work on them."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "cancel", "list"],
                    "description": "Action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Teammate name (required for spawn/cancel)"
                },
                "role": {
                    "type": "string",
                    "enum": ["builder", "reviewer", "tester"],
                    "description": "Teammate role (required for spawn)"
                },
                "task_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs to assign to the teammate (optional, for spawn)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if !VALID_ACTIONS.contains(&params.action.as_str()) {
            return ToolResult::invalid_parameters(format!(
                "Invalid action '{}'. Must be one of: spawn, cancel, list",
                params.action
            ));
        }

        match params.action.as_str() {
            "spawn" => self.execute_spawn(params).await,
            "cancel" => self.execute_cancel(params).await,
            "list" => self.execute_list().await,
            _ => unreachable!(),
        }
    }
}

impl TeammateTool {
    async fn execute_spawn(&self, params: Params) -> ToolResult {
        let name = match params.name {
            Some(n) if !n.is_empty() => n,
            _ => return ToolResult::invalid_parameters("name is required for spawn"),
        };

        let role_str = match params.role {
            Some(r) if VALID_ROLES.contains(&r.as_str()) => r,
            Some(r) => {
                return ToolResult::invalid_parameters(format!(
                    "Invalid role '{}'. Must be one of: builder, reviewer, tester",
                    r
                ))
            }
            None => return ToolResult::invalid_parameters("role is required for spawn"),
        };

        let role = match role_str.as_str() {
            "builder" => TeammateRole::Builder,
            "reviewer" => TeammateRole::Reviewer,
            "tester" => TeammateRole::Tester,
            _ => unreachable!(),
        };

        let config = TeammateConfig {
            name: name.clone(),
            role,
            max_turns: None,
        };

        match self.team_manager.spawn_teammate(config).await {
            Ok(data) => ToolResult::success_data(json!({
                "name": data,
                "role": role_str,
                "status": "spawned",
            })),
            Err(e) => ToolResult::error(format!("Failed to spawn teammate: {e}")),
        }
    }

    async fn execute_cancel(&self, params: Params) -> ToolResult {
        let name = match params.name {
            Some(n) if !n.is_empty() => n,
            _ => return ToolResult::invalid_parameters("name is required for cancel"),
        };

        match self.team_manager.cancel_teammate(&name).await {
            Ok(()) => ToolResult::success_data(json!({
                "name": name,
                "status": "cancelled",
            })),
            Err(e) => ToolResult::error(format!("Failed to cancel teammate: {e}")),
        }
    }

    async fn execute_list(&self) -> ToolResult {
        let teammates = self.team_manager.list_teammates().await;
        let data = teammates
            .into_iter()
            .map(|(name, status, progress)| {
                json!({
                    "name": name,
                    "status": status.label(),
                    "details": status.to_string(),
                    "progress": progress,
                })
            })
            .collect::<Vec<_>>();
        ToolResult::success_data(json!({
            "total": data.len(),
            "teammates": data,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::client::{AiClient, AiClientConfig};
    use crate::tools::registry::ToolRegistry;
    use std::path::PathBuf;

    fn make_tool() -> TeammateTool {
        let ai_client = Arc::new(AiClient::new(
            AiClientConfig::default(),
            "test-key".to_string(),
        ));
        let tool_registry = Arc::new(ToolRegistry::new());
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("krusty.db");
        TeammateTool::new(Arc::new(TeamManager::new(
            ai_client,
            tool_registry,
            PathBuf::from("/tmp"),
            None,
            "test-session".to_string(),
            db_path,
        )))
    }

    fn default_ctx() -> ToolContext {
        ToolContext::default()
    }

    #[tokio::test]
    async fn spawn_requires_name() {
        let tool = make_tool();
        let result = tool
            .execute(
                json!({ "action": "spawn", "role": "builder" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("name"));
    }

    #[tokio::test]
    async fn spawn_requires_role() {
        let tool = make_tool();
        let result = tool
            .execute(
                json!({ "action": "spawn", "name": "test-builder" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("role"));
    }

    #[tokio::test]
    async fn spawn_rejects_invalid_role() {
        let tool = make_tool();
        let result = tool
            .execute(
                json!({ "action": "spawn", "name": "test", "role": "manager" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("Invalid role"));
    }

    #[tokio::test]
    async fn spawn_succeeds() {
        let tool = make_tool();
        let result = tool
            .execute(
                json!({ "action": "spawn", "name": "auth-builder", "role": "builder" }),
                &default_ctx(),
            )
            .await;
        assert!(!result.is_error, "{}", result.output);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["name"], "auth-builder");
        assert_eq!(parsed["data"]["role"], "builder");
    }

    #[tokio::test]
    async fn spawn_duplicate_name_rejected() {
        let tool = make_tool();
        tool.execute(
            json!({ "action": "spawn", "name": "worker", "role": "tester" }),
            &default_ctx(),
        )
        .await;
        let result = tool
            .execute(
                json!({ "action": "spawn", "name": "worker", "role": "tester" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("already exists"));
    }

    #[tokio::test]
    async fn cancel_requires_name() {
        let tool = make_tool();
        let result = tool
            .execute(json!({ "action": "cancel" }), &default_ctx())
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn cancel_nonexistent_fails() {
        let tool = make_tool();
        let result = tool
            .execute(
                json!({ "action": "cancel", "name": "ghost" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn list_empty() {
        let tool = make_tool();
        let result = tool
            .execute(json!({ "action": "list" }), &default_ctx())
            .await;
        assert!(!result.is_error, "{}", result.output);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["total"], 0);
    }

    #[tokio::test]
    async fn list_shows_spawned_teammates() {
        let tool = make_tool();
        tool.execute(
            json!({ "action": "spawn", "name": "a", "role": "builder" }),
            &default_ctx(),
        )
        .await;
        tool.execute(
            json!({ "action": "spawn", "name": "b", "role": "tester" }),
            &default_ctx(),
        )
        .await;

        let result = tool
            .execute(json!({ "action": "list" }), &default_ctx())
            .await;
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["total"], 2);
    }

    #[tokio::test]
    async fn invalid_action_rejected() {
        let tool = make_tool();
        let result = tool
            .execute(json!({ "action": "restart" }), &default_ctx())
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("Invalid action"));
    }

    #[tokio::test]
    async fn missing_action_field() {
        let tool = make_tool();
        let result = tool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }
}
