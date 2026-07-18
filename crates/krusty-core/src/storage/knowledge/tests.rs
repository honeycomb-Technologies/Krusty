use rusqlite::params;
use tempfile::TempDir;

use super::activity::{SnapshotRunSummary, SnapshotTaskOutcome};
use super::render::build_current_snapshot_content;
use super::*;
use crate::agent::loop_events::LoopStopReason;
use crate::storage::{
    AgentMemory, AutonomousTaskStore, Database, MemoryNamespace, MemorySensitivity, MemorySource,
    MemoryStatus, MemoryStore, MemoryType, ReportStore, TaskStatus,
};

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

fn test_memory(id: &str, memory_type: MemoryType, title: &str, content: &str) -> AgentMemory {
    AgentMemory {
        id: id.to_string(),
        memory_type,
        title: title.to_string(),
        content: content.to_string(),
        project_dir: Some("/repo".to_string()),
        user_id: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        canonical_key: None,
        namespace: MemoryNamespace::Shared,
        namespace_id: None,
        status: MemoryStatus::Active,
        source: MemorySource::Legacy,
        source_session_id: None,
        source_message_id: None,
        confidence: 1.0,
        sensitivity: MemorySensitivity::Normal,
        pinned: false,
        supersedes_id: None,
        last_accessed_at: None,
        access_count: 0,
    }
}

