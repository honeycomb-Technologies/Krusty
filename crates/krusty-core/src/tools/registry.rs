//! Tool registry for managing available tools
//!
//! Supports pre/post execution hooks for logging, validation, and safety.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

use serde::{Deserialize, Serialize};

use crate::agent::hooks::{HookResult, PostToolHook, PreToolHook};
use crate::agent::subagent::AgentProgress;
use crate::ai::types::AiTool;
use crate::mcp::McpManager;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::tools::git_identity::GitIdentity;

/// Tool category for permission checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    /// Read-only tools that never modify state.
    ReadOnly,
    /// Write tools that modify files, execute commands, etc.
    Write,
    /// Interactive tools that require user input.
    Interactive,
}

/// Permission mode for tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    #[default]
    Supervised,
    Autonomous,
}

/// Categorize a tool by name.
pub fn tool_category(name: &str) -> ToolCategory {
    match name {
        "read" | "glob" | "grep" | "list" | "web_search" | "web_fetch" | "explore" => {
            ToolCategory::ReadOnly
        }
        "AskUserQuestion" | "PlanConfirm" | "enter_plan_mode" | "set_work_mode" | "task_start"
        | "task_complete" | "add_subtask" | "set_dependency" => ToolCategory::Interactive,
        _ => ToolCategory::Write,
    }
}

/// Default tool execution timeout (2 minutes)
const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// Tool execution result
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

impl ToolResult {
    /// Create a success result
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
        }
    }

    /// Create a structured success envelope with `ok=true` and `data`.
    pub fn success_data(data: Value) -> Self {
        Self::success_data_with(data, Vec::new(), None, None)
    }

    /// Create a structured success envelope with optional warnings/diff/metadata.
    pub fn success_data_with(
        data: Value,
        warnings: Vec<String>,
        diff: Option<String>,
        metadata: Option<Value>,
    ) -> Self {
        let mut envelope = serde_json::Map::new();
        envelope.insert("ok".to_string(), Value::Bool(true));
        envelope.insert("data".to_string(), data);

        if !warnings.is_empty() {
            envelope.insert(
                "warnings".to_string(),
                Value::Array(warnings.into_iter().map(Value::String).collect()),
            );
        }

        if let Some(diff) = diff.filter(|d| !d.is_empty()) {
            envelope.insert("diff".to_string(), Value::String(diff));
        }

        if let Some(metadata) = metadata {
            envelope.insert("metadata".to_string(), metadata);
        }

        Self {
            output: Value::Object(envelope).to_string(),
            is_error: false,
        }
    }

    /// Create a structured error with explicit code.
    pub fn error_with_code(code: &str, msg: impl std::fmt::Display) -> Self {
        Self::error_with_details(code, msg, None, None)
    }

    /// Create a structured error envelope with optional data/metadata.
    pub fn error_with_details(
        code: &str,
        msg: impl std::fmt::Display,
        data: Option<Value>,
        metadata: Option<Value>,
    ) -> Self {
        let mut envelope = serde_json::Map::new();
        envelope.insert("ok".to_string(), Value::Bool(false));
        envelope.insert(
            "error".to_string(),
            serde_json::json!({
                "code": code,
                "message": msg.to_string()
            }),
        );

        if let Some(data) = data {
            envelope.insert("data".to_string(), data);
        }

        if let Some(metadata) = metadata {
            envelope.insert("metadata".to_string(), metadata);
        }

        Self {
            output: Value::Object(envelope).to_string(),
            is_error: true,
        }
    }

    /// Create an invalid-parameters error.
    pub fn invalid_parameters(msg: impl std::fmt::Display) -> Self {
        Self::error_with_code("invalid_parameters", msg)
    }

    /// Create an error result with JSON-formatted error message
    pub fn error(msg: impl std::fmt::Display) -> Self {
        let message = msg.to_string();
        let code = classify_error_code(&message);
        Self::error_with_details(code, message, None, None)
    }
}

/// Parse tool parameters, returning a ToolResult error on failure
pub fn parse_params<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ToolResult> {
    serde_json::from_value(params)
        .map_err(|e| ToolResult::invalid_parameters(format!("Invalid parameters: {}", e)))
}

