use chrono::Utc;
use rusqlite::params;
use tempfile::TempDir;

use super::*;
use crate::agent::subagent::AgentCapability;
use crate::agent::DelegatedRunStage;
use crate::storage::Database;
use crate::tools::registry::{DelegationPolicy, PermissionMode};

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
fn round_trip_preserves_name_and_execute_only_contract() {
    let (store, _tmp) = create_store();
    let capabilities = [AgentCapability::Execute].into_iter().collect();
    store
        .create_run_with_child_contract(
            &DelegatedRunStartInput {
                delegated_run_id: "run-execute-only".to_string(),
                parent_session_id: "session-1".to_string(),
                parent_tool_call_id: Some("tool-execute".to_string()),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Created,
                provider: None,
                model: None,
                resumable: true,
                resumed_from_run_id: None,
                target_scope: vec![scope("project", ".", "project")],
            },
            Some("focused validation"),
            &capabilities,
        )
        .expect("create contracted run");

    let record = store
        .get_run("run-execute-only")
        .expect("get run")
        .expect("record exists");
    assert_eq!(record.child_name.as_deref(), Some("focused validation"));
    assert_eq!(record.capabilities, capabilities);
    assert_eq!(record.effective_capabilities(), capabilities);
}

#[test]
fn empty_contract_uses_legacy_role_fallback_only() {
    let (store, _tmp) = create_store();
    store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: "legacy-build".to_string(),
            parent_session_id: "session-1".to_string(),
            parent_tool_call_id: None,
            role: DelegatedRunRole::Build,
            stage: DelegatedRunStage::Complete,
            provider: None,
            model: None,
            resumable: true,
            resumed_from_run_id: None,
            target_scope: vec![scope("project", ".", "project")],
        })
        .expect("create legacy row");

    let record = store
        .get_run("legacy-build")
        .expect("get run")
        .expect("record exists");
    assert!(record.capabilities.is_empty());
    assert_eq!(
        record.effective_capabilities(),
        [
            AgentCapability::Read,
            AgentCapability::Write,
            AgentCapability::Execute,
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn finalized_background_artifact_preserves_delegation_policy_metadata() {
    let (store, _tmp) = create_store();
    let delegated_run_id = "run-policy".to_string();
    let policy = DelegationPolicy::for_subagent_build(PermissionMode::Supervised, Some(12));

    store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: delegated_run_id.clone(),
            parent_session_id: "session-1".to_string(),
            parent_tool_call_id: Some("tool-1".to_string()),
            role: DelegatedRunRole::Build,
            stage: DelegatedRunStage::Created,
            provider: Some("openai".to_string()),
            model: Some("gpt-test".to_string()),
            resumable: true,
            resumed_from_run_id: None,
            target_scope: vec![scope("main", ".", "project")],
        })
        .expect("create run");

    store
        .finalize_run(
            &delegated_run_id,
            DelegatedRunStage::Complete,
            &serde_json::json!({
                "delegated_run_id": delegated_run_id,
                "outcome": "success",
                "delegation_policy": policy.audit_json(),
            }),
            Some("Build complete"),
            false,
        )
        .expect("finalize run");

    let record = store
        .get_run("run-policy")
        .expect("get run")
        .expect("record exists");
    let artifact = record.artifact.expect("artifact persisted");
    assert_eq!(artifact["delegation_policy"]["surface"], "subagent_build");
    assert_eq!(
        artifact["delegation_policy"]["permission_mode"],
        "supervised"
    );
    assert_eq!(artifact["delegation_policy"]["max_turns"], 12);
    assert_eq!(artifact["delegation_policy"]["read_only_only"], false);
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

#[test]
fn cancelled_run_cannot_be_overwritten_by_late_child_finalization() {
    let (store, _tmp) = create_store();
    store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: "run-cancel".to_string(),
            parent_session_id: "session-1".to_string(),
            parent_tool_call_id: Some("tool-1".to_string()),
            role: DelegatedRunRole::Build,
            stage: DelegatedRunStage::Running,
            provider: None,
            model: None,
            resumable: true,
            resumed_from_run_id: None,
            target_scope: vec![scope("main", ".", "project")],
        })
        .expect("create run");

    store
        .finalize_run(
            "run-cancel",
            DelegatedRunStage::Cancelled,
            &serde_json::json!({"outcome": "cancelled"}),
            Some("parent interrupt"),
            true,
        )
        .expect("cancel run");
    store
        .finalize_run(
            "run-cancel",
            DelegatedRunStage::Failed,
            &serde_json::json!({"outcome": "failed"}),
            Some("late child result"),
            true,
        )
        .expect("late finalization is ignored");

    let record = store
        .get_run("run-cancel")
        .expect("load run")
        .expect("record");
    assert_eq!(record.stage, DelegatedRunStage::Cancelled);
    assert_eq!(record.artifact.unwrap()["outcome"], "cancelled");
}
