use std::collections::HashMap;
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::agent::hooks::{HookResult, PostToolHook, PreToolHook};
use crate::agent::subagent::AgentRuntimeManager;
use crate::ai::types::AiTool;

use super::policy::DEFAULT_TOOL_TIMEOUT;
use super::{tool_policy_for_call, DelegationPolicy, ToolContext, ToolRequestPolicy, ToolResult};

/// Bash performs its own process-group termination and drains bounded stdout/stderr
/// readers after the requested command timeout. Keep the registry's outer guard
/// beyond that lifecycle so it cannot drop the Bash future before cleanup finishes.
// Two pipe-reader joins may each consume 10s, in addition to process-group
// termination/reaping and the final spool flush.
const BASH_TIMEOUT_CLEANUP_MARGIN: Duration = Duration::from_secs(25);
const MAX_BASH_REQUESTED_TIMEOUT_MS: u64 = 600_000;

fn requested_bash_timeout(params: &Value) -> Option<Duration> {
    let raw = params.get("timeout")?;
    let timeout_ms = raw.as_u64().or_else(|| {
        let value = raw.as_f64()?;
        (value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= u64::MAX as f64)
            .then_some(value as u64)
    })?;

    Some(Duration::from_millis(
        timeout_ms.min(MAX_BASH_REQUESTED_TIMEOUT_MS),
    ))
}

pub(super) fn execution_timeout_for_call(
    name: &str,
    params: &Value,
    context_override: Option<Duration>,
    policy_override: Option<Duration>,
    default_timeout: Duration,
) -> Duration {
    let configured_timeout = context_override
        .or(policy_override)
        .unwrap_or(default_timeout);

    if name != "bash" {
        return configured_timeout;
    }

    requested_bash_timeout(params)
        .map(|requested| requested.saturating_add(BASH_TIMEOUT_CLEANUP_MARGIN))
        .map_or(configured_timeout, |bash_lifecycle_timeout| {
            configured_timeout.max(bash_lifecycle_timeout)
        })
}

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
    /// Executable agent-extension host shared by every caller of this registry.
    /// A standard lock keeps lookup available from the synchronous orchestrator
    /// startup boundary; extension work itself remains fully async.
    agent_extension_manager: StdRwLock<Option<Arc<crate::extensions::AgentExtensionManager>>>,
    /// Live delegated-run control shared across agent tool re-registration.
    agent_runtime_manager: AgentRuntimeManager,
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
            agent_extension_manager: StdRwLock::new(None),
            agent_runtime_manager: AgentRuntimeManager::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_default_timeout(default_timeout: Duration) -> Self {
        Self {
            default_timeout,
            ..Self::new()
        }
    }

    /// Register a tool
    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let mut tools = self.tools.write().await;
        tools.insert(name, tool);
    }

    /// Remove one exact tool name. Runtime extension and MCP refresh paths use
    /// this to avoid stale registrations without touching unrelated tools.
    pub async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.write().await.remove(name)
    }

    pub fn set_agent_extension_manager(
        &self,
        manager: Arc<crate::extensions::AgentExtensionManager>,
    ) {
        let mut current = self
            .agent_extension_manager
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = Some(manager);
    }

    pub fn agent_extension_manager(&self) -> Option<Arc<crate::extensions::AgentExtensionManager>> {
        self.agent_extension_manager
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn agent_runtime_manager(&self) -> AgentRuntimeManager {
        self.agent_runtime_manager.clone()
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

    /// Get the compact default coding tool surface, sorted by name.
    pub async fn get_ai_tools(&self) -> Vec<AiTool> {
        self.get_ai_tools_for_request(&ToolRequestPolicy::default())
            .await
    }

    /// Get the compact coding surface for a concrete runtime policy.
    pub async fn get_ai_tools_for_request(&self, policy: &ToolRequestPolicy) -> Vec<AiTool> {
        policy.filter(self.get_ai_tools_all().await)
    }

    /// Get every registered tool as an AI definition, sorted by name.
    ///
    /// Deterministic ordering is critical for prompt caching — tool definitions
    /// are part of the cached prefix, and non-deterministic order (from HashMap
    /// iteration) silently breaks the cache between API calls.
    pub async fn get_ai_tools_all(&self) -> Vec<AiTool> {
        let tools = self.tools.read().await;
        let mut ai_tools: Vec<AiTool> = tools
            .values()
            .map(|t| AiTool {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.parameters_schema(),
                prompt: None,
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
                prompt: None,
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
        self.execute_inner(name, params, ctx, true).await
    }

    /// Execute an agent-loop call whose extension interception already ran
    /// before the canonical authorization prompt. This prevents a second
    /// rewrite after the user approved the effective arguments.
    pub(crate) async fn execute_prepared(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        self.execute_inner(name, params, ctx, false).await
    }

    async fn execute_inner(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
        run_extension_intercept: bool,
    ) -> Option<ToolResult> {
        tracing::info!(tool = name, "ToolRegistry: execute called");
        let tool = self.get(name).await?;
        tracing::info!(tool = name, "ToolRegistry: tool found, executing");
        let mut params = params;
        let extension_manager = if run_extension_intercept {
            self.agent_extension_manager()
        } else {
            None
        };
        if let Some(manager) = extension_manager {
            let intercept = manager.before_tool(name, params, ctx).await;
            if let Some(reason) = intercept.block_reason {
                tracing::info!(tool = name, reason = %reason, "Agent extension blocked execution");
                return Some(ToolResult::error_with_code("blocked_by_extension", reason));
            }
            params = intercept.params;
        }
        let timeout = execution_timeout_for_call(
            name,
            &params,
            ctx.timeout,
            tool_policy_for_call(name, &params).timeout_override,
            self.default_timeout,
        );
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
            let _ = hook
                .after_execute(name, &params, &result, duration, ctx)
                .await;
        }

        if let Some(manager) = self.agent_extension_manager() {
            manager.after_tool(name, &params, &result, ctx).await;
        }

        Some(result)
    }
}
