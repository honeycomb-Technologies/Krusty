use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::hooks::{HookResult, PreToolHook};

use super::policy::DELEGATED_TOOL_TIMEOUT;
use super::runtime::execution_timeout_for_call;
use super::*;

fn create_test_context() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_tool_registry_nonexistent_tool() {
    let registry = ToolRegistry::new();
    let ctx = create_test_context();

    let result = registry.execute("nonexistent_tool", json!({}), &ctx).await;

    assert!(result.is_none());
}

#[tokio::test]
async fn test_tool_context_defaults() {
    let ctx = ToolContext::default();

    assert!(ctx.process_registry.is_none());
    assert!(ctx.timeout.is_none());
    assert!(!ctx.plan_mode);
    assert_eq!(
        ctx.working_dir,
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    );
}

#[tokio::test]
async fn test_tool_result_success() {
    let result = ToolResult::success("Test output");
    assert!(!result.is_error);
    assert_eq!(result.output, "Test output");
}

#[tokio::test]
async fn test_tool_result_error() {
    let result = ToolResult::error("Test error");
    assert!(result.is_error);
    assert!(result.output.contains("error"));
    assert!(result.output.contains("Test error"));
    let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["error"]["message"], "Test error");
    assert_eq!(parsed["error"]["code"], "tool_error");
}

#[test]
fn test_tool_policy_contracts() {
    let read_policy = tool_policy("read");
    assert_eq!(read_policy.category, ToolCategory::ReadOnly);
    assert!(read_policy.retry_timeout_once);
    assert!(read_policy.allowed_in_plan_mode);
    assert!(!read_policy.requires_supervised_approval);
    assert_eq!(read_policy.timeout_override, None);

    let delegated_read_policy = tool_policy("agent");
    assert_eq!(delegated_read_policy.category, ToolCategory::ReadOnly);
    assert!(!delegated_read_policy.retry_timeout_once);
    assert_eq!(
        delegated_read_policy.timeout_override,
        Some(DELEGATED_TOOL_TIMEOUT)
    );

    let write_policy = tool_policy("apply_patch");
    assert_eq!(write_policy.category, ToolCategory::Write);
    assert!(!write_policy.retry_timeout_once);
    assert!(!write_policy.allowed_in_plan_mode);
    assert!(write_policy.requires_supervised_approval);

    let interactive_policy = tool_policy("task_start");
    assert_eq!(interactive_policy.category, ToolCategory::Interactive);
    assert!(interactive_policy.allowed_in_plan_mode);
    assert!(!interactive_policy.requires_supervised_approval);

    let workspace_policy = tool_policy("set_workspace_context");
    assert_eq!(workspace_policy.category, ToolCategory::Interactive);
    assert!(workspace_policy.allowed_in_plan_mode);
    assert!(workspace_policy.requires_supervised_approval);

    let agent_build_policy = tool_policy_for_call("agent", &json!({ "agent_type": "build" }));
    assert_eq!(agent_build_policy.category, ToolCategory::Write);
    assert!(agent_build_policy.requires_supervised_approval);
    assert!(!agent_build_policy.allowed_in_plan_mode);
    assert_eq!(
        agent_build_policy.timeout_override,
        Some(DELEGATED_TOOL_TIMEOUT)
    );

    let agent_explore_policy = tool_policy_for_call("agent", &json!({ "agent_type": "explore" }));
    assert_eq!(agent_explore_policy.category, ToolCategory::ReadOnly);
    assert!(!agent_explore_policy.requires_supervised_approval);
    assert!(!agent_explore_policy.retry_timeout_once);
    assert!(agent_explore_policy.allowed_in_plan_mode);

    let custom_read_policy = tool_policy_for_call("agent", &json!({ "profile": "security-audit" }));
    assert_eq!(custom_read_policy.category, ToolCategory::ReadOnly);
    assert!(!custom_read_policy.requires_supervised_approval);

    let custom_write_policy = tool_policy_for_call(
        "agent",
        &json!({ "profile": "refactor", "capabilities": ["read", "write"] }),
    );
    assert_eq!(custom_write_policy.category, ToolCategory::Write);
    assert!(custom_write_policy.requires_supervised_approval);

    let execute_policy = tool_policy_for_call(
        "agent",
        &json!({ "name": "validator", "capabilities": ["execute"] }),
    );
    assert_eq!(execute_policy.category, ToolCategory::Write);
    assert!(execute_policy.requires_supervised_approval);
    assert!(!execute_policy.allowed_in_plan_mode);

    for action in ["list", "status", "wait"] {
        let policy = tool_policy_for_call("agent", &json!({ "action": action }));
        assert_eq!(policy.category, ToolCategory::ReadOnly, "{action}");
        assert!(policy.retry_timeout_once, "{action}");
        assert!(!agent_call_starts_run(&json!({ "action": action })));
    }
    for action in ["message", "interrupt"] {
        let policy = tool_policy_for_call("agent", &json!({ "action": action }));
        assert_eq!(policy.category, ToolCategory::Interactive, "{action}");
        assert!(!policy.requires_supervised_approval);
    }
    let followup_policy = tool_policy_for_call("agent", &json!({ "action": "followup" }));
    assert_eq!(followup_policy.category, ToolCategory::Write);
    assert!(followup_policy.requires_supervised_approval);
    assert!(!followup_policy.retry_timeout_once);
    assert!(!followup_policy.allowed_in_plan_mode);
    assert!(!agent_call_starts_run(&json!({ "action": "followup" })));
    assert!(agent_call_may_start_run(&json!({ "action": "followup" })));
    let resume_policy = tool_policy_for_call("agent", &json!({ "action": "resume" }));
    assert_eq!(resume_policy.category, ToolCategory::Write);
    assert!(resume_policy.requires_supervised_approval);
    assert!(!resume_policy.retry_timeout_once);
    assert!(agent_call_starts_run(&json!({ "action": "resume" })));
    assert!(agent_call_may_start_run(&json!({ "action": "resume" })));

    let deferred_read_policy = tool_policy_for_call(
        "tool_search",
        &json!({
            "action": "execute",
            "tool": "web_fetch",
            "arguments": {"url": "https://example.com"}
        }),
    );
    assert_eq!(deferred_read_policy.category, ToolCategory::ReadOnly);
    assert!(!deferred_read_policy.requires_supervised_approval);

    let deferred_write_policy = tool_policy_for_call(
        "tool_search",
        &json!({
            "action": "execute",
            "tool": "edit",
            "arguments": {"file_path": "src/lib.rs"}
        }),
    );
    assert_eq!(deferred_write_policy.category, ToolCategory::Write);
    assert!(deferred_write_policy.requires_supervised_approval);

    assert_eq!(
        authorize_tool_call(
            "agent",
            &json!({ "agent_type": "build" }),
            PermissionMode::Supervised,
            false,
        ),
        ToolAuthorization::RequiresApproval
    );
    assert_eq!(
        authorize_tool_call(
            "agent",
            &json!({ "agent_type": "build" }),
            PermissionMode::Autonomous,
            false,
        ),
        ToolAuthorization::Execute
    );
    assert_eq!(
        authorize_tool_call(
            "agent",
            &json!({ "agent_type": "build" }),
            PermissionMode::Autonomous,
            true,
        ),
        ToolAuthorization::BlockedInPlanMode
    );
    assert_eq!(
        authorize_tool_call(
            "agent",
            &json!({ "agent_type": "verify" }),
            PermissionMode::Supervised,
            true,
        ),
        ToolAuthorization::BlockedInPlanMode
    );
    assert_eq!(
        authorize_tool_call(
            "agent",
            &json!({ "name": "validator", "capabilities": ["execute"] }),
            PermissionMode::Supervised,
            false,
        ),
        ToolAuthorization::RequiresApproval
    );
    assert_eq!(
        authorize_tool_call(
            "agent",
            &json!({ "action": "followup", "delegated_run_id": "run-build" }),
            PermissionMode::Supervised,
            false,
        ),
        ToolAuthorization::RequiresApproval
    );
    assert_eq!(
        authorize_tool_call(
            "agent",
            &json!({ "action": "followup", "delegated_run_id": "run-build" }),
            PermissionMode::Autonomous,
            true,
        ),
        ToolAuthorization::BlockedInPlanMode
    );
}