fn classify_error_code(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("invalid parameters")
        || lower.contains("missing field")
        || lower.contains("unknown field")
    {
        "invalid_parameters"
    } else if lower.contains("access denied") || lower.contains("outside workspace") {
        "access_denied"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if lower.contains("denied") {
        "permission_denied"
    } else if lower.contains("unknown tool") {
        "unknown_tool"
    } else {
        "tool_error"
    }
}

/// Output chunk from a streaming tool (like bash)
#[derive(Debug, Clone)]
pub struct ToolOutputChunk {
    pub tool_use_id: String,
    pub chunk: String,
    pub is_complete: bool,
    pub exit_code: Option<i32>,
}

/// Context for tool execution
pub struct ToolContext {
    pub working_dir: std::path::PathBuf,
    /// Sandbox root for multi-tenant path isolation (e.g., /workspaces/{user_id})
    /// If set, all file operations must be within this directory.
    pub sandbox_root: Option<std::path::PathBuf>,
    /// User ID for multi-tenant operation scoping (processes, etc.)
    pub user_id: Option<String>,
    pub process_registry: Option<Arc<ProcessRegistry>>,
    pub skills_manager: Option<Arc<RwLock<SkillsManager>>>,
    pub mcp_manager: Option<Arc<McpManager>>,
    /// Optional per-call timeout override
    pub timeout: Option<Duration>,
    /// Channel for streaming output (used by bash tool)
    pub output_tx: Option<mpsc::UnboundedSender<ToolOutputChunk>>,
    /// Tool use ID for streaming output
    pub tool_use_id: Option<String>,
    /// Whether plan mode is active (restricts write tools)
    pub plan_mode: bool,
    /// Channel for explore tool sub-agent progress updates
    pub explore_progress_tx: Option<mpsc::UnboundedSender<AgentProgress>>,
    /// Channel for build tool builder agent progress updates
    pub build_progress_tx: Option<mpsc::UnboundedSender<AgentProgress>>,
    /// Current user-selected model (for non-Anthropic providers, subagents use this)
    pub current_model: Option<String>,
    /// Git identity for commit attribution
    pub git_identity: Option<GitIdentity>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            sandbox_root: None,
            user_id: None,
            process_registry: None,
            skills_manager: None,
            mcp_manager: None,
            timeout: None,
            output_tx: None,
            tool_use_id: None,
            plan_mode: false,
            explore_progress_tx: None,
            build_progress_tx: None,
            current_model: None,
            git_identity: None,
        }
    }
}

impl ToolContext {
    /// Create a new tool context with process registry
    pub fn with_process_registry(
        working_dir: std::path::PathBuf,
        process_registry: Arc<ProcessRegistry>,
    ) -> Self {
        Self {
            working_dir,
            process_registry: Some(process_registry),
            ..Default::default()
        }
    }

    /// Set sandbox root for multi-tenant path isolation.
    pub fn with_sandbox(mut self, sandbox_root: std::path::PathBuf) -> Self {
        self.sandbox_root = Some(sandbox_root);
        self
    }

    /// Set user ID for multi-tenant operation scoping.
    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Add MCP manager to context
    pub fn with_mcp_manager(mut self, mcp_manager: Arc<McpManager>) -> Self {
        self.mcp_manager = Some(mcp_manager);
        self
    }

    /// Add skills manager to context
    pub fn with_skills_manager(mut self, skills_manager: Arc<RwLock<SkillsManager>>) -> Self {
        self.skills_manager = Some(skills_manager);
        self
    }

    /// Add streaming output channel to context
    pub fn with_output_stream(
        mut self,
        tx: mpsc::UnboundedSender<ToolOutputChunk>,
        tool_use_id: String,
    ) -> Self {
        self.output_tx = Some(tx);
        self.tool_use_id = Some(tool_use_id);
        self
    }

    /// Add explore progress channel to context
    pub fn with_explore_progress(mut self, tx: mpsc::UnboundedSender<AgentProgress>) -> Self {
        self.explore_progress_tx = Some(tx);
        self
    }

    /// Add build progress channel to context
    pub fn with_build_progress(mut self, tx: mpsc::UnboundedSender<AgentProgress>) -> Self {
        self.build_progress_tx = Some(tx);
        self
    }

