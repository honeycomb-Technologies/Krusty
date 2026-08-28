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

/// Which agent-extension stages may observe one registry execution.
///
/// Standard pre/post hooks are deliberately outside this policy: safety,
/// permission, logging, and lifecycle hooks remain authoritative even when an
/// isolated Worker run excludes the optional extension subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentExtensionDispatch {
    BeforeAndAfter,
    AfterOnly,
    Disabled,
}

impl AgentExtensionDispatch {
    fn runs_before(self) -> bool {
        matches!(self, Self::BeforeAndAfter)
    }

    fn runs_after(self) -> bool {
        matches!(self, Self::BeforeAndAfter | Self::AfterOnly)
    }
}

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

/// Foreground Agent runs own their convergence through durable group state,
/// per-provider deadlines, semantic loop guards, and explicit cancellation.
/// Wrapping that lifecycle in the generic tool timeout can abandon a healthy
/// graph while its leased tasks are still making progress. Explicit context
/// deadlines remain authoritative, and detached starts/control calls retain
/// the ordinary outer guard.
pub(super) fn should_apply_outer_timeout(
    name: &str,
    params: &Value,
    context_override: Option<Duration>,
) -> bool {
    if name != "agent" || context_override.is_some() {
        return true;
    }
    let action = params
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("spawn");
    let foreground_run = matches!(action, "spawn" | "resume" | "followup")
        && params.get("run_in_background").and_then(Value::as_bool) != Some(true);
    !foreground_run
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
        self.execute_inner(name, params, ctx, AgentExtensionDispatch::BeforeAndAfter)
            .await
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
        self.execute_inner(name, params, ctx, AgentExtensionDispatch::AfterOnly)
            .await
    }

    /// Execute an isolated Worker call without exposing its arguments or
    /// result to optional agent extensions. Canonical pre/post hooks still run
    /// and the ordinary timeout and tool policy remain unchanged.
    pub(crate) async fn execute_without_extensions(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        self.execute_inner(name, params, ctx, AgentExtensionDispatch::Disabled)
            .await
    }

    async fn execute_inner(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
        extension_dispatch: AgentExtensionDispatch,
    ) -> Option<ToolResult> {
        tracing::info!(tool = name, "ToolRegistry: execute called");
        let tool = self.get(name).await?;
        tracing::info!(tool = name, "ToolRegistry: tool found, executing");
        let mut params = params;
        let extension_manager = if extension_dispatch.runs_before() {
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
                    return Some(policy_block_result(name, &params, reason));
                }
            }
        }

        let result = if should_apply_outer_timeout(name, &params, ctx.timeout) {
            match tokio::time::timeout(timeout, tool.execute(params.clone(), ctx)).await {
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
            }
        } else {
            tool.execute(params.clone(), ctx).await
        };

        let duration = start.elapsed();

        for hook in &self.post_hooks {
            let _ = hook
                .after_execute(name, &params, &result, duration, ctx)
                .await;
        }

        if extension_dispatch.runs_after() {
            if let Some(manager) = self.agent_extension_manager() {
                manager.after_tool(name, &params, &result, ctx).await;
            }
        }

        Some(result)
    }
}

