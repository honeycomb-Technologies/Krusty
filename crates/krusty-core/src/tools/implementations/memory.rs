//! Memory tool — persistent cross-session knowledge retention.
//!
//! Lets the agent save, update, delete, and list persistent memories
//! that survive across sessions (user preferences, feedback, project
//! context, external references).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{Database, MemoryStore, MemoryType};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct MemoryTool;

#[derive(Deserialize)]
struct Params {
    action: String,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    memory_id: Option<String>,
}

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Save, update, or delete persistent memories that survive across sessions. Use to remember user preferences, feedback, project context, and external references."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Save memories when you learn something worth retaining across sessions:
- User role, expertise, or preferences -> memory_type "user"
- Corrections to your approach or confirmed conventions -> memory_type "feedback"
- Project decisions, deadlines, ongoing work context -> memory_type "project"
- Pointers to external systems (issue trackers, dashboards) -> memory_type "reference"

Do NOT save: code patterns (derivable from code), git history, debugging solutions, or current conversation context.

Actions:
- "save": Create a new memory (requires memory_type, title, content)
- "update": Update an existing memory (requires memory_id, plus title and/or content)
- "delete": Delete a memory (requires memory_id)
- "list": List all memories (optionally filtered by memory_type)"#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "update", "delete", "list"],
                    "description": "The operation to perform"
                },
                "memory_type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"],
                    "description": "Category of memory (required for save, optional filter for list)"
                },
                "title": {
                    "type": "string",
                    "description": "Short label for the memory (required for save, optional for update)"
                },
                "content": {
                    "type": "string",
                    "description": "Memory content (required for save, optional for update)"
                },
                "memory_id": {
                    "type": "string",
                    "description": "Memory ID (required for update and delete)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let Some(db_path) = ctx.db_path.as_deref() else {
            return ToolResult::error("memory tool requires database access");
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(err) => {
                return ToolResult::error(format!("failed to open database: {err}"));
            }
        };

        let store = MemoryStore::new(db);
        let project_dir = ctx
            .project_dir
            .as_deref()
            .map(|p| p.to_string_lossy().to_string());
        let user_id = ctx.user_id.as_deref();

        match params.action.as_str() {
            "save" => execute_save(&store, &params, project_dir.as_deref(), user_id),
            "update" => execute_update(&store, &params),
            "delete" => execute_delete(&store, &params),
            "list" => execute_list(&store, &params, project_dir.as_deref(), user_id),
            other => ToolResult::invalid_parameters(format!(
                "Unknown action '{}'. Valid actions: save, update, delete, list",
                other
            )),
        }
    }
}

