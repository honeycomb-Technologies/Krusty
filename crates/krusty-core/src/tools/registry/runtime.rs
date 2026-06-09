use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::agent::hooks::{HookResult, PostToolHook, PreToolHook};
use crate::ai::types::AiTool;

use super::policy::DEFAULT_TOOL_TIMEOUT;
use super::{tool_policy_for_call, DelegationPolicy, ToolContext, ToolResult};

/// Trait for tool implementations
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (id)
    fn name(&self) -> &str;

    /// Tool description for AI
    fn description(&self) -> &str;

    /// Extended usage guidance injected into the system prompt.
    /// Unlike description() which goes in the tool schema,
    /// this returns detailed instructions that become part of
    /// the system prompt's tool guidance section.
    fn prompt(&self) -> Option<&str> {
        None
    }

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
                prompt: t.prompt().map(|s| s.to_string()),
            })
            .collect();
        ai_tools.sort_by(|a, b| a.name.cmp(&b.name));
        ai_tools
    }

    /// Get tools filtered by a delegation policy (excludes tools the policy denies + recursive delegation tools).
    pub async fn get_ai_tools_filtered(&self, policy: &DelegationPolicy) -> Vec<AiTool> {
        let tools = self.tools.read().await;
        let mut ai_tools: Vec<AiTool> = tools
            .values()
            .filter(|t| {
                let name = t.name();
                if matches!(
                    name,
                    "agent"
                        | "skill"
                        | "enter_plan_mode"
                        | "set_work_mode"
                        | "set_workspace_context"
                ) {
                    return false;
                }
                policy.authorize_tool(name, false).is_ok()
            })
            .map(|t| AiTool {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.parameters_schema(),
                prompt: t.prompt().map(|s| s.to_string()),
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
        let timeout = ctx
            .timeout
            .or(tool_policy_for_call(name, &params).timeout_override)
            .unwrap_or(self.default_timeout);
        let start = Instant::now();

        for hook in &self.pre_hooks {
            match hook.before_execute(name, &params, ctx).await {
                HookResult::Continue => {}
                HookResult::Block { reason } => {
                    tracing::info!(tool = name, reason = %reason, "Pre-hook blocked execution");
                    return Some(ToolResult::error_with_code("blocked_by_policy", reason));
                }
            }
        }

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

        for hook in &self.post_hooks {
            let _ = hook.after_execute(name, &params, &result, duration).await;
        }

        Some(result)
    }
}