    /// Set the current user-selected model (for non-Anthropic provider subagents)
    pub fn with_current_model(mut self, model: String) -> Self {
        self.current_model = Some(model);
        self
    }

    /// Set git identity for commit attribution
    pub fn with_git_identity(mut self, identity: GitIdentity) -> Self {
        self.git_identity = Some(identity);
        self
    }

    /// Resolve a path relative to working directory (absolute paths pass through)
    pub fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = std::path::PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            self.working_dir.join(p)
        }
    }

    /// Resolve a path with sandbox enforcement for multi-tenant isolation.
    ///
    /// If sandbox_root is set, ensures the resolved path is within the sandbox.
    /// Returns an error if the path escapes the sandbox via symlinks or `..`.
    pub fn sandboxed_resolve(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let resolved = self.resolve_path(path);

        // If no sandbox, allow everything (single-tenant mode)
        let Some(ref sandbox) = self.sandbox_root else {
            return Ok(resolved);
        };

        // Canonicalize to resolve symlinks and `..`
        let canonical = resolved
            .canonicalize()
            .map_err(|e| format!("Invalid path '{}': {}", path, e))?;

        // Check if the canonical path is within the sandbox
        if !canonical.starts_with(sandbox) {
            return Err(format!(
                "Access denied: path '{}' is outside workspace",
                path
            ));
        }

        Ok(canonical)
    }

    /// Check if a path is within the sandbox (for validation without resolving).
    pub fn is_path_allowed(&self, path: &std::path::Path) -> bool {
        let Some(ref sandbox) = self.sandbox_root else {
            return true;
        };

        // Try to canonicalize, default to false if it fails
        path.canonicalize()
            .map(|p| p.starts_with(sandbox))
            .unwrap_or(false)
    }

    /// Resolve a path that may not exist yet (for write operations) with sandbox enforcement.
    ///
    /// Unlike `sandboxed_resolve`, this handles paths where parent directories don't exist yet.
    /// It finds the nearest existing ancestor, canonicalizes it, validates it's within sandbox,
    /// then appends the remaining path components (which are verified to not contain traversal).
    pub fn sandboxed_resolve_new_path(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let resolved = self.resolve_path(path);

        let Some(ref sandbox) = self.sandbox_root else {
            return Ok(resolved);
        };

        // Reject any path with traversal components - this is the key security fix
        for component in resolved.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err("Path traversal (..) not allowed".into());
            }
        }

        // If path exists, just canonicalize and check
        if resolved.exists() {
            let canonical = resolved
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e))?;
            if !canonical.starts_with(sandbox) {
                return Err("Access denied: path is outside workspace".into());
            }
            return Ok(canonical);
        }

        // Find nearest existing ancestor and canonicalize it
        let mut check = resolved;
        let mut suffix: Vec<std::ffi::OsString> = Vec::new();

        while !check.exists() {
            if let Some(name) = check.file_name() {
                suffix.push(name.to_owned());
            }
            if !check.pop() {
                break;
            }
        }

        // check is now the nearest existing ancestor (or empty)
        let canonical_base = if check.as_os_str().is_empty() || !check.exists() {
            // No existing ancestor found - use sandbox root as base for validation
            sandbox.clone()
        } else {
            check
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e))?
        };

        if !canonical_base.starts_with(sandbox) {
            return Err("Access denied: path is outside workspace".into());
        }

        // Rebuild path with canonical base + remaining components
        let mut final_path = canonical_base;
        for component in suffix.into_iter().rev() {
            final_path.push(component);
        }

        Ok(final_path)
    }
}

/// Trait for tool implementations
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (id)
    fn name(&self) -> &str;

    /// Tool description for AI
    fn description(&self) -> &str;

    /// JSON schema for parameters
    fn parameters_schema(&self) -> Value;

    /// Execute the tool
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult;
}