fn ai_tool(name: &str) -> crate::ai::types::AiTool {
    crate::ai::types::AiTool {
        name: name.to_string(),
        description: format!("{name} description"),
        input_schema: json!({"type": "object"}),
        prompt: Some("legacy prompt".to_string()),
    }
}

#[test]
fn default_code_request_surface_is_small_and_deterministic() {
    let names = [
        "write",
        "tool_search",
        "read",
        "grep",
        "glob",
        "bash",
        "apply_patch",
        "agent",
        "AskUserQuestion",
        "enter_plan_mode",
        "web_search",
    ];
    let tools = names.into_iter().map(ai_tool).collect();
    let filtered = ToolRequestPolicy::default().filter(tools);
    let filtered_names = filtered
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();

    assert!(filtered.len() <= DEFAULT_CODE_TOOL_LIMIT);
    assert_eq!(filtered.len(), 8);
    assert_eq!(
        filtered_names,
        vec![
            "AskUserQuestion",
            "agent",
            "apply_patch",
            "bash",
            "glob",
            "grep",
            "read",
            "tool_search",
        ]
    );
}

#[test]
fn autonomous_build_cannot_turn_authorized_work_into_plan_approval_stop() {
    let tools = vec![
        ai_tool("enter_plan_mode"),
        ai_tool("agent"),
        ai_tool("read"),
    ];
    let autonomous = ToolRequestPolicy::code(PermissionMode::Autonomous, false, false, true, &[])
        .filter(tools.clone());
    let supervised =
        ToolRequestPolicy::code(PermissionMode::Supervised, false, false, true, &[]).filter(tools);

    assert!(!autonomous.iter().any(|tool| tool.name == "enter_plan_mode"));
    assert!(supervised.iter().any(|tool| tool.name == "enter_plan_mode"));
}

