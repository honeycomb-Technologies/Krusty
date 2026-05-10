use super::{
    build_parent_context_brief, build_single_agent_artifact, build_single_agent_warnings,
    concise_target_label, emit_single_agent_completion, open_delegated_run_store,
    resolve_explore_target, truncate_utf8,
};
use crate::agent::subagent::{AgentProgressStatus, SubAgentResult};
use crate::agent::DelegatedRunStage;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::WorkspaceMode;
use crate::tools::registry::{DelegationPolicy, PermissionMode};
use crate::tools::ToolContext;
use crate::Database;
use std::fs;
use tempfile::TempDir;
use tokio::sync::mpsc;

fn sample_result(success: bool, error: Option<&str>) -> SubAgentResult {
    SubAgentResult {
        task_id: "agent-1".to_string(),
        agent_name: "planner".to_string(),
        delegated_run_id: Some("run-123".to_string()),
        success,
        output: "Found the relevant code paths".to_string(),
        files_examined: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
        duration_ms: 1200,
        turns_used: 3,
        error: error.map(ToString::to_string),
        policy_violations: vec![],
    }
}

#[test]
fn concise_target_label_uses_stable_readable_segments() {
    assert_eq!(
        concise_target_label("crates/krusty-core/src", 0),
        "krusty-core/src"
    );
    assert_eq!(concise_target_label("README.md", 1), "README.md");
    assert_eq!(concise_target_label("", 2), "target-3");
}

#[test]
fn resolve_explore_target_rejects_missing_or_outside_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).expect("project dirs");

    let inside = resolve_explore_target("src", &project, "directory").expect("inside target");
    assert!(inside.ends_with("src"));

    let missing = resolve_explore_target("missing", &project, "directory")
        .expect_err("missing target should fail");
    assert!(missing.contains("Missing explore target"));

    let outside = resolve_explore_target("../outside", &project, "directory")
        .expect_err("outside target should fail");
    assert!(outside.contains("outside project root") || outside.contains("Missing explore target"));
}

#[test]
fn parent_context_brief_extracts_last_turns() {
    let conversation = vec![
        ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "First question".to_string(),
            }],
        },
        ModelMessage {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "First answer".to_string(),
            }],
        },
        ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "Second question".to_string(),
            }],
        },
    ];

    let brief = build_parent_context_brief(&conversation, 2);
    assert!(brief.contains("[PARENT CONTEXT]"));
    assert!(brief.contains("[/PARENT CONTEXT]"));
    assert!(brief.contains("First answer"));
    assert!(brief.contains("Second question"));
}

#[test]
fn parent_context_brief_truncates_long_messages() {
    let long_text = "x".repeat(300);
    let conversation = vec![ModelMessage {
        role: Role::Assistant,
        content: vec![Content::Text {
            text: long_text.clone(),
        }],
    }];

    let brief = build_parent_context_brief(&conversation, 10);
    assert!(brief.contains("..."));
    assert!(brief.len() < long_text.len());
}

#[test]
fn parent_context_brief_empty_on_no_messages() {
    let brief = build_parent_context_brief(&[], 10);
    assert!(brief.is_empty());
}

#[test]
fn open_delegated_run_store_returns_none_without_db_path() {
    let ctx = ToolContext::default();

    assert!(open_delegated_run_store(&ctx).is_none());
}

#[test]
fn open_delegated_run_store_opens_valid_database() {
    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("delegated.db");
    Database::new(&db_path).expect("database");

    let ctx = ToolContext {
        working_dir: temp_dir.path().to_path_buf(),
        workspace_mode: WorkspaceMode::Neutral,
        db_path: Some(db_path),
        ..Default::default()
    };

    assert!(open_delegated_run_store(&ctx).is_some());
}

#[test]
fn open_delegated_run_store_returns_none_for_unopenable_database() {
    let temp_dir = TempDir::new().expect("tempdir");
    let invalid_db_path = temp_dir.path().join("not-a-db-file");
    std::fs::create_dir_all(&invalid_db_path).expect("directory path should exist");
    let ctx = ToolContext {
        working_dir: temp_dir.path().to_path_buf(),
        workspace_mode: WorkspaceMode::Neutral,
        db_path: Some(invalid_db_path),
        session_id: Some("session-1".to_string()),
        tool_use_id: Some("tool-1".to_string()),
        ..Default::default()
    };

    assert!(open_delegated_run_store(&ctx).is_none());
}

#[test]
fn single_agent_artifact_keeps_payload_shape() {
    let result = sample_result(true, None);
    let policy = DelegationPolicy::for_subagent_plan(PermissionMode::Autonomous, Some(8));

    let artifact = build_single_agent_artifact("run-123", &result, &policy);

    assert_eq!(artifact.review_summary, "Found the relevant code paths");
    assert_eq!(artifact.final_stage, DelegatedRunStage::Complete);
    assert_eq!(artifact.payload["delegated_run_id"], "run-123");
    assert_eq!(
        artifact.payload["findings"],
        "Found the relevant code paths"
    );
    assert_eq!(artifact.payload["files_examined_count"], 2);
    assert_eq!(artifact.payload["success"], true);
    assert_eq!(artifact.payload["outcome"], "success");
    assert_eq!(artifact.payload["agent_count"], 1);
    assert_eq!(artifact.payload["usable_agents"], 1);
    assert_eq!(artifact.payload["failed_agents"], 0);
    assert_eq!(
        artifact.payload["agents"][0]["summary"],
        "Found the relevant code paths"
    );
    assert_eq!(
        artifact.payload["delegation_policy"]["surface"],
        "subagent_plan"
    );
}

#[test]
fn single_agent_warnings_preserve_failure_wording() {
    let errored = sample_result(false, Some("tool timeout"));
    assert_eq!(
        build_single_agent_warnings(&errored, "Verification"),
        vec!["Verification failed: tool timeout".to_string()]
    );

    let empty = sample_result(false, None);
    assert_eq!(
        build_single_agent_warnings(&empty, "Planning"),
        vec!["Planning completed without usable results.".to_string()]
    );

    let ok = sample_result(true, Some("ignored"));
    assert!(build_single_agent_warnings(&ok, "Exploration").is_empty());
}

#[test]
fn emit_single_agent_completion_reports_summary_and_status() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let result = sample_result(false, Some("lint failed"));

    emit_single_agent_completion(&Some(tx), "run-999", "verify", &result, "short summary");

    let progress = rx.try_recv().expect("completion event");
    assert_eq!(progress.delegated_run_id.as_deref(), Some("run-999"));
    assert_eq!(progress.task_id, "agent-1");
    assert_eq!(progress.name, "verify");
    assert_eq!(progress.status, AgentProgressStatus::Failed);
    assert_eq!(
        progress.completion_summary.as_deref(),
        Some("short summary")
    );
}

#[test]
fn truncate_utf8_respects_char_boundaries() {
    // ASCII: truncation at exact byte offset is fine
    assert_eq!(truncate_utf8("hello world", 5), "hello");
    // Multi-byte: U+00E9 (e-acute) is 2 bytes in UTF-8
    let s = "caf\u{00e9}!"; // "cafe!" with accented e — 6 bytes total
    assert_eq!(truncate_utf8(s, 10), s); // within budget
    assert_eq!(truncate_utf8(s, 4), "caf"); // byte 4 is mid-char, backs up to 3
    assert_eq!(truncate_utf8(s, 5), "caf\u{00e9}"); // byte 5 is right after the char
                                                    // Empty and zero budget
    assert_eq!(truncate_utf8("", 0), "");
    assert_eq!(truncate_utf8("abc", 0), "");
}