#[test]
fn build_current_snapshot_content_excludes_existing_snapshot_memory() {
    let snapshot = test_memory(
        "snapshot",
        MemoryType::Project,
        CURRENT_SNAPSHOT_TITLE,
        "old",
    );
    let durable = test_memory(
        "durable",
        MemoryType::Feedback,
        "Wake cadence",
        "Favor faster cadence while the queue is active.",
    );
    let compaction_flush = test_memory(
        "flush",
        MemoryType::Project,
        &format!("{}1", crate::storage::COMPACTION_FLUSH_TITLE_PREFIX),
        "Full old transcript should not enter snapshots.",
    );

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
fn refresh_current_snapshot_uses_separate_knowledge_storage() {
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
    assert_eq!(snapshot.project_dir.as_deref(), Some("/repo"));
    assert_eq!(snapshot.user_id, None);

    let all_memories = memory_store.list(Some("/repo"), None);
    assert!(all_memories
        .iter()
        .all(|memory| !is_current_snapshot(memory)));

    let loaded = get_current_snapshot(&db_path, Some("/repo"), None)
        .expect("load snapshot")
        .expect("stored snapshot");
    assert_eq!(loaded, snapshot);

    let replay = refresh_current_snapshot(&db_path, Some("/repo"), None)
        .expect("refresh replay")
        .expect("snapshot replay");
    assert_eq!(replay, snapshot, "unchanged materialization is stable");
}

#[test]
fn refresh_current_snapshot_isolated_by_exact_owner_and_project() {
    let (db_path, _temp) = create_db();
    let memory_store = MemoryStore::new(Database::new(&db_path).expect("db"));
    memory_store
        .save(
            MemoryType::Project,
            "Alice project",
            "Alice-only context.",
            Some("/repo-a"),
            Some("alice"),
        )
        .expect("alice memory");
    memory_store
        .save(
            MemoryType::Project,
            "Bob project",
            "Bob-only context.",
            Some("/repo-b"),
            Some("bob"),
        )
        .expect("bob memory");

    let alice = refresh_current_snapshot(&db_path, Some("/repo-a"), Some("alice"))
        .expect("alice refresh")
        .expect("alice snapshot");
    let bob = refresh_current_snapshot(&db_path, Some("/repo-b"), Some("bob"))
        .expect("bob refresh")
        .expect("bob snapshot");

    assert_ne!(alice.id, bob.id);
    assert!(alice.content.contains("Alice project"));
    assert!(!alice.content.contains("Bob project"));
    assert!(bob.content.contains("Bob project"));
    assert!(!bob.content.contains("Alice project"));
    assert_eq!(
        get_current_snapshot(&db_path, Some("/repo-a"), Some("bob")).unwrap(),
        None
    );
}

#[test]
fn refresh_snapshot_same_project_isolates_memory_reports_and_activity_by_exact_owner() {
    let (db_path, _temp) = create_db();
    let project = "/shared/repo";
    let db = Database::new(&db_path).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO users (id, email, license_tier)
                 VALUES ('alice', 'alice@snapshot.test', 'free');
             INSERT INTO users (id, email, license_tier)
                 VALUES ('bob', 'bob@snapshot.test', 'free');",
        )
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    for (session_id, title, user_id) in [
        ("snapshot-local", "Local Snapshot", None),
        ("snapshot-alice", "Alice Snapshot", Some("alice")),
        ("snapshot-bob", "Bob Snapshot", Some("bob")),
    ] {
        db.conn()
            .execute(
                "INSERT INTO sessions (
                    id, title, created_at, updated_at, working_dir, project_dir,
                    workspace_mode, user_id, session_type
                 ) VALUES (?1, ?2, ?3, ?3, ?4, ?4, 'selected', ?5, 'mako')",
                params![session_id, title, now, project, user_id],
            )
            .unwrap();
    }

    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    for (title, marker, user_id) in [
        ("Local durable", "local-memory", None),
        ("Alice durable", "alice-memory", Some("alice")),
        ("Bob durable", "bob-memory", Some("bob")),
    ] {
        memory_store
            .save(MemoryType::Project, title, marker, Some(project), user_id)
            .unwrap();
    }

    let report_store = ReportStore::new(Database::new(&db_path).unwrap());
    for (session_id, title, marker) in [
        ("snapshot-local", "Local report", "local-report"),
        ("snapshot-alice", "Alice report", "alice-report"),
        ("snapshot-bob", "Bob report", "bob-report"),
    ] {
        report_store
            .create_report(crate::storage::reports::CreateReportInput {
                title,
                session_id,
                project_dir: Some(project),
                report_root: None,
                content: marker,
                summary: marker,
                tags: &[],
                sources: &[],
            })
            .unwrap();
    }

    let task_store = AutonomousTaskStore::new(Database::new(&db_path).unwrap());
    for (session_id, subject) in [
        ("snapshot-local", "local-task-marker"),
        ("snapshot-alice", "alice-task-marker"),
        ("snapshot-bob", "bob-task-marker"),
    ] {
        task_store
            .create_task(session_id, subject, "", &[])
            .unwrap();
    }

    let alice = refresh_current_snapshot(&db_path, Some(project), Some("alice"))
        .unwrap()
        .unwrap();
    let bob = refresh_current_snapshot(&db_path, Some(project), Some("bob"))
        .unwrap()
        .unwrap();
    let local = refresh_current_snapshot(&db_path, Some(project), None)
        .unwrap()
        .unwrap();

    for (snapshot, owned, foreign_a, foreign_b) in [
        (
            &alice,
            ["alice-memory", "Alice report", "alice-task-marker"],
            ["bob-memory", "Bob report", "bob-task-marker"],
            ["local-memory", "Local report", "local-task-marker"],
        ),
        (
            &bob,
            ["bob-memory", "Bob report", "bob-task-marker"],
            ["alice-memory", "Alice report", "alice-task-marker"],
            ["local-memory", "Local report", "local-task-marker"],
        ),
        (
            &local,
            ["local-memory", "Local report", "local-task-marker"],
            ["alice-memory", "Alice report", "alice-task-marker"],
            ["bob-memory", "Bob report", "bob-task-marker"],
        ),
    ] {
        for marker in owned {
            assert!(snapshot.content.contains(marker));
        }
        for marker in foreign_a.into_iter().chain(foreign_b) {
            assert!(!snapshot.content.contains(marker));
        }
    }
}

#[test]
fn refresh_removes_empty_materialization_without_creating_memory() {
    let (db_path, _temp) = create_db();
    let memory_store = MemoryStore::new(Database::new(&db_path).expect("db"));
    let memory = memory_store
        .save(
            MemoryType::Project,
            "Temporary",
            "This will be removed.",
            Some("/repo"),
            None,
        )
        .expect("memory");
    assert!(refresh_current_snapshot(&db_path, Some("/repo"), None)
        .unwrap()
        .is_some());

    memory_store.delete(&memory.id).expect("delete memory");
    assert_eq!(
        refresh_current_snapshot(&db_path, Some("/repo"), None).unwrap(),
        None
    );
    assert_eq!(
        get_current_snapshot(&db_path, Some("/repo"), None).unwrap(),
        None
    );
    assert!(memory_store.list(Some("/repo"), None).is_empty());
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