#[test]
fn active_goal_surface_keeps_execution_and_strict_step_tools_reachable() {
    let tools = [
        "AskUserQuestion",
        "add_subtask",
        "agent",
        "apply_patch",
        "bash",
        "read",
        "set_dependency",
        "set_work_mode",
        "task_complete",
        "task_start",
        "tool_search",
        "workflow_update",
        "write",
    ]
    .into_iter()
    .map(ai_tool)
    .collect();
    let policy = ToolRequestPolicy::code(PermissionMode::Autonomous, false, true, true, &[]);
    let names = policy
        .filter(tools)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert_eq!(names.len(), 9);
    for required in [
        "AskUserQuestion",
        "agent",
        "apply_patch",
        "task_start",
        "task_complete",
        "tool_search",
        "workflow_update",
    ] {
        assert!(
            names.iter().any(|name| name == required),
            "missing {required}"
        );
    }
}

#[test]
fn non_gpt_models_receive_edit_and_write_instead_of_apply_patch() {
    let tools = [
        "AskUserQuestion",
        "agent",
        "apply_patch",
        "bash",
        "edit",
        "enter_plan_mode",
        "glob",
        "grep",
        "read",
        "tool_search",
        "write",
    ]
    .into_iter()
    .map(ai_tool)
    .collect();
    let policy = ToolRequestPolicy::default().with_mutation_surface(
        MutationToolSurface::for_model(crate::ai::providers::ProviderId::Grok, "grok-4.5"),
    );
    let names = policy
        .filter(tools)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert_eq!(names.len(), DEFAULT_CODE_TOOL_LIMIT - 1);
    assert!(names.iter().any(|name| name == "edit"));
    assert!(names.iter().any(|name| name == "write"));
    assert!(names.iter().any(|name| name == "glob"));
    assert!(!names.iter().any(|name| name == "apply_patch"));
    assert!(!names.iter().any(|name| name == "enter_plan_mode"));
}

#[test]
fn effective_deferred_call_exposes_target_and_arguments() {
    let wrapper = json!({
        "action": "execute",
        "tool": "edit",
        "arguments": {"file_path": "src/lib.rs"}
    });
    let (name, arguments) = effective_tool_call("tool_search", &wrapper);

    assert_eq!(name, "edit");
    assert_eq!(arguments["file_path"], "src/lib.rs");
}

#[test]
fn request_policy_filters_plan_writes_disabled_tools_and_unapprovable_mutations() {
    let all = vec![
        ai_tool("read"),
        ai_tool("bash"),
        ai_tool("apply_patch"),
        ai_tool("set_work_mode"),
        ai_tool("tool_search"),
        ai_tool("workflow_propose"),
    ];
    let plan_policy = ToolRequestPolicy::code(PermissionMode::Supervised, true, false, false, &[]);
    let names = plan_policy
        .filter(all)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["read", "workflow_propose"]);
}

#[test]
fn delegated_explore_policy_blocks_write_tools() {
    let policy = DelegationPolicy::for_subagent_explore(PermissionMode::Autonomous, Some(20));
    assert!(policy.authorize_tool("read", false).is_ok());
    assert!(policy.authorize_tool("edit", false).is_err());
    assert!(policy
        .authorize_tool_call("agent", &json!({ "agent_type": "explore" }), false)
        .is_err());
    assert!(policy
        .authorize_tool_call("agent", &json!({ "agent_type": "build" }), false)
        .is_err());
    assert!(policy
        .authorize_tool_call(
            "tool_search",
            &json!({
                "action": "execute",
                "tool": "edit",
                "arguments": {"file_path": "src/lib.rs"}
            }),
            false,
        )
        .is_err());
}

#[test]
fn delegated_execute_policy_routes_browser_qa_to_governed_tool() {
    let scope = std::collections::HashSet::from(["bash".to_string(), "browser_check".to_string()]);
    let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, None)
        .with_execution_tool_allowlist(Some(&scope));
    let install = json!({
        "command": "PLAYWRIGHT_BROWSERS_PATH=.pw-browsers npx --yes playwright@1.55.0 install chromium"
    });

    let error = policy
        .authorize_tool_call("bash", &install, false)
        .expect_err("delegated browser download must be blocked");
    assert!(error.contains("browser_check"));
    assert!(policy
        .authorize_tool_call(
            "bash",
            &json!({"command": "npm test && npm run build"}),
            false,
        )
        .is_ok());
    assert!(policy
        .authorize_tool_call(
            "browser_check",
            &json!({"url": "http://127.0.0.1:4173/"}),
            false,
        )
        .is_ok());
}

#[test]
fn exact_parent_agent_scope_cannot_expand_into_child_mutation_tools() {
    let parent_scope = HashSet::from(["agent".to_string()]);
    let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, Some(13))
        .with_execution_tool_allowlist(Some(&parent_scope));

    for escaped_tool in ["bash", "write", "edit", "apply_patch"] {
        let error = policy
            .authorize_tool(escaped_tool, false)
            .expect_err("outer agent capability must not create child mutation capabilities");
        assert!(error.contains("explicit tool capability"));
    }

    // The outer `agent` name is a parent invocation capability, not permission
    // for a child to recursively spawn another agent.
    assert!(policy
        .authorize_tool_call(
            "agent",
            &json!({"agent_type": "build", "prompt": "escape"}),
            false,
        )
        .expect_err("recursive delegation must fail closed")
        .contains("cannot recursively delegate"));
    assert_eq!(policy.inherited_permission_mode, PermissionMode::Autonomous);
    assert_eq!(policy.max_turns, Some(13));
    assert_eq!(
        policy.execution_tool_allowlist,
        Some(["agent".to_string()].into_iter().collect())
    );
}

