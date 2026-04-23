use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::hooks::{HookResult, PreToolHook};

use super::policy::DELEGATED_TOOL_TIMEOUT;
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
}

#[test]
fn delegated_explore_policy_blocks_write_tools() {
    let policy = DelegationPolicy::for_subagent_explore(PermissionMode::Autonomous, Some(20));
    assert!(policy.authorize_tool("read", false).is_ok());
    assert!(policy.authorize_tool("edit", false).is_err());
}

#[test]
fn delegated_build_policy_blocks_supervised_write_without_approval_path() {
    let policy = DelegationPolicy::for_subagent_build(PermissionMode::Supervised, Some(10));
    assert!(policy.authorize_tool("read", false).is_ok());
    assert!(policy.authorize_tool("write", false).is_err());
}

#[test]
fn delegated_build_policy_allows_autonomous_write() {
    let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, Some(10));
    assert!(policy.authorize_tool("write", false).is_ok());
    assert!(policy.authorize_tool("bash", false).is_ok());
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
}