pub(super) fn policy_block_result(name: &str, params: &Value, reason: String) -> ToolResult {
    let normalized = reason.to_ascii_lowercase();
    let normalized_command = params
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let public_preview_bind = name == "bash"
        && (normalized.contains("0.0.0.0")
            || normalized.contains("non-loopback")
            || normalized.contains("all-interface")
            || normalized.contains("all network interfaces")
            || normalized_command.contains("0.0.0.0")
            || normalized_command.contains("--host 0.0.0.0")
            || normalized_command.contains("--bind 0.0.0.0"));

    if public_preview_bind {
        return ToolResult::error_with_recovery(
            "blocked_by_policy",
            reason,
            false,
            "Bind the preview server to 127.0.0.1, verify it locally, then expose that loopback endpoint through the approved private proxy such as Tailscale Serve.",
            vec![
                "Do not retry the same command with another wildcard or non-loopback bind syntax."
                    .to_string(),
                "Do not use an untracked shell background process to bypass listener policy."
                    .to_string(),
            ],
            Some(serde_json::json!({
                "bind_address": "127.0.0.1",
                "exposure": "tailscale_serve",
                "verify_local_first": true,
            })),
        );
    }

    ToolResult::error_with_recovery(
        "blocked_by_policy",
        reason,
        false,
        format!(
            "Change the {name} operation to satisfy the reported policy boundary; do not repeat the same blocked operation unchanged."
        ),
        vec!["Do not retry the same blocked operation with cosmetic argument changes.".to_string()],
        params.get("command").map(|_| {
            serde_json::json!({
                "strategy": "use a narrower in-scope command or a dedicated governed tool"
            })
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    use super::{AgentExtensionDispatch, Tool, ToolRegistry};
    use crate::agent::hooks::{HookResult, PostToolHook, PreToolHook};
    use crate::tools::registry::{ToolContext, ToolResult};

    struct CountingTool(Arc<AtomicUsize>);

    #[async_trait]
    impl Tool for CountingTool {
        fn name(&self) -> &str {
            "counting"
        }

        fn description(&self) -> &str {
            "count executions"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            ToolResult::success("executed")
        }
    }

    struct CountingPreHook(Arc<AtomicUsize>);

    #[async_trait]
    impl PreToolHook for CountingPreHook {
        async fn before_execute(
            &self,
            _name: &str,
            _params: &Value,
            _ctx: &ToolContext,
        ) -> HookResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            HookResult::Continue
        }
    }

    struct CountingPostHook(Arc<AtomicUsize>);

    #[async_trait]
    impl PostToolHook for CountingPostHook {
        async fn after_execute(
            &self,
            _name: &str,
            _params: &Value,
            _result: &ToolResult,
            _duration: Duration,
            _ctx: &ToolContext,
        ) -> HookResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            HookResult::Continue
        }
    }

    #[test]
    fn isolated_dispatch_exposes_neither_extension_stage() {
        assert!(AgentExtensionDispatch::BeforeAndAfter.runs_before());
        assert!(AgentExtensionDispatch::BeforeAndAfter.runs_after());
        assert!(!AgentExtensionDispatch::AfterOnly.runs_before());
        assert!(AgentExtensionDispatch::AfterOnly.runs_after());
        assert!(!AgentExtensionDispatch::Disabled.runs_before());
        assert!(!AgentExtensionDispatch::Disabled.runs_after());
    }

    #[tokio::test]
    async fn execute_without_extensions_retains_canonical_hooks_and_tool_execution() {
        let temp_dir = TempDir::new().expect("temp directory should be created");
        let tool_calls = Arc::new(AtomicUsize::new(0));
        let pre_hook_calls = Arc::new(AtomicUsize::new(0));
        let post_hook_calls = Arc::new(AtomicUsize::new(0));
        let extension_calls = Arc::new(AtomicUsize::new(0));

        let mut registry = ToolRegistry::new();
        registry.add_pre_hook(Arc::new(CountingPreHook(Arc::clone(&pre_hook_calls))));
        registry.add_post_hook(Arc::new(CountingPostHook(Arc::clone(&post_hook_calls))));
        registry
            .register(Arc::new(CountingTool(Arc::clone(&tool_calls))))
            .await;

        let manager = crate::extensions::AgentExtensionManager::new_with_paths(
            temp_dir.path(),
            temp_dir.path().join("extension-runtime"),
            temp_dir.path().join("global-extensions"),
        );
        manager.set_test_tool_interceptor({
            let extension_calls = Arc::clone(&extension_calls);
            move |_name, params| {
                extension_calls.fetch_add(1, Ordering::SeqCst);
                crate::extensions::AgentExtensionToolIntercept {
                    params,
                    block_reason: Some("extension must not run".to_string()),
                }
            }
        });
        registry.set_agent_extension_manager(manager);

        let result = registry
            .execute_without_extensions("counting", json!({}), &ToolContext::default())
            .await
            .expect("registered tool should produce a result");

        assert!(!result.is_error);
        assert_eq!(tool_calls.load(Ordering::SeqCst), 1);
        assert_eq!(pre_hook_calls.load(Ordering::SeqCst), 1);
        assert_eq!(post_hook_calls.load(Ordering::SeqCst), 1);
        assert_eq!(extension_calls.load(Ordering::SeqCst), 0);
    }
}