#[test]
fn delegated_exact_scope_checks_both_tool_search_wrapper_and_effective_target() {
    let parent_scope = HashSet::from([
        "agent".to_string(),
        "read".to_string(),
        "tool_search".to_string(),
    ]);
    let policy = DelegationPolicy::for_subagent_explore(PermissionMode::Supervised, Some(5))
        .with_execution_tool_allowlist(Some(&parent_scope));

    assert!(policy.authorize_tool("read", false).is_ok());
    assert!(policy
        .authorize_tool_call(
            "tool_search",
            &json!({
                "action": "execute",
                "tool": "read",
                "arguments": {"file_path": "src/lib.rs"}
            }),
            false,
        )
        .is_ok());
    assert!(policy
        .authorize_tool_call(
            "tool_search",
            &json!({
                "action": "execute",
                "tool": "bash",
                "arguments": {"command": "pwd"}
            }),
            false,
        )
        .expect_err("hidden target must remain below the parent ceiling")
        .contains("explicit tool capability"));
    assert!(policy
        .authorize_tool_call(
            "tool_search",
            &json!({
                "action": "execute",
                "tool": "agent",
                "arguments": {"agent_type": "explore", "prompt": "recurse"}
            }),
            false,
        )
        .expect_err("nested agent dispatch must remain non-recursive")
        .contains("cannot recursively delegate"));

    let audit = policy.audit_json();
    assert_eq!(audit["permission_mode"], "supervised");
    assert_eq!(audit["max_turns"], 5);
    assert_eq!(
        audit["execution_tool_allowlist"],
        json!(["agent", "read", "tool_search"])
    );
}

#[test]
fn execute_only_child_keeps_an_exact_capability_surface() {
    let policy = DelegationPolicy::for_subagent_child(
        PermissionMode::Autonomous,
        Some(6),
        false,
        false,
        true,
    );

    assert!(policy.authorize_tool("bash", false).is_ok());
    assert!(policy.authorize_tool("browser_check", false).is_ok());
    for forbidden in ["read", "grep", "write", "edit", "apply_patch"] {
        assert!(
            policy.authorize_tool(forbidden, false).is_err(),
            "execute-only child unexpectedly allowed {forbidden}"
        );
    }
    assert_eq!(
        policy.execution_tool_allowlist,
        Some(BTreeSet::from([
            "bash".to_string(),
            "browser_check".to_string(),
        ]))
    );
}

#[test]
fn mixed_write_execute_group_is_a_valid_ceiling_for_disjoint_child_policies() {
    let group =
        DelegationPolicy::for_subagent_child(PermissionMode::Autonomous, None, true, true, true);
    let engine =
        DelegationPolicy::for_subagent_child(PermissionMode::Autonomous, None, true, true, true);
    let pwa =
        DelegationPolicy::for_subagent_child(PermissionMode::Autonomous, None, true, true, false);
    let verifier =
        DelegationPolicy::for_subagent_child(PermissionMode::Autonomous, None, true, false, true);

    assert!(group.bash_allowed);
    assert!(group
        .execution_tool_allowlist
        .as_ref()
        .is_some_and(|tools| tools.contains("bash")));
    assert!(engine.is_within(&group));
    assert!(pwa.is_within(&group));
    assert!(verifier.is_within(&group));
}

#[test]
fn explicit_child_capabilities_intersect_the_parent_tool_ceiling() {
    let parent_scope = HashSet::from(["read".to_string(), "bash".to_string()]);
    let policy =
        DelegationPolicy::for_subagent_child(PermissionMode::Autonomous, None, true, true, false)
            .with_execution_tool_allowlist(Some(&parent_scope));

    assert!(policy.authorize_tool("read", false).is_ok());
    assert!(policy.authorize_tool("bash", false).is_err());
    assert!(policy.authorize_tool("write", false).is_err());
    assert_eq!(
        policy.execution_tool_allowlist,
        Some(BTreeSet::from(["read".to_string()]))
    );
}

#[test]
fn research_accounting_prefers_capabilities_with_legacy_fallback() {
    assert!(agent_call_is_research(&json!({
        "name": "audit",
        "instructions": "inspect",
        "capabilities": ["read", "execute"]
    })));
    assert!(!agent_call_is_research(&json!({
        "name": "repair",
        "instructions": "edit",
        "capabilities": ["read", "write"]
    })));
    assert!(!agent_call_is_research(&json!({
        "name": "validator",
        "instructions": "test",
        "capabilities": ["execute"]
    })));
    assert!(agent_call_is_research(&json!({"agent_type": "verify"})));
    assert!(agent_call_is_research(&json!({
        "name": "audit",
        "instructions": "inspect"
    })));
    assert!(!agent_call_is_research(&json!({
        "action": "resume",
        "delegated_run_id": "run-1"
    })));
}

