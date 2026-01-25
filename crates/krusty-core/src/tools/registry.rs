//! Tool registry for managing available tools
//!
//! Supports pre/post execution hooks for logging, validation, and safety.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

use crate::agent::hooks::{HookResult, PostToolHook, PreToolHook};
use crate::agent::subagent::AgentProgress;
use crate::ai::types::AiTool;
use crate::lsp::manager::MissingLspInfo;
use crate::lsp::LspManager;
use crate::mcp::McpManager;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;

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

    /// Create an error result with JSON-formatted error message
    pub fn error(msg: impl std::fmt::Display) -> Self {
        Self {
            output: serde_json::json!({"error": msg.to_string()}).to_string(),
            is_error: true,
        }
    }
}

/// Parse tool parameters, returning a ToolResult error on failure
pub fn parse_params<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ToolResult> {
    serde_json::from_value(params)
        .map_err(|e| ToolResult::error(format!("Invalid parameters: {}", e)))
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
    pub lsp_manager: Option<Arc<LspManager>>,
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
    /// Channel for signaling missing LSP to prompt user for installation
    pub missing_lsp_tx: Option<mpsc::UnboundedSender<MissingLspInfo>>,
    /// Current user-selected model (for non-Anthropic providers, subagents use this)
    pub current_model: Option<String>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            sandbox_root: None,
            user_id: None,
            lsp_manager: None,
            process_registry: None,
            skills_manager: None,
            mcp_manager: None,
            timeout: None,
            output_tx: None,
            tool_use_id: None,
            plan_mode: false,
            explore_progress_tx: None,
            build_progress_tx: None,
            missing_lsp_tx: None,
            current_model: None,
        }
    }
}

impl ToolContext {
    /// Create a new tool context with LSP manager and process registry
    pub fn with_lsp_and_processes(
        working_dir: std::path::PathBuf,
        lsp_manager: Arc<LspManager>,
        process_registry: Arc<ProcessRegistry>,
    ) -> Self {
        Self {
            working_dir,
            sandbox_root: None,
            user_id: None,
            lsp_manager: Some(lsp_manager),
            process_registry: Some(process_registry),
            skills_manager: None,
            mcp_manager: None,
            timeout: None,
            output_tx: None,
            tool_use_id: None,
            plan_mode: false,
            explore_progress_tx: None,
            build_progress_tx: None,
            missing_lsp_tx: None,
            current_model: None,
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

    /// Add missing LSP notification channel to context
    pub fn with_missing_lsp_channel(mut self, tx: mpsc::UnboundedSender<MissingLspInfo>) -> Self {
        self.missing_lsp_tx = Some(tx);
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

    /// Notify LSP about a file change and optionally get diagnostics
    ///
    /// After modifying a file, triggers LSP analysis and waits briefly
    /// for diagnostics to be updated.
    ///
    /// If no LSP is available for this file type and a suggestion exists,
    /// sends the info through the missing_lsp_tx channel for user prompting.
    pub async fn touch_file(&self, path: &std::path::Path, wait_for_diagnostics: bool) {
        if let Some(ref lsp) = self.lsp_manager {
            match lsp.touch_file(path, wait_for_diagnostics).await {
                Ok(Some(missing)) => {
                    // No LSP for this file - send through channel for user prompting
                    if let Some(ref tx) = self.missing_lsp_tx {
                        let _ = tx.send(missing);
                    }
                }
                Ok(None) => {
                    // LSP handled the file or no suggestion available
                }
                Err(e) => {
                    tracing::warn!("Failed to touch file {:?}: {}", path, e);
                }
            }
        }
    }

    /// Get formatted diagnostics for a file
    ///
    /// Returns XML-formatted diagnostics block if the file has errors.
    pub fn get_file_diagnostics(&self, path: &std::path::Path) -> Option<String> {
        self.lsp_manager.as_ref()?.get_file_diagnostics(path)
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

    /// Get all tools as AI tool definitions
    pub async fn get_ai_tools(&self) -> Vec<AiTool> {
        let tools = self.tools.read().await;
        tools
            .values()
            .map(|t| AiTool {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.parameters_schema(),
            })
            .collect()
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
                    return Some(ToolResult {
                        output: format!("Blocked: {}", reason),
                        is_error: true,
                    });
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
                ToolResult {
                    output: format!(
                        "Tool '{}' timed out after {} seconds",
                        name,
                        timeout.as_secs()
                    ),
                    is_error: true,
                }
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
    use serde_json::json;
    use std::path::PathBuf;

    fn create_test_context() -> ToolContext {
        ToolContext {
            working_dir: PathBuf::from("/tmp"),
            sandbox_root: None,
            user_id: None,
            lsp_manager: None,
            process_registry: None,
            skills_manager: None,
            mcp_manager: None,
            timeout: None,
            output_tx: None,
            tool_use_id: None,
            plan_mode: false,
            explore_progress_tx: None,
            build_progress_tx: None,
            missing_lsp_tx: None,
            current_model: None,
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

        assert!(ctx.lsp_manager.is_none());
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
            name: String,
        }

        let params = json!({"name": 123}); // Wrong type
        let result: Result<TestParams, ToolResult> = parse_params(params);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_error);
        assert!(err.output.contains("Invalid parameters"));
    }
}
