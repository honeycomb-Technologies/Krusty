use rusqlite::params;
use tempfile::TempDir;

use super::activity::{SnapshotRunSummary, SnapshotTaskOutcome};
use super::render::build_current_snapshot_content;
use super::*;
use crate::agent::loop_events::LoopStopReason;
use crate::storage::{AgentMemory, Database, MemoryStore, MemoryType, ReportStore, TaskStatus};

fn create_db() -> (std::path::PathBuf, TempDir) {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("knowledge.db");
    let db = Database::new(&db_path).expect("db");
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params!["sess-1", "Knowledge Test", now, now],
        )
        .expect("seed session");
    (db_path, temp)
}

#[test]
fn build_current_snapshot_content_excludes_existing_snapshot_memory() {
    let snapshot = AgentMemory {
        id: "snapshot".to_string(),
        memory_type: MemoryType::Project,
        title: CURRENT_SNAPSHOT_TITLE.to_string(),
        content: "old".to_string(),
        project_dir: Some("/repo".to_string()),
        user_id: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let durable = AgentMemory {
        id: "durable".to_string(),
        memory_type: MemoryType::Feedback,
        title: "Wake cadence".to_string(),
        content: "Favor faster cadence while the queue is active.".to_string(),
        project_dir: Some("/repo".to_string()),
        user_id: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    };
    let compaction_flush = AgentMemory {
        id: "flush".to_string(),
        memory_type: MemoryType::Project,
        title: format!("{}1", crate::storage::COMPACTION_FLUSH_TITLE_PREFIX),
        content: "Full old transcript should not enter snapshots.".to_string(),
        project_dir: Some("/repo".to_string()),
        user_id: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-03T00:00:00Z".to_string(),
    };

    let content = build_current_snapshot_content(
        &[snapshot, durable, compaction_flush],
        &[],
        &[],
        &[],
        Some("/repo"),
    )
    .unwrap();

    assert!(content.contains("Wake cadence"));
    assert!(!content.contains("old"));
    assert!(!content.contains("Full old transcript"));
}

#[test]
fn refresh_current_snapshot_creates_and_updates_snapshot_memory() {
    let (db_path, _temp) = create_db();
    let memory_store = MemoryStore::new(Database::new(&db_path).expect("db"));
    let report_store = ReportStore::new(Database::new(&db_path).expect("db"));

    memory_store
        .save(
            MemoryType::Project,
            "Auth decision",
            "Keep wake state canonical in runtime state.",
            Some("/repo"),
            None,
        )
        .expect("seed memory");
    report_store
        .create_report(crate::storage::reports::CreateReportInput {
            title: "Wake audit",
            session_id: "sess-1",
            project_dir: Some("/repo"),
            report_root: None,
            content: "# Wake\nStable.",
            summary: "Wake is stable.",
            tags: &[],
            sources: &[],
        })
        .expect("seed report");

    let snapshot = refresh_current_snapshot(&db_path, Some("/repo"), None)
        .expect("refresh snapshot")
        .expect("snapshot");
    assert_eq!(snapshot.title, CURRENT_SNAPSHOT_TITLE);
    assert!(snapshot.content.contains("Auth decision"));
    assert!(snapshot.content.contains("Wake audit"));

    let all_memories = memory_store.list(Some("/repo"), None);
    assert_eq!(
        all_memories
            .iter()
            .filter(|memory| is_current_snapshot(memory))
            .count(),
        1
    );
}

#[test]
fn build_current_snapshot_content_includes_run_and_task_activity() {
    let content = build_current_snapshot_content(
        &[],
        &[],
        &[SnapshotRunSummary {
            title: "Wake audit".to_string(),
            updated_at: "2026-04-07T00:00:00Z".to_string(),
            pending_tasks: 1,
            in_progress_tasks: 1,
            completed_tasks: 2,
            failed_tasks: 0,
            blocked_tasks: 1,
            focus_subjects: vec!["Lock runtime cadence".to_string()],
            tool_calls: 4,
            awaiting_input_events: 0,
            provider_failures: 0,
            tool_errors: 0,
            last_stop_reason: Some(LoopStopReason::Completed),
        }],
        &[SnapshotTaskOutcome {
            session_title: "Wake audit".to_string(),
            subject: "Lock runtime cadence".to_string(),
            status: TaskStatus::Completed,
            updated_at: "2026-04-07T00:10:00Z".to_string(),
            result: Some("Cadence is now configurable per workspace.".to_string()),
        }],
        Some("/repo"),
    )
    .expect("snapshot content");

    assert!(content.contains("Recent Mako runs: 1"));
    assert!(content.contains("## Active Work"));
    assert!(content.contains("Lock runtime cadence"));
}