#[test]
fn execution_profile_prefers_exact_capabilities_over_legacy_labels() {
    let write_child = json!({
        "profile": "verify",
        "capabilities": ["read", "write"]
    });
    assert_eq!(agent_call_execution_profile(&write_child), "build");
    assert!(agent_call_requests_write(&write_child));

    let execute_only = json!({
        "profile": "build",
        "capabilities": ["execute"]
    });
    assert_eq!(agent_call_execution_profile(&execute_only), "explore");
    assert!(agent_call_requests_write(&execute_only));
}

#[test]
fn structured_task_capabilities_are_resolved_before_tool_execution() {
    let graph = json!({
        "name": "build proof",
        "instructions": "build and verify the proof",
        "tasks": [
            {
                "id": "core",
                "instructions": "implement the core",
                "capabilities": ["read", "write"],
                "write_intent": ["src"]
            },
            {
                "id": "verify",
                "instructions": "run verification",
                "capabilities": ["read", "execute"],
                "depends_on": ["core"]
            }
        ]
    });

    assert!(agent_call_requests_write(&graph));
    assert_eq!(agent_call_execution_profile(&graph), "build");
    assert!(!agent_call_is_research(&graph));
}

#[test]
fn structured_read_graph_keeps_the_executor_default_in_policy_resolution() {
    let graph = json!({
        "name": "audit proof",
        "instructions": "inspect the proof",
        "tasks": [{"id": "audit", "instructions": "inspect files"}]
    });

    assert!(!agent_call_requests_write(&graph));
    assert_eq!(agent_call_execution_profile(&graph), "explore");
    assert!(agent_call_is_research(&graph));
}

#[test]
fn delegated_build_policy_blocks_supervised_write_without_approval_path() {
    let policy = DelegationPolicy::for_subagent_build(PermissionMode::Supervised, Some(10));
    assert!(policy.authorize_tool("read", false).is_ok());
    assert!(policy.authorize_tool("write", false).is_err());
}

#[test]
fn delegated_policy_accepts_only_the_approved_capability_ceiling() {
    let policy = DelegationPolicy::for_subagent_child(
        PermissionMode::Supervised,
        Some(10),
        false,
        false,
        true,
    )
    .with_supervised_approval(true);

    assert!(policy.authorize_tool("bash", false).is_ok());
    assert!(policy.authorize_tool("write", false).is_err());
}

#[test]
fn delegated_build_policy_allows_autonomous_write() {
    let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, Some(10));
    assert!(policy.authorize_tool("write", false).is_ok());
    assert!(policy.authorize_tool("bash", false).is_ok());
}

#[test]
fn delegated_verify_bash_allowance_follows_effective_deferred_target() {
    let policy = DelegationPolicy::for_subagent_verify(PermissionMode::Autonomous, Some(10));

    assert!(policy.authorize_tool("bash", false).is_ok());
    assert!(policy
        .authorize_tool_call(
            "tool_search",
            &json!({
                "action": "execute",
                "tool": "bash",
                "arguments": {"command": "cargo test"}
            }),
            false,
        )
        .is_ok());
    assert!(policy
        .authorize_tool_call(
            "tool_search",
            &json!({
                "action": "execute",
                "tool": "edit",
                "arguments": {"file_path": "src/lib.rs"}
            }),
            false,
        )
        .is_err());
}

#[test]
fn skill_tool_is_classified_as_read_only() {
    let policy = tool_policy("skill");
    assert_eq!(policy.category, ToolCategory::ReadOnly);
    assert!(policy.allowed_in_plan_mode);
    assert!(!policy.requires_supervised_approval);
}

#[tokio::test]
async fn test_tool_result_success_data_with_envelope_fields() {
    let result = ToolResult::success_data_with(
        json!({"message": "ok"}),
        vec!["warn".to_string()],
        Some("diff body".to_string()),
        Some(json!({"exit_code": 0})),
    );

    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["data"]["message"], "ok");
    assert_eq!(parsed["warnings"][0], "warn");
    assert_eq!(parsed["diff"], "diff body");
    assert_eq!(parsed["metadata"]["exit_code"], 0);
}

#[test]
fn tool_result_changed_is_a_structured_producer_contract() {
    let changed = ToolResult::success_data(json!({"message": "written"})).with_changed(true);
    let parsed: serde_json::Value = serde_json::from_str(&changed.output).unwrap();
    assert_eq!(parsed["changed"], true);

    let unchanged = ToolResult::success("plain output").with_changed(false);
    let parsed: serde_json::Value = serde_json::from_str(&unchanged.output).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["changed"], false);
    assert_eq!(parsed["data"], "plain output");
}

#[tokio::test]
async fn test_tool_result_error_with_details_includes_data_and_metadata() {
    let result = ToolResult::error_with_details(
        "command_failed",
        "Command exited",
        Some(json!({"output": "stderr"})),
        Some(json!({"exit_code": 1})),
    );

    assert!(result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["error"]["code"], "command_failed");
    assert_eq!(parsed["error"]["message"], "Command exited");
    assert_eq!(parsed["data"]["output"], "stderr");
    assert_eq!(parsed["metadata"]["exit_code"], 1);
}

