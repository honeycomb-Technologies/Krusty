//! Memory tool — persistent cross-session knowledge retention.
//!
//! Lets the agent save, update, delete, and list persistent memories
//! that survive across sessions (user preferences, feedback, project
//! context, external references).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{
    is_compaction_flush_memory, is_current_snapshot, is_current_snapshot_title,
    refresh_current_snapshot, AgentMemory, CanonicalMemoryInput, Database, HiveMemoryReader,
    HiveWorkerStore, MemoryAclScope, MemoryNamespace, MemorySource, MemoryStore, MemoryType,
};
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
    #[serde(default)]
    include_content: Option<bool>,
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

Do NOT save: code patterns (derivable from code), git history, debugging solutions, compaction summaries, or current conversation context.

Do NOT call the memory tool for generic/non-project questions unless the user explicitly asks about stored memory.

Actions:
- "save": Create a new memory (requires memory_type, title, content)
- "update": Update an existing memory (requires memory_id, plus title and/or content)
- "delete": Delete a memory (requires memory_id)
- "list": List memory previews (optionally filtered by memory_type; full content is omitted unless include_content is true and the user asked for stored memory)"#,
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
                },
                "include_content": {
                    "type": "boolean",
                    "description": "For list only: include full memory content. Defaults to false; use only when the user explicitly asks to inspect stored memory."
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
        let hive_scope = resolve_hive_tool_scope(ctx, db_path);

        match params.action.as_str() {
            "save" => execute_save(
                &store,
                db_path,
                &params,
                project_dir.as_deref(),
                user_id,
                hive_scope.as_ref(),
                ctx.session_id.as_deref(),
            ),
            "update" => execute_update(&store, db_path, &params, user_id, hive_scope.as_ref()),
            "delete" => execute_delete(&store, db_path, &params, user_id, hive_scope.as_ref()),
            "list" => execute_list(
                &store,
                &params,
                project_dir.as_deref(),
                user_id,
                hive_scope.as_ref(),
            ),
            other => ToolResult::invalid_parameters(format!(
                "Unknown action '{}'. Valid actions: save, update, delete, list",
                other
            )),
        }
    }
}

struct HiveToolScope {
    worker_namespace_id: Option<String>,
    conversation_id: Option<String>,
    group_id: Option<String>,
}

impl HiveToolScope {
    fn reader<'a>(
        &'a self,
        user_id: Option<&'a str>,
        project_dir: Option<&'a str>,
    ) -> HiveMemoryReader<'a> {
        HiveMemoryReader {
            user_id,
            project_dir,
            worker_namespace_id: self.worker_namespace_id.as_deref(),
            conversation_id: self.conversation_id.as_deref(),
            group_id: self.group_id.as_deref(),
        }
    }
}