/// Registry for managing tools with hook support
pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    /// Default timeout for tool execution
    default_timeout: Duration,
    /// Pre-execution hooks (run before each tool)
    pre_hooks: Vec<Arc<dyn PreToolHook>>,
    /// Post-execution hooks (run after each tool)
    post_hooks: Vec<Arc<dyn PostToolHook>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            default_timeout: DEFAULT_TOOL_TIMEOUT,
            pre_hooks: Vec::new(),
            post_hooks: Vec::new(),
        }
    }

    /// Register a tool
    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let mut tools = self.tools.write().await;
        tools.insert(name, tool);
    }

    /// Add a pre-execution hook
    pub fn add_pre_hook(&mut self, hook: Arc<dyn PreToolHook>) {
        self.pre_hooks.push(hook);
    }

    /// Add a post-execution hook
    pub fn add_post_hook(&mut self, hook: Arc<dyn PostToolHook>) {
        self.post_hooks.push(hook);
    }

    /// Get a tool by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    /// Get all tools as AI tool definitions, sorted by name.
    ///
    /// Deterministic ordering is critical for prompt caching — tool definitions
    /// are part of the cached prefix, and non-deterministic order (from HashMap
    /// iteration) silently breaks the cache between API calls.
    pub async fn get_ai_tools(&self) -> Vec<AiTool> {
        let tools = self.tools.read().await;
        let mut ai_tools: Vec<AiTool> = tools
            .values()
            .map(|t| AiTool {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.parameters_schema(),
            })
            .collect();
        ai_tools.sort_by(|a, b| a.name.cmp(&b.name));
        ai_tools
    }

    /// Unregister all tools with names starting with the given prefix
    pub async fn unregister_by_prefix(&self, prefix: &str) {
        let mut tools = self.tools.write().await;
        let to_remove: Vec<String> = tools
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect();

        for name in to_remove {
            tools.remove(&name);
            tracing::debug!("Unregistered tool: {}", name);
        }
    }

    /// Execute a tool by name with hooks and timeout
    pub async fn execute(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        tracing::info!(tool = name, "ToolRegistry: execute called");
        let tool = self.get(name).await?;
        tracing::info!(tool = name, "ToolRegistry: tool found, executing");
        let timeout = ctx.timeout.unwrap_or(self.default_timeout);
        let start = Instant::now();

        // Run pre-hooks - they can block execution
        for hook in &self.pre_hooks {
            match hook.before_execute(name, &params, ctx).await {
                HookResult::Continue => {}
                HookResult::Block { reason } => {
                    tracing::info!(tool = name, reason = %reason, "Pre-hook blocked execution");
                    return Some(ToolResult::error_with_code("blocked_by_policy", reason));
                }
            }
        }

        // Execute the tool with timeout
        let result = match tokio::time::timeout(timeout, tool.execute(params.clone(), ctx)).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(
                    tool = name,
                    timeout_secs = timeout.as_secs(),
                    "Tool execution timed out"
                );
                ToolResult::error_with_code(
                    "timeout",
                    format!(
                        "Tool '{}' timed out after {} seconds",
                        name,
                        timeout.as_secs()
                    ),
                )
            }
        };

        let duration = start.elapsed();

        // Run post-hooks - they can inspect/log but we don't modify results (yet)
        for hook in &self.post_hooks {
            let _ = hook.after_execute(name, &params, &result, duration).await;
        }

        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::hooks::{HookResult, PreToolHook};
    use async_trait::async_trait;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;

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

        let params = json!({"name": 123}); // Wrong type
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

        // Direct traversal attempt should be rejected
        let result = ctx.sandboxed_resolve_new_path("../../../etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("traversal"));

        // Traversal in middle of path should be rejected
        let result = ctx.sandboxed_resolve_new_path("subdir/../../../etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("traversal"));
    }

    #[test]
    fn test_sandboxed_resolve_new_path_allows_valid_paths() {
        // Use /tmp which always exists
        let ctx = ToolContext {
            working_dir: PathBuf::from("/tmp"),
            sandbox_root: Some(PathBuf::from("/tmp")),
            ..Default::default()
        };

        // Valid relative path within sandbox
        let result = ctx.sandboxed_resolve_new_path("newfile.txt");
        assert!(result.is_ok());

        // Valid nested path within sandbox (parent exists: /tmp)
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

        // Without sandbox, any path should be allowed (including traversal)
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
        async fn before_execute(
            &self,
            _name: &str,
            _params: &Value,
            _ctx: &ToolContext,
        ) -> HookResult {
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
}
