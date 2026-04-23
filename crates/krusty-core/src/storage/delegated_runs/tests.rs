use chrono::Utc;
use rusqlite::params;
use tempfile::TempDir;

use super::*;
use crate::agent::DelegatedRunStage;
use crate::storage::Database;

fn create_store() -> (DelegatedRunStore, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("delegated-runs.db");
    let db = Database::new(&db_path).expect("db");
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params!["session-1", "Delegated Run Test", now, now],
        )
        .expect("seed session");
    (DelegatedRunStore::new(db), temp_dir)
}

fn scope(label: &str, path: &str, kind: &str) -> DelegatedRunScope {
    DelegatedRunScope {
        label: label.to_string(),
        path: path.to_string(),
        kind: kind.to_string(),
    }
}

#[test]
fn round_trip_persisted_delegated_run() {
    let (store, _tmp) = create_store();
    let delegated_run_id = "run-1".to_string();
    store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: delegated_run_id.clone(),
            parent_session_id: "session-1".to_string(),
            parent_tool_call_id: Some("tool-1".to_string()),
            role: DelegatedRunRole::Explore,
            stage: DelegatedRunStage::Created,
            provider: Some("minimax".to_string()),
            model: Some("MiniMax-M2.5".to_string()),
            resumable: true,
            resumed_from_run_id: None,
            target_scope: vec![scope(
                "storage",
                "crates/krusty-core/src/storage",
                "directory",
            )],
        })
        .expect("create run");

    store
        .update_snapshot(
            &delegated_run_id,
            DelegatedRunStage::Running,
            &DelegatedRunSnapshot {
                stage: DelegatedRunStage::Running,
                agents: vec![DelegatedRunAgentSnapshot {
                    task_id: "dir-0".to_string(),
                    agent_name: "storage".to_string(),
                    status: "running".to_string(),
                    tool_count: 3,
                    tokens: 120,
                    current_action: Some("reading mod.rs".to_string()),
                    completion_summary: None,
                    lines_added: 0,
                    lines_removed: 0,
                    completed_plan_task: None,
                }],
            },
        )
        .expect("update snapshot");

    store
        .finalize_run(
            &delegated_run_id,
            DelegatedRunStage::Complete,
            &serde_json::json!({
                "human_review": "Storage owns session persistence and runtime traces."
            }),
            Some("Storage owns session persistence and runtime traces."),
            true,
        )
        .expect("finalize run");

    let record = store
        .get_run(&delegated_run_id)
        .expect("get run")
        .expect("record exists");
    assert_eq!(record.stage, DelegatedRunStage::Complete);
    assert_eq!(record.role, DelegatedRunRole::Explore);
    assert!(record.resumable);
    assert_eq!(record.target_scope.len(), 1);
    assert_eq!(
        record.snapshot.as_ref().unwrap().agents[0].agent_name,
        "storage"
    );
    assert_eq!(
        record.human_review.as_deref(),
        Some("Storage owns session persistence and runtime traces.")
    );
}

#[test]
fn find_related_run_matches_scope_key() {
    let (store, _tmp) = create_store();
    let scope = vec![scope("agent", "crates/krusty-core/src/agent", "directory")];
    store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: "run-1".to_string(),
            parent_session_id: "session-1".to_string(),
            parent_tool_call_id: Some("tool-1".to_string()),
            role: DelegatedRunRole::Explore,
            stage: DelegatedRunStage::Complete,
            provider: None,
            model: None,
            resumable: true,
            resumed_from_run_id: None,
            target_scope: scope.clone(),
        })
        .expect("create run");

    let found = store
        .find_related_run("session-1", DelegatedRunRole::Explore, &scope)
        .expect("query related")
        .expect("related run");
    assert_eq!(found.delegated_run_id, "run-1");
}
