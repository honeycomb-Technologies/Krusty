use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use tempfile::TempDir;

use super::ReportTool;
use crate::storage::reports::{CreateReportInput, ReportScope};
use crate::storage::{
    Database, HiveGroupRunContext, HiveGroupStore, HiveGroupWorkerLaneStore, HiveMemoryReader,
    HiveWorkerStore, MemoryAclScope, MemoryNamespace, MemorySource, MemoryStore, NewHiveGroup,
    NewHiveGroupWorkerLane, NewHiveWorker, ReportStore, SessionManager, WorkspaceMode,
};
use crate::tools::registry::{Tool, ToolContext, ToolResult};

fn default_ctx() -> ToolContext {
    ToolContext::default()
}

fn session_ctx() -> (ToolContext, TempDir) {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("mitsuro.db");
    let session_id = SessionManager::new(Database::new(&db_path).expect("db"))
        .create_session(
            "report test",
            None,
            Some(temp.path().to_string_lossy().as_ref()),
        )
        .expect("session");
    let ctx = ToolContext::default()
        .with_workspace(Some(temp.path().to_path_buf()), WorkspaceMode::Selected)
        .with_session_metadata(session_id, db_path);
    (ctx, temp)
}

fn insert_session(db: &Database, id: &str, session_type: &str, project_dir: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (
                 id, title, created_at, updated_at, working_dir, project_dir,
                 workspace_mode, session_type
             ) VALUES (?1, ?1, ?2, ?2, ?3, ?3, 'selected', ?4)",
            rusqlite::params![id, now, project_dir, session_type],
        )
        .expect("seed report session");
}

fn report_ctx(
    db_path: &Path,
    project_dir: &Path,
    session_id: &str,
    group_run: Option<HiveGroupRunContext>,
) -> ToolContext {
    ToolContext::default()
        .with_workspace(Some(project_dir.to_path_buf()), WorkspaceMode::Selected)
        .with_session_metadata(session_id.to_string(), db_path.to_path_buf())
        .with_hive_group_run(group_run)
}

fn report_titles(result: &ToolResult) -> BTreeSet<String> {
    assert!(!result.is_error, "report list failed: {}", result.output);
    let payload: Value = serde_json::from_str(&result.output).expect("report list JSON");
    payload["data"]["reports"]
        .as_array()
        .expect("report list entries")
        .iter()
        .map(|report| report["title"].as_str().expect("report title").to_string())
        .collect()
}