fn execute_save(
    store: &MemoryStore,
    params: &Params,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> ToolResult {
    let Some(type_str) = params.memory_type.as_deref() else {
        return ToolResult::invalid_parameters("'save' requires memory_type");
    };
    let Ok(memory_type) = type_str.parse::<MemoryType>() else {
        return ToolResult::invalid_parameters(format!(
            "Invalid memory_type '{}'. Valid types: user, feedback, project, reference",
            type_str
        ));
    };
    let Some(title) = params.title.as_deref().filter(|t| !t.is_empty()) else {
        return ToolResult::invalid_parameters("'save' requires a non-empty title");
    };
    let Some(content) = params.content.as_deref().filter(|c| !c.is_empty()) else {
        return ToolResult::invalid_parameters("'save' requires non-empty content");
    };

    match store.save(memory_type, title, content, project_dir, user_id) {
        Ok(memory) => ToolResult::success_data(json!({
            "id": memory.id,
            "memory_type": memory.memory_type.as_str(),
            "title": memory.title,
            "content": memory.content,
            "project_dir": memory.project_dir,
            "created_at": memory.created_at,
        })),
        Err(err) => ToolResult::error(format!("failed to save memory: {err}")),
    }
}

fn execute_update(store: &MemoryStore, params: &Params) -> ToolResult {
    let Some(id) = params.memory_id.as_deref().filter(|i| !i.is_empty()) else {
        return ToolResult::invalid_parameters("'update' requires memory_id");
    };

    let title = params.title.as_deref().filter(|t| !t.is_empty());
    let content = params.content.as_deref().filter(|c| !c.is_empty());

    if title.is_none() && content.is_none() {
        return ToolResult::invalid_parameters(
            "'update' requires at least one of title or content",
        );
    }

    match store.update(id, title, content) {
        Ok(()) => ToolResult::success_data(json!({
            "id": id,
            "updated": true,
        })),
        Err(err) => ToolResult::error(format!("failed to update memory: {err}")),
    }
}

fn execute_delete(store: &MemoryStore, params: &Params) -> ToolResult {
    let Some(id) = params.memory_id.as_deref().filter(|i| !i.is_empty()) else {
        return ToolResult::invalid_parameters("'delete' requires memory_id");
    };

    match store.delete(id) {
        Ok(()) => ToolResult::success_data(json!({
            "id": id,
            "deleted": true,
        })),
        Err(err) => ToolResult::error(format!("failed to delete memory: {err}")),
    }
}

fn execute_list(
    store: &MemoryStore,
    params: &Params,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> ToolResult {
    let memories = if let Some(type_str) = params.memory_type.as_deref() {
        if let Ok(memory_type) = type_str.parse::<MemoryType>() {
            store.list_by_type(memory_type, project_dir, user_id)
        } else {
            return ToolResult::invalid_parameters(format!(
                "Invalid memory_type '{}'. Valid types: user, feedback, project, reference",
                type_str
            ));
        }
    } else {
        store.list(project_dir, user_id)
    };

    let entries: Vec<Value> = memories
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "memory_type": m.memory_type.as_str(),
                "title": m.title,
                "content": m.content,
                "project_dir": m.project_dir,
                "updated_at": m.updated_at,
            })
        })
        .collect();

    ToolResult::success_data(json!({
        "count": entries.len(),
        "memories": entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::ToolContext;
    use tempfile::TempDir;

    fn test_ctx() -> (ToolContext, TempDir) {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("krusty.db");
        // Ensure database exists with schema
        let _ = Database::new(&db_path).expect("database");
        let ctx = ToolContext {
            db_path: Some(db_path),
            ..Default::default()
        };
        (ctx, temp_dir)
    }

    #[tokio::test]
    async fn save_and_list_round_trip() {
        let (ctx, _tmp) = test_ctx();

        let save_result = MemoryTool
            .execute(
                json!({
                    "action": "save",
                    "memory_type": "user",
                    "title": "Preferred language",
                    "content": "Rust"
                }),
                &ctx,
            )
            .await;
        assert!(!save_result.is_error, "save failed: {}", save_result.output);

        let list_result = MemoryTool.execute(json!({ "action": "list" }), &ctx).await;
        assert!(!list_result.is_error);
        let parsed: Value = serde_json::from_str(&list_result.output).unwrap();
        assert_eq!(parsed["data"]["count"], 1);
        assert_eq!(parsed["data"]["memories"][0]["title"], "Preferred language");
    }

    #[tokio::test]
    async fn save_requires_type_title_content() {
        let (ctx, _tmp) = test_ctx();

        let result = MemoryTool
            .execute(
                json!({ "action": "save", "title": "X", "content": "Y" }),
                &ctx,
            )
            .await;
        assert!(result.is_error);

        let result = MemoryTool
            .execute(
                json!({ "action": "save", "memory_type": "user", "content": "Y" }),
                &ctx,
            )
            .await;
        assert!(result.is_error);

        let result = MemoryTool
            .execute(
                json!({ "action": "save", "memory_type": "user", "title": "X" }),
                &ctx,
            )
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn update_and_delete() {
        let (ctx, _tmp) = test_ctx();

        let save_result = MemoryTool
            .execute(
                json!({
                    "action": "save",
                    "memory_type": "feedback",
                    "title": "Testing",
                    "content": "Use mocks"
                }),
                &ctx,
            )
            .await;
        let parsed: Value = serde_json::from_str(&save_result.output).unwrap();
        let id = parsed["data"]["id"].as_str().unwrap().to_string();

        let update_result = MemoryTool
            .execute(
                json!({
                    "action": "update",
                    "memory_id": id,
                    "content": "Use integration tests instead of mocks"
                }),
                &ctx,
            )
            .await;
        assert!(!update_result.is_error);

        let delete_result = MemoryTool
            .execute(json!({ "action": "delete", "memory_id": id }), &ctx)
            .await;
        assert!(!delete_result.is_error);

        let list_result = MemoryTool.execute(json!({ "action": "list" }), &ctx).await;
        let parsed: Value = serde_json::from_str(&list_result.output).unwrap();
        assert_eq!(parsed["data"]["count"], 0);
    }
}