#[tokio::test]
async fn test_parse_params_success() {
    #[derive(serde::Deserialize)]
    struct TestParams {
        name: String,
        count: i32,
    }

    let params = json!({"name": "test", "count": 42});
    let result: Result<TestParams, ToolResult> = parse_params(params);

    assert!(result.is_ok());
    let parsed = result.unwrap();
    assert_eq!(parsed.name, "test");
    assert_eq!(parsed.count, 42);
}

#[tokio::test]
async fn test_parse_params_accepts_integral_floats_for_integer_fields() {
    #[derive(serde::Deserialize)]
    struct TestParams {
        depth: Option<usize>,
        timeout: u64,
        nested: NestedParams,
    }

    #[derive(serde::Deserialize)]
    struct NestedParams {
        limit: usize,
    }

    let params = json!({
        "depth": 2.0,
        "timeout": 600000.0,
        "nested": { "limit": 5.0 }
    });
    let parsed: TestParams = parse_params(params).expect("integral floats should deserialize");

    assert_eq!(parsed.depth, Some(2));
    assert_eq!(parsed.timeout, 600_000);
    assert_eq!(parsed.nested.limit, 5);
}

#[tokio::test]
async fn test_parse_params_rejects_fractional_values_for_integer_fields() {
    #[derive(Debug, serde::Deserialize)]
    struct TestParams {
        #[serde(rename = "count")]
        _count: usize,
    }

    let result: Result<TestParams, ToolResult> = parse_params(json!({"count": 2.5}));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_error);
    assert!(err.output.contains("Invalid parameters"));
}

#[tokio::test]
async fn test_parse_params_invalid_json() {
    #[derive(serde::Deserialize, Debug)]
    struct TestParams {
        #[serde(rename = "name")]
        _name: String,
    }

    let params = json!({"name": 123});
    let result: Result<TestParams, ToolResult> = parse_params(params);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_error);
    assert!(err.output.contains("Invalid parameters"));
    let parsed: serde_json::Value = serde_json::from_str(&err.output).unwrap();
    assert_eq!(parsed["error"]["code"], "invalid_parameters");
}

#[test]
fn test_sandboxed_resolve_new_path_rejects_traversal() {
    let ctx = ToolContext {
        working_dir: PathBuf::from("/sandbox/project"),
        sandbox_root: Some(PathBuf::from("/sandbox")),
        ..Default::default()
    };

    let result = ctx.sandboxed_resolve_new_path("../../../etc/passwd");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("traversal"));

    let result = ctx.sandboxed_resolve_new_path("subdir/../../../etc/passwd");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("traversal"));
}

#[test]
fn test_sandboxed_resolve_new_path_allows_valid_paths() {
    let ctx = ToolContext {
        working_dir: PathBuf::from("/tmp"),
        sandbox_root: Some(PathBuf::from("/tmp")),
        ..Default::default()
    };

    let result = ctx.sandboxed_resolve_new_path("newfile.txt");
    assert!(result.is_ok());

    let result = ctx.sandboxed_resolve_new_path("subdir/nested/file.txt");
    assert!(result.is_ok());
}

#[test]
fn test_sandboxed_resolve_new_path_no_sandbox() {
    let ctx = ToolContext {
        working_dir: PathBuf::from("/home/user"),
        sandbox_root: None,
        ..Default::default()
    };

    let result = ctx.sandboxed_resolve_new_path("../other/file.txt");
    assert!(result.is_ok());
}

struct TestTool;

struct SlowBashLifecycleTool;

struct LeaseHoldingTool {
    db_path: PathBuf,
}

#[async_trait]
impl Tool for LeaseHoldingTool {
    fn name(&self) -> &str {
        "lease_holding_test"
    }