fn resolve_hive_tool_scope(ctx: &ToolContext, db_path: &std::path::Path) -> Option<HiveToolScope> {
    if let Some(run) = ctx.hive_group_run.as_ref() {
        let worker_namespace_id = Database::new(db_path).ok().and_then(|db| {
            HiveWorkerStore::new(db)
                .get(&run.worker_id)
                .ok()
                .flatten()
                .filter(|worker| worker.user_id.as_deref() == ctx.user_id.as_deref())
                .map(|worker| worker.memory_namespace_id)
        });
        return Some(HiveToolScope {
            worker_namespace_id,
            conversation_id: ctx.session_id.clone(),
            group_id: Some(run.group_id.clone()),
        });
    }

    let session_id = ctx.session_id.as_deref()?;
    if let Some(worker) = Database::new(db_path).ok().and_then(|db| {
        HiveWorkerStore::new(db)
            .get_by_dm_session(session_id)
            .ok()
            .flatten()
            .filter(|worker| worker.user_id.as_deref() == ctx.user_id.as_deref())
    }) {
        return Some(HiveToolScope {
            worker_namespace_id: Some(worker.memory_namespace_id),
            conversation_id: Some(session_id.to_string()),
            group_id: None,
        });
    }

    let session_type: Option<String> = Database::new(db_path).ok().and_then(|db| {
        db.conn()
            .query_row(
                "SELECT session_type FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .ok()
    });
    if session_type.as_deref() == Some("hive") {
        return Some(HiveToolScope {
            worker_namespace_id: None,
            conversation_id: Some(session_id.to_string()),
            group_id: None,
        });
    }
    None
}

fn tool_canonical_key(title: &str) -> String {
    let slug = title
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("tool.memory:{slug}")
}

fn execute_save(
    store: &MemoryStore,
    db_path: &std::path::Path,
    params: &Params,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    hive_scope: Option<&HiveToolScope>,
    session_id: Option<&str>,
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
    if is_current_snapshot_title(title) {
        return ToolResult::invalid_parameters("Current Snapshot is managed automatically");
    }
    let Some(content) = params.content.as_deref().filter(|c| !c.is_empty()) else {
        return ToolResult::invalid_parameters("'save' requires non-empty content");
    };

    let saved = if let Some(scope) = hive_scope {
        let mut input =
            CanonicalMemoryInput::new(memory_type, tool_canonical_key(title), title, content);
        input.project_dir = project_dir.map(str::to_string);
        input.user_id = user_id.map(str::to_string);
        input.source = MemorySource::Tool;
        input.source_session_id = session_id.map(str::to_string);
        if let Some(namespace_id) = scope.worker_namespace_id.as_deref() {
            input.namespace = MemoryNamespace::Crew;
            input.namespace_id = Some(namespace_id.to_string());
            input.acl_scope = MemoryAclScope::Worker;
        } else {
            input.namespace = MemoryNamespace::Hive;
            input.acl_scope = MemoryAclScope::Owner;
        }
        store.save_canonical(&input)
    } else {
        store.save(memory_type, title, content, project_dir, user_id)
    };

    match saved {
        Ok(memory) => {
            if let Err(err) = refresh_current_snapshot(db_path, project_dir, user_id) {
                return ToolResult::error(format!(
                    "memory saved but snapshot refresh failed: {err}"
                ));
            }

            ToolResult::success_data(json!({
                "id": memory.id,
                "memory_type": memory.memory_type.as_str(),
                "title": memory.title,
                "content": memory.content,
                "project_dir": memory.project_dir,
                "created_at": memory.created_at,
            }))
        }
        Err(err) => ToolResult::error(format!("failed to save memory: {err}")),
    }
}

fn execute_update(
    store: &MemoryStore,
    db_path: &std::path::Path,
    params: &Params,
    user_id: Option<&str>,
    hive_scope: Option<&HiveToolScope>,
) -> ToolResult {
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

    let Some(existing) = store.get(id).ok().flatten() else {
        return ToolResult::error(format!(
            "failed to update memory: memory '{}' not found",
            id
        ));
    };
    if is_current_snapshot(&existing) {
        return ToolResult::invalid_parameters("Current Snapshot is managed automatically");
    }
    if !can_mutate_memory(&existing, user_id, hive_scope) {
        return ToolResult::error(format!(
            "failed to update memory: memory '{}' not found",
            id
        ));
    }
    if title.is_some_and(is_current_snapshot_title) {
        return ToolResult::invalid_parameters("Current Snapshot is managed automatically");
    }

    match store.update(id, title, content) {
        Ok(()) => {
            if let Err(err) = refresh_current_snapshot(
                db_path,
                existing.project_dir.as_deref(),
                existing.user_id.as_deref(),
            ) {
                return ToolResult::error(format!(
                    "memory updated but snapshot refresh failed: {err}"
                ));
            }

            ToolResult::success_data(json!({
                "id": id,
                "updated": true,
            }))
        }
        Err(err) => ToolResult::error(format!("failed to update memory: {err}")),
    }
}

fn execute_delete(
    store: &MemoryStore,
    db_path: &std::path::Path,
    params: &Params,
    user_id: Option<&str>,
    hive_scope: Option<&HiveToolScope>,
) -> ToolResult {
    let Some(id) = params.memory_id.as_deref().filter(|i| !i.is_empty()) else {
        return ToolResult::invalid_parameters("'delete' requires memory_id");
    };

    let Some(existing) = store.get(id).ok().flatten() else {
        return ToolResult::error(format!(
            "failed to delete memory: memory '{}' not found",
            id
        ));
    };
    if is_current_snapshot(&existing) {
        return ToolResult::invalid_parameters("Current Snapshot is managed automatically");
    }
    if !can_mutate_memory(&existing, user_id, hive_scope) {
        return ToolResult::error(format!(
            "failed to delete memory: memory '{}' not found",
            id
        ));
    }

    match store.delete(id) {
        Ok(()) => {
            if let Err(err) = refresh_current_snapshot(
                db_path,
                existing.project_dir.as_deref(),
                existing.user_id.as_deref(),
            ) {
                return ToolResult::error(format!(
                    "memory deleted but snapshot refresh failed: {err}"
                ));
            }

            ToolResult::success_data(json!({
                "id": id,
                "deleted": true,
            }))
        }
        Err(err) => ToolResult::error(format!("failed to delete memory: {err}")),
    }
}

fn can_mutate_memory(
    memory: &crate::storage::AgentMemory,
    user_id: Option<&str>,
    hive_scope: Option<&HiveToolScope>,
) -> bool {
    let owner_matches = match user_id {
        Some(uid) => memory.user_id.as_deref() == Some(uid),
        None => memory.user_id.is_none(),
    };
    if !owner_matches {
        return false;
    }
    match hive_scope {
        Some(scope) => MemoryStore::visible_to_hive_reader(memory, &scope.reader(user_id, None)),
        None => true,
    }
}

const MEMORY_LIST_PREVIEW_CHARS: usize = 280;

fn execute_list(
    store: &MemoryStore,
    params: &Params,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    hive_scope: Option<&HiveToolScope>,
) -> ToolResult {
    let type_filter = if let Some(type_str) = params.memory_type.as_deref() {
        match type_str.parse::<MemoryType>() {
            Ok(memory_type) => Some(memory_type),
            Err(_) => {
                return ToolResult::invalid_parameters(format!(
                    "Invalid memory_type '{}'. Valid types: user, feedback, project, reference",
                    type_str
                ))
            }
        }
    } else {
        None
    };
    let memories = if let Some(scope) = hive_scope {
        let mut memories = store.list_for_hive_reader(&scope.reader(user_id, project_dir));
        if let Some(memory_type) = type_filter {
            memories.retain(|memory| memory.memory_type == memory_type);
        }
        memories
    } else if let Some(memory_type) = type_filter {
        store.list_by_type(memory_type, project_dir, user_id)
    } else {
        store.list(project_dir, user_id)
    };

    let include_content = params.include_content.unwrap_or(false);
    let entries: Vec<Value> = memories
        .iter()
        .filter(|memory| !is_current_snapshot(memory))
        .filter(|memory| !is_compaction_flush_memory(memory))
        .map(|memory| memory_list_entry(memory, include_content))
        .collect();

    ToolResult::success_data(json!({
        "count": entries.len(),
        "memories": entries,
        "content_included": include_content,
    }))
}

fn memory_list_entry(memory: &AgentMemory, include_content: bool) -> Value {
    let compact = memory
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let content_chars = compact.chars().count();
    let preview = truncate_preview(&compact, MEMORY_LIST_PREVIEW_CHARS);
    let truncated = content_chars > MEMORY_LIST_PREVIEW_CHARS;

    let mut entry = json!({
        "id": memory.id,
        "memory_type": memory.memory_type.as_str(),
        "title": memory.title,
        "content_preview": preview,
        "content_chars": content_chars,
        "truncated": truncated,
        "project_dir": memory.project_dir,
        "updated_at": memory.updated_at,
    });

    if include_content {
        entry["content"] = json!(memory.content);
    }

    entry
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::ToolContext;
    use tempfile::TempDir;

    fn test_ctx() -> (ToolContext, TempDir) {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("mitsuro.db");
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

    #[tokio::test]
    async fn update_and_delete_require_matching_user_scope() {
        let (mut alice_ctx, _tmp) = test_ctx();
        let db_path = alice_ctx.db_path.clone();
        alice_ctx.user_id = Some("alice".to_string());
        let bob_ctx = ToolContext {
            db_path,
            user_id: Some("bob".to_string()),
            ..Default::default()
        };

        let save_result = MemoryTool
            .execute(
                json!({
                    "action": "save",
                    "memory_type": "feedback",
                    "title": "Private",
                    "content": "Alice only"
                }),
                &alice_ctx,
            )
            .await;
        let parsed: Value = serde_json::from_str(&save_result.output).unwrap();
        let id = parsed["data"]["id"].as_str().unwrap().to_string();

        let update_result = MemoryTool
            .execute(
                json!({
                    "action": "update",
                    "memory_id": id,
                    "content": "Bob cannot change this"
                }),
                &bob_ctx,
            )
            .await;
        assert!(update_result.is_error);

        let delete_result = MemoryTool
            .execute(json!({ "action": "delete", "memory_id": id }), &bob_ctx)
            .await;
        assert!(delete_result.is_error);

        let list_result = MemoryTool
            .execute(json!({ "action": "list" }), &alice_ctx)
            .await;
        let parsed: Value = serde_json::from_str(&list_result.output).unwrap();
        assert_eq!(parsed["data"]["count"], 1);
        assert_eq!(
            parsed["data"]["memories"][0]["content_preview"],
            "Alice only"
        );
        assert!(parsed["data"]["memories"][0].get("content").is_none());
    }

    #[tokio::test]
    async fn list_returns_previews_by_default_and_full_content_on_request() {
        let (ctx, _tmp) = test_ctx();
        let long_content = "alpha ".repeat(120);
        let save_result = MemoryTool
            .execute(
                json!({
                    "action": "save",
                    "memory_type": "project",
                    "title": "Long Architecture",
                    "content": long_content
                }),
                &ctx,
            )
            .await;
        assert!(!save_result.is_error, "save failed: {}", save_result.output);

        let list_result = MemoryTool.execute(json!({ "action": "list" }), &ctx).await;
        let parsed: Value = serde_json::from_str(&list_result.output).unwrap();
        let memory = &parsed["data"]["memories"][0];
        assert!(memory.get("content").is_none());
        assert!(memory["content_preview"].as_str().unwrap().len() < long_content.len());
        assert_eq!(memory["truncated"], true);

        let full_result = MemoryTool
            .execute(json!({ "action": "list", "include_content": true }), &ctx)
            .await;
        let parsed: Value = serde_json::from_str(&full_result.output).unwrap();
        assert_eq!(parsed["data"]["content_included"], true);
        assert_eq!(
            parsed["data"]["memories"][0]["content"].as_str(),
            Some(long_content.as_str())
        );
    }

    #[tokio::test]
    async fn list_hides_compaction_flush_memory() {
        let (ctx, _tmp) = test_ctx();
        let db_path = ctx.db_path.as_ref().expect("db path");
        let store = MemoryStore::new(Database::new(db_path).expect("database"));
        store
            .save(
                MemoryType::Project,
                &format!("{}1", crate::storage::COMPACTION_FLUSH_TITLE_PREFIX),
                "full old transcript",
                None,
                None,
            )
            .expect("flush should save");

        let list_result = MemoryTool
            .execute(json!({ "action": "list", "include_content": true }), &ctx)
            .await;
        let parsed: Value = serde_json::from_str(&list_result.output).unwrap();
        assert_eq!(parsed["data"]["count"], 0);
    }

    #[tokio::test]
    async fn list_hides_current_snapshot_memory() {
        let (ctx, _tmp) = test_ctx();
        let db_path = ctx.db_path.as_ref().expect("db path");
        let store = MemoryStore::new(Database::new(db_path).expect("database"));
        store
            .save(
                MemoryType::Project,
                crate::storage::CURRENT_SNAPSHOT_TITLE,
                "snapshot",
                None,
                None,
            )
            .expect("snapshot should save");

        let list_result = MemoryTool.execute(json!({ "action": "list" }), &ctx).await;
        let parsed: Value = serde_json::from_str(&list_result.output).unwrap();
        assert_eq!(parsed["data"]["count"], 0);
    }

    #[tokio::test]
    async fn hive_worker_list_does_not_reveal_another_workers_private_memory() {
        let (mut researcher_ctx, _tmp) = test_ctx();
        let db_path = researcher_ctx.db_path.clone().expect("db path");
        let db = Database::new(&db_path).expect("database");
        db.conn()
            .execute_batch(
                "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
                 VALUES ('researcher-dm', 'Researcher', '2026-08-16T00:00:00.000000Z',
                         '2026-08-16T00:00:00.000000Z', 'hive');
                 INSERT INTO sessions (id, title, created_at, updated_at, session_type)
                 VALUES ('builder-dm', 'Builder', '2026-08-16T00:00:00.000000Z',
                         '2026-08-16T00:00:00.000000Z', 'hive');",
            )
            .expect("seed hive sessions");
        let workers = crate::storage::HiveWorkerStore::new(Database::new(&db_path).expect("db"));
        let researcher = workers
            .create(&crate::storage::NewHiveWorker::new("researcher"))
            .expect("researcher");
        let builder = workers
            .create(&crate::storage::NewHiveWorker::new("builder"))
            .expect("builder");
        workers
            .bind_dm_session(&researcher.id, Some("researcher-dm"))
            .expect("bind researcher");
        workers
            .bind_dm_session(&builder.id, Some("builder-dm"))
            .expect("bind builder");

        researcher_ctx.session_id = Some("researcher-dm".into());
        let builder_ctx = ToolContext {
            db_path: Some(db_path),
            session_id: Some("builder-dm".into()),
            ..Default::default()
        };

        let save_result = MemoryTool
            .execute(
                json!({
                    "action": "save",
                    "memory_type": "project",
                    "title": "Researcher private",
                    "content": "researcher-private-marker"
                }),
                &researcher_ctx,
            )
            .await;
        assert!(!save_result.is_error, "save failed: {}", save_result.output);

        let researcher_list = MemoryTool
            .execute(
                json!({ "action": "list", "include_content": true }),
                &researcher_ctx,
            )
            .await;
        let researcher_parsed: Value = serde_json::from_str(&researcher_list.output).unwrap();
        assert_eq!(researcher_parsed["data"]["count"], 1);
        assert_eq!(
            researcher_parsed["data"]["memories"][0]["content"],
            "researcher-private-marker"
        );

        let builder_list = MemoryTool
            .execute(
                json!({ "action": "list", "include_content": true }),
                &builder_ctx,
            )
            .await;
        let builder_parsed: Value = serde_json::from_str(&builder_list.output).unwrap();
        assert_eq!(builder_parsed["data"]["count"], 0);

        let parsed: Value = serde_json::from_str(&save_result.output).unwrap();
        let id = parsed["data"]["id"].as_str().unwrap().to_string();
        let update = MemoryTool
            .execute(
                json!({
                    "action": "update",
                    "memory_id": id,
                    "content": "builder cannot change this"
                }),
                &builder_ctx,
            )
            .await;
        assert!(update.is_error);
    }
}