#[tokio::test]
async fn report_create_requires_title() {
    let result = ReportTool
        .execute(
            json!({ "action": "create", "title": "", "content": "something" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("title"));
}

#[tokio::test]
async fn report_create_requires_content() {
    let result = ReportTool
        .execute(
            json!({ "action": "create", "title": "Test", "content": "" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("content"));
}

#[tokio::test]
async fn report_create_requires_session() {
    let result = ReportTool
        .execute(
            json!({ "action": "create", "title": "Test", "content": "Body" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("session"));
}

#[tokio::test]
async fn report_list_requires_db() {
    let result = ReportTool
        .execute(json!({ "action": "list" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_read_requires_id() {
    let result = ReportTool
        .execute(json!({ "action": "read", "report_id": "" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_read_requires_db() {
    let result = ReportTool
        .execute(
            json!({ "action": "read", "report_id": "some-id" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_create_missing_fields() {
    let result = ReportTool
        .execute(json!({ "action": "create" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_create_promotes_memory_when_requested() {
    let (ctx, _temp) = session_ctx();

    let result = ReportTool
        .execute(
            json!({
                "action": "create",
                "title": "Wake audit",
                "content": "# Wake
            The wake flow is stable.",
                "summary": "Wake flow is stable.",
                "promote_to_memory": true,
                "memory_type": "project"
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error);
    let payload: Value = serde_json::from_str(&result.output).expect("json tool result");
    let promoted = payload
        .get("data")
        .and_then(|value| value.as_object())
        .and_then(|data| data.get("promoted_memory"))
        .and_then(|value| value.as_object())
        .expect("promoted memory object");
    assert_eq!(
        promoted.get("memory_type").and_then(|value| value.as_str()),
        Some("project")
    );
    assert_eq!(
        promoted.get("title").and_then(|value| value.as_str()),
        Some("Wake audit")
    );

    let memories = MemoryStore::new(Database::new(ctx.db_path.as_ref().expect("db")).unwrap())
        .list(
            ctx.project_dir.as_ref().and_then(|path| path.to_str()),
            None,
        );
    let durable = memories
        .iter()
        .find(|memory| memory.title == "Wake audit")
        .expect("durable promoted memory");
    assert_eq!(durable.content, "Wake flow is stable.");
}

#[tokio::test]
async fn report_create_rejects_invalid_promoted_memory_type() {
    let (ctx, _temp) = session_ctx();

    let result = ReportTool
        .execute(
            json!({
                "action": "create",
                "title": "Wake audit",
                "content": "# Wake
            The wake flow is stable.",
                "promote_to_memory": true,
                "memory_type": "unknown"
            }),
            &ctx,
        )
        .await;

    assert!(result.is_error);
    assert!(result.output.contains("Invalid memory_type"));
}

#[tokio::test]
async fn report_create_rejects_a_forged_tool_context_owner() {
    let (ctx, _temp) = session_ctx();
    let forged = ctx.with_user_id("forged-owner".to_string());

    let result = ReportTool
        .execute(
            json!({
                "action": "create",
                "title": "Forged scope",
                "content": "This must not persist."
            }),
            &forged,
        )
        .await;

    assert!(result.is_error);
    assert!(result.output.contains("does not match the session owner"));
}

#[tokio::test]
async fn report_read_missing_fields() {
    let result = ReportTool
        .execute(json!({ "action": "read" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_readers_isolate_worker_dm_and_group_reports() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("report-isolation.db");
    let project = temp.path().join("project");
    let project_str = project.to_string_lossy().to_string();
    let db = Database::new(&db_path).expect("database");
    for (id, session_type) in [
        ("ordinary-code", "code"),
        ("primary-hive", "hive"),
        ("worker-a-dm", "hive"),
        ("worker-b-dm", "hive"),
        ("worker-a-group", "hive"),
    ] {
        insert_session(&db, id, session_type, &project_str);
    }

    let workers = HiveWorkerStore::new(Database::new(&db_path).unwrap());
    let worker_a = workers
        .create(&NewHiveWorker {
            dm_session_id: Some("worker-a-dm".into()),
            ..NewHiveWorker::new("worker-a")
        })
        .expect("worker A");
    let worker_b = workers
        .create(&NewHiveWorker {
            dm_session_id: Some("worker-b-dm".into()),
            ..NewHiveWorker::new("worker-b")
        })
        .expect("worker B");
    let group = HiveGroupStore::new(Database::new(&db_path).unwrap())
        .create(&NewHiveGroup {
            title: "Isolation Room".into(),
            member_worker_ids: vec![worker_a.id.clone(), worker_b.id.clone()],
            ..NewHiveGroup::default()
        })
        .expect("group");
    HiveGroupWorkerLaneStore::new(Database::new(&db_path).unwrap())
        .upsert(&NewHiveGroupWorkerLane::new(
            &group.id,
            &worker_a.id,
            "worker-a-group",
        ))
        .expect("Worker A group lane");

    let reports = ReportStore::new(Database::new(&db_path).unwrap());
    let mut report_ids = std::collections::HashMap::new();
    for (label, title, session_id) in [
        ("shared", "Isolation Canary Shared", "ordinary-code"),
        ("worker-a-dm", "Isolation Canary Worker A DM", "worker-a-dm"),
        (
            "worker-a-group",
            "Isolation Canary Worker A Group",
            "worker-a-group",
        ),
        ("worker-b-dm", "Isolation Canary Worker B DM", "worker-b-dm"),
    ] {
        let scope = match label {
            "worker-a-dm" | "worker-a-group" => ReportScope::worker_private(
                worker_a.id.clone(),
                worker_a.memory_namespace_id.clone(),
            )
            .expect("Worker A report scope"),
            "worker-b-dm" => ReportScope::worker_private(
                worker_b.id.clone(),
                worker_b.memory_namespace_id.clone(),
            )
            .expect("Worker B report scope"),
            _ => ReportScope::owner_shared(),
        };
        let id = reports
            .create_report(CreateReportInput {
                title,
                session_id,
                project_dir: Some(&project_str),
                report_root: Some(temp.path()),
                content: title,
                summary: "report_isolation_canary",
                tags: &[],
                sources: &[],
                scope,
            })
            .expect("seed report");
        report_ids.insert(label, id);
    }

    let ordinary = report_ctx(&db_path, &project, "ordinary-code", None);
    let primary_hive = report_ctx(&db_path, &project, "primary-hive", None);
    let worker_a_dm = report_ctx(&db_path, &project, "worker-a-dm", None);
    let worker_b_dm = report_ctx(&db_path, &project, "worker-b-dm", None);
    let group_run = HiveGroupRunContext {
        group_id: group.id.clone(),
        group_turn_id: "turn-a".into(),
        run_id: "run-a".into(),
        worker_id: worker_a.id.clone(),
        max_member_messages_per_turn: 2,
        context_window_messages: 24,
    };
    let worker_a_group = report_ctx(
        &db_path,
        &project,
        "worker-a-group",
        Some(group_run.clone()),
    );

    let shared = BTreeSet::from(["Isolation Canary Shared".to_string()]);
    let worker_a_visible = BTreeSet::from([
        "Isolation Canary Shared".to_string(),
        "Isolation Canary Worker A DM".to_string(),
        "Isolation Canary Worker A Group".to_string(),
    ]);
    let worker_b_visible = BTreeSet::from([
        "Isolation Canary Shared".to_string(),
        "Isolation Canary Worker B DM".to_string(),
    ]);

    for ctx in [&ordinary, &primary_hive] {
        let listed = ReportTool.execute(json!({ "action": "list" }), ctx).await;
        assert_eq!(report_titles(&listed), shared);
        let searched = ReportTool
            .execute(
                json!({ "action": "list", "query": "report_isolation_canary" }),
                ctx,
            )
            .await;
        assert_eq!(report_titles(&searched), shared);
    }
    for ctx in [&worker_a_dm, &worker_a_group] {
        let listed = ReportTool.execute(json!({ "action": "list" }), ctx).await;
        assert_eq!(report_titles(&listed), worker_a_visible);
        let searched = ReportTool
            .execute(
                json!({ "action": "list", "query": "report_isolation_canary" }),
                ctx,
            )
            .await;
        assert_eq!(report_titles(&searched), worker_a_visible);
    }
    let worker_b_list = ReportTool
        .execute(json!({ "action": "list" }), &worker_b_dm)
        .await;
    assert_eq!(report_titles(&worker_b_list), worker_b_visible);

    let promoted = ReportTool
        .execute(
            json!({
                "action": "create",
                "title": "Isolation Canary Worker A Promotion",
                "content": "This conclusion belongs only to Worker A.",
                "summary": "Worker A private conclusion.",
                "promote_to_memory": true,
                "memory_type": "reference"
            }),
            &worker_a_dm,
        )
        .await;
    assert!(!promoted.is_error, "promotion failed: {}", promoted.output);
    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    let ordinary_memories = memory_store.list_for_standard_reader(Some(&project_str), None);
    assert!(!ordinary_memories
        .iter()
        .any(|memory| memory.title == "Isolation Canary Worker A Promotion"));
    let worker_a_memories = memory_store.list_for_hive_reader(&HiveMemoryReader {
        user_id: None,
        project_dir: Some(&project_str),
        worker_namespace_id: Some(&worker_a.memory_namespace_id),
        ..HiveMemoryReader::default()
    });
    let private_promotion = worker_a_memories
        .iter()
        .find(|memory| memory.title == "Isolation Canary Worker A Promotion")
        .expect("Worker A can read its promoted report memory");
    assert_eq!(private_promotion.namespace, MemoryNamespace::Crew);
    assert_eq!(
        private_promotion.namespace_id.as_deref(),
        Some(worker_a.memory_namespace_id.as_str())
    );
    assert_eq!(private_promotion.acl_scope, MemoryAclScope::Worker);
    assert_eq!(private_promotion.source, MemorySource::Tool);
    assert_eq!(
        private_promotion.source_session_id.as_deref(),
        Some("worker-a-dm")
    );
    let worker_b_memories = memory_store.list_for_hive_reader(&HiveMemoryReader {
        user_id: None,
        project_dir: Some(&project_str),
        worker_namespace_id: Some(&worker_b.memory_namespace_id),
        ..HiveMemoryReader::default()
    });
    assert!(!worker_b_memories
        .iter()
        .any(|memory| memory.title == "Isolation Canary Worker A Promotion"));

    let worker_a_report = report_ids.get("worker-a-dm").unwrap();
    for ctx in [&ordinary, &primary_hive, &worker_b_dm] {
        let hidden = ReportTool
            .execute(
                json!({ "action": "read", "report_id": worker_a_report }),
                ctx,
            )
            .await;
        assert!(hidden.is_error);
        assert!(hidden.output.contains("not_found"), "{}", hidden.output);
    }
    let worker_b_report = report_ids.get("worker-b-dm").unwrap();
    let hidden_from_a = ReportTool
        .execute(
            json!({ "action": "read", "report_id": worker_b_report }),
            &worker_a_group,
        )
        .await;
    assert!(hidden_from_a.is_error);
    assert!(hidden_from_a.output.contains("not_found"));
    for report_id in [report_ids.get("shared").unwrap(), worker_a_report] {
        let visible = ReportTool
            .execute(
                json!({ "action": "read", "report_id": report_id }),
                &worker_a_group,
            )
            .await;
        assert!(!visible.is_error, "read failed: {}", visible.output);
    }

    let unresolved = report_ctx(&db_path, &project, "primary-hive", Some(group_run.clone()));
    let unresolved_result = ReportTool
        .execute(json!({ "action": "list" }), &unresolved)
        .await;
    assert!(unresolved_result.is_error);
    assert!(
        unresolved_result
            .output
            .contains("no persisted lane binding"),
        "{}",
        unresolved_result.output
    );

    db.conn()
        .execute(
            "DELETE FROM hive_group_members WHERE group_id = ?1 AND worker_id = ?2",
            rusqlite::params![group.id, worker_a.id],
        )
        .expect("malform persisted lane");
    let malformed_result = ReportTool
        .execute(json!({ "action": "list" }), &worker_a_group)
        .await;
    assert!(malformed_result.is_error);
    assert!(
        malformed_result.output.contains("same-owner member"),
        "{}",
        malformed_result.output
    );
}