    fn description(&self) -> &str {
        "test durable delegated-run cancellation on registry timeout"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        use crate::agent::DelegatedRunStage;
        use crate::storage::{
            Database, DelegatedRunLease, DelegatedRunRole, DelegatedRunScope,
            DelegatedRunStartInput, DelegatedRunStore, DelegationCompletionPolicy,
            DelegationExecutionMode, DelegationFailurePolicy, DelegationGovernance,
            DelegationGroupContract, DelegationGroupStartInput, DelegationStore,
            DelegationTaskSpec, DelegationWriterMode,
        };

        let store = DelegatedRunStore::new(Database::new(&self.db_path).expect("lease database"));
        let mut lease = DelegatedRunLease::new(store);
        lease
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: "registry-timeout-run".to_string(),
                parent_session_id: "registry-timeout-session".to_string(),
                parent_tool_call_id: Some("registry-timeout-call".to_string()),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Running,
                provider: None,
                model: None,
                resumable: true,
                resumed_from_run_id: None,
                target_scope: vec![DelegatedRunScope {
                    label: "workspace".to_string(),
                    path: ".".to_string(),
                    kind: "workspace".to_string(),
                }],
            })
            .expect("create timeout run");

        let policy = DelegationPolicy::for_subagent_explore(PermissionMode::Autonomous, None);
        let group_store = DelegationStore::new(
            Database::new(&self.db_path).expect("timeout delegation-group database"),
        );
        group_store
            .create_group(&DelegationGroupStartInput {
                delegation_group_id: "registry-timeout-run".to_string(),
                parent_session_id: "registry-timeout-session".to_string(),
                parent_tool_call_id: Some("registry-timeout-call".to_string()),
                contract: DelegationGroupContract {
                    execution_mode: DelegationExecutionMode::Foreground,
                    completion_policy: DelegationCompletionPolicy::AllSettled,
                    failure_policy: DelegationFailurePolicy::Continue,
                    governance: DelegationGovernance {
                        permission_mode: PermissionMode::Autonomous,
                        reasoning_effort: None,
                        delegated_turn_budget: None,
                        max_parallelism: 1,
                        execution_tool_allowlist: policy.execution_tool_allowlist.clone(),
                        delegation_policy: policy.clone(),
                    },
                },
                tasks: vec![DelegationTaskSpec {
                    delegation_task_id: "registry-timeout-task".to_string(),
                    task_key: "timeout".to_string(),
                    objective: "remain active until the registry timeout".to_string(),
                    role: DelegatedRunRole::Explore,
                    target_scope: vec![],
                    max_attempts: 2,
                    depends_on: vec![],
                    write_intent: vec![],
                    task_policy: Some(policy),
                    writer_mode: DelegationWriterMode::Shared,
                    attempt_workspace: None,
                    workspace_baseline: None,
                    executor_envelope: None,
                }],
            })
            .expect("create timeout group");
        group_store
            .queue_group("registry-timeout-run")
            .expect("queue timeout group");
        group_store
            .claim_task("registry-timeout-task", "timeout-owner", 60_000)
            .expect("claim timeout task")
            .expect("timeout task lease");
        assert!(group_store
            .mark_task_running("registry-timeout-task", "timeout-owner", "provider/model",)
            .expect("start timeout task"));
        std::future::pending::<()>().await;
        ToolResult::success("unreachable")
    }
}

#[async_trait]
impl Tool for SlowBashLifecycleTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "test Bash lifecycle"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        ToolResult::success("cleanup complete")
    }
}

#[async_trait]
impl Tool for TestTool {
    fn name(&self) -> &str {
        "test_tool"
    }

    fn description(&self) -> &str {
        "Test tool"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success("{}")
    }
}

struct AlwaysBlockHook;

#[async_trait]
impl PreToolHook for AlwaysBlockHook {
    async fn before_execute(&self, _name: &str, _params: &Value, _ctx: &ToolContext) -> HookResult {
        HookResult::Block {
            reason: "blocked for test".to_string(),
        }
    }
}

#[tokio::test]
async fn test_pre_hook_block_returns_structured_json_error() {
    let mut registry = ToolRegistry::new();
    registry.add_pre_hook(Arc::new(AlwaysBlockHook));
    registry.register(Arc::new(TestTool)).await;
    let ctx = create_test_context();

    let result = registry
        .execute("test_tool", json!({}), &ctx)
        .await
        .unwrap();

    assert!(result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(parsed["error"]["code"], "blocked_by_policy");
    assert_eq!(parsed["error"]["message"], "blocked for test");
    assert_eq!(parsed["error"]["retryable"], false);
    assert!(parsed["error"]["next_action"]
        .as_str()
        .is_some_and(|value| value.contains("satisfy the reported policy boundary")));
}

#[test]
fn public_preview_policy_error_provides_one_safe_recovery_path() {
    let result = super::runtime::policy_block_result(
        "bash",
        &json!({"command": "python -m http.server --bind 0.0.0.0"}),
        "preview servers must bind explicitly to 127.0.0.1 or localhost".to_string(),
    );
    let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(parsed["error"]["retryable"], false);
    assert_eq!(
        parsed["error"]["safe_alternative"]["bind_address"],
        "127.0.0.1"
    );
    assert_eq!(
        parsed["error"]["safe_alternative"]["exposure"],
        "tailscale_serve"
    );
}

#[test]
fn bash_requested_timeout_extends_registry_guard_through_cleanup() {
    let timeout = execution_timeout_for_call(
        "bash",
        &json!({"command": "cargo test", "timeout": 600_000}),
        None,
        None,
        std::time::Duration::from_secs(120),
    );

    assert_eq!(timeout, std::time::Duration::from_secs(625));
}

#[test]
fn foreground_agent_run_uses_lifecycle_instead_of_generic_outer_timeout() {
    use super::runtime::should_apply_outer_timeout;

    assert!(!should_apply_outer_timeout(
        "agent",
        &json!({"tasks": [{"id": "build", "instructions": "work"}]}),
        None,
    ));
    assert!(!should_apply_outer_timeout(
        "agent",
        &json!({"action": "resume", "delegated_run_id": "run-1"}),
        None,
    ));
    assert!(should_apply_outer_timeout(
        "agent",
        &json!({"run_in_background": true, "prompt": "work"}),
        None,
    ));
    assert!(should_apply_outer_timeout(
        "agent",
        &json!({"action": "wait", "delegated_run_id": "run-1"}),
        None,
    ));
    assert!(should_apply_outer_timeout(
        "agent",
        &json!({"prompt": "bounded work"}),
        Some(std::time::Duration::from_secs(30)),
    ));
}

#[test]
fn bash_short_timeout_keeps_larger_registry_guard_without_waiting() {
    let timeout = execution_timeout_for_call(
        "bash",
        &json!({"command": "sleep 1", "timeout": 5}),
        None,
        None,
        std::time::Duration::from_secs(120),
    );

    // Bash still enforces the requested 5ms internally. The registry guard must
    // stay out of the way so Bash can terminate the process group and drain pipes.
    assert_eq!(timeout, std::time::Duration::from_secs(120));
}

#[tokio::test]
async fn registry_short_bash_timeout_leaves_room_for_inner_cleanup() {
    let registry = ToolRegistry::with_default_timeout(std::time::Duration::from_millis(1));
    registry.register(Arc::new(SlowBashLifecycleTool)).await;

    let result = registry
        .execute(
            "bash",
            json!({"command": "test", "timeout": 5}),
            &create_test_context(),
        )
        .await
        .expect("test Bash tool should be registered");

    assert!(!result.is_error, "{}", result.output);
    assert_eq!(result.output, "cleanup complete");
}

#[tokio::test]
async fn registry_timeout_drops_lease_and_cancels_durable_run() {
    use crate::agent::DelegatedRunStage;
    use crate::storage::{
        Database, DelegatedRunStore, DelegationGroupState, DelegationStore, DelegationTaskState,
    };
    use rusqlite::params;
    use tempfile::TempDir;

    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("registry-timeout.db");
    let db = Database::new(&db_path).expect("timeout database");
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params!["registry-timeout-session", "Registry Timeout", now, now],
        )
        .expect("seed parent session");
    drop(db);

    let registry = ToolRegistry::with_default_timeout(std::time::Duration::from_millis(1));
    registry
        .register(Arc::new(LeaseHoldingTool {
            db_path: db_path.clone(),
        }))
        .await;

    let result = registry
        .execute("lease_holding_test", json!({}), &create_test_context())
        .await
        .expect("lease test tool should be registered");
    assert!(result.is_error);
    assert_eq!(
        serde_json::from_str::<Value>(&result.output).unwrap()["error"]["code"],
        "timeout"
    );

    let store = DelegatedRunStore::new(Database::new(&db_path).expect("reopen timeout database"));
    let record = store
        .get_run("registry-timeout-run")
        .expect("load timed-out run")
        .expect("timed-out run exists");
    assert_eq!(record.stage, DelegatedRunStage::Cancelled);
    assert_eq!(
        record.artifact.unwrap()["outcome_reason"],
        "caller_aborted_before_terminal"
    );

    let group_store =
        DelegationStore::new(Database::new(&db_path).expect("reopen timeout group database"));
    let group = group_store
        .get_group("registry-timeout-run")
        .expect("load timed-out group")
        .expect("timed-out group exists");
    assert_eq!(group.state, DelegationGroupState::Cancelled);
    assert_eq!(group.tasks[0].state, DelegationTaskState::Cancelled);
    assert!(group_store
        .claim_task("registry-timeout-task", "late-owner", 60_000)
        .expect("cancelled task remains readable")
        .is_none());

    let attempt_db = Database::new(&db_path).expect("reopen timeout attempt database");
    let (task_owner, task_expiry, attempt_state): (Option<String>, Option<i64>, String) =
        attempt_db
            .conn()
            .query_row(
                "SELECT tasks.lease_owner_id, tasks.lease_expires_at_ms, attempts.state
               FROM delegation_tasks AS tasks
               JOIN delegation_attempts AS attempts
                 ON attempts.delegation_task_id = tasks.delegation_task_id
              WHERE tasks.delegation_task_id = ?1",
                params!["registry-timeout-task"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("load timed-out task and attempt");
    assert!(task_owner.is_none());
    assert!(task_expiry.is_none());
    assert_eq!(attempt_state, "cancelled");
}

#[test]
fn bash_timeout_resolution_clamps_provider_float_and_preserves_explicit_longer_guard() {
    let timeout = execution_timeout_for_call(
        "bash",
        &json!({"command": "cargo test", "timeout": 900_000.0}),
        Some(std::time::Duration::from_secs(700)),
        None,
        std::time::Duration::from_secs(120),
    );

    assert_eq!(timeout, std::time::Duration::from_secs(700));
}

#[test]
fn mcp_capability_discovery_tools_are_read_only_but_dynamic_tools_remain_conservative() {
    for name in [
        "mcp__list_resources",
        "mcp__list_resource_templates",
        "mcp__read_resource",
        "mcp__list_prompts",
        "mcp__get_prompt",
        "mcp__list_tools",
    ] {
        assert_eq!(tool_category(name), ToolCategory::ReadOnly, "{name}");
    }
    assert_eq!(
        tool_category("mcp__github__create_issue"),
        ToolCategory::Write
    );
}
