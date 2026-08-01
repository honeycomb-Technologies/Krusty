use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use crate::ai::types::AiTool;
use crate::tools::registry::{DelegationPolicy, ToolContext, ToolRegistry, ToolResult};

use super::super::build_context::SharedBuildContext;
use super::super::tools::BuilderTools;
use super::super::types::{AgentProgress, SubAgentTask};

/// Configuration trait for agent behavior. This abstracts explorer vs builder differences.
#[async_trait::async_trait]
pub(crate) trait AgentConfig: Send + Sync {
    /// Get system prompt (can be static or dynamic per turn).
    fn system_prompt(&self, turn: usize) -> String;

    /// Append-only runtime context. The loop persists a new tail message only
    /// when this value changes, preserving every previously cached prefix.
    fn dynamic_context(&self) -> Option<String> {
        None
    }

    /// Tool timeout in seconds.
    fn timeout_secs(&self) -> u64;

    /// Per-turn API call timeout.
    fn api_call_timeout(&self) -> Duration {
        crate::agent::constants::timeouts::EXPLORER_API_CALL
    }

    /// Max tokens for API calls.
    fn max_tokens(&self) -> usize;

    /// Get tool definitions for AI.
    fn get_ai_tools(&self) -> Vec<AiTool>;

    /// Execute a tool call.
    async fn execute_tool(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult>;

    /// Format action description for progress reporting.
    fn format_action(&self, tool_name: &str, params: &Value) -> String {
        match tool_name {
            "read" => {
                let path = params
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let short_path = path.rsplit('/').next().unwrap_or(path);
                format!("read {}", short_path)
            }
            "glob" => {
                let pattern = params
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*");
                format!("glob {}", pattern)
            }
            "grep" => {
                let pattern = params
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let short = if pattern.len() > 12 {
                    &pattern[..12]
                } else {
                    pattern
                };
                format!("grep {}", short)
            }
            "write" | "edit" => {
                let path = params
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let short_path = path.rsplit('/').next().unwrap_or(path);
                format!("{} {}", tool_name, short_path)
            }
            "bash" => {
                let cmd = params
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let short = if cmd.len() > 15 { &cmd[..15] } else { cmd };
                format!("bash {}", short)
            }
            _ => tool_name.to_string(),
        }
    }

    /// Update progress with agent-specific metadata (e.g., line counts for builders).
    fn update_progress(&self, progress: &mut AgentProgress);

    /// Cleanup on exit (e.g., release locks for builders).
    fn cleanup(&self);

    /// Check if a file was read (for tracking files examined).
    fn is_read_tool(&self, name: &str) -> bool {
        name == "read"
    }

    /// Whether to apply explorer-specific heuristics (forced reports, stale cycle detection).
    /// The legacy multi-agent pool ExplorerConfig returns true; the unified SingleChildConfig
    /// returns false since the capable model manages its own completion.
    fn use_explorer_heuristics(&self) -> bool {
        true
    }
}

/// Builder configuration - read-write, coordinated.
pub(crate) struct BuilderConfig {
    task: SubAgentTask,
    tools: BuilderTools,
    context: Arc<SharedBuildContext>,
}

impl BuilderConfig {
    pub fn new(task: SubAgentTask, context: Arc<SharedBuildContext>) -> Self {
        let task_id = task.id.clone();
        Self {
            task,
            tools: BuilderTools::new(context.clone(), task_id),
            context,
        }
    }
}

#[async_trait::async_trait]
impl AgentConfig for BuilderConfig {
    fn system_prompt(&self, _turn: usize) -> String {
        builder_system_prompt(&self.task.working_dir)
    }

    fn dynamic_context(&self) -> Option<String> {
        let context = self.context.generate_context_injection();
        let body = if context.trim().is_empty() {
            "No active shared coordination metadata.".to_string()
        } else {
            context
        };
        Some(format!(
            "[COORDINATION UPDATE]\n{body}\n[/COORDINATION UPDATE]"
        ))
    }

    fn timeout_secs(&self) -> u64 {
        120
    }

    fn api_call_timeout(&self) -> Duration {
        crate::agent::constants::timeouts::BUILDER_API_CALL
    }

    fn max_tokens(&self) -> usize {
        16384
    }

    fn get_ai_tools(&self) -> Vec<AiTool> {
        let Some(policy) = self.task.delegation_policy.as_ref() else {
            // Execution also fails closed without delegated policy metadata;
            // keep the provider schema aligned with that runtime boundary.
            return Vec::new();
        };

        self.tools
            .get_ai_tools()
            .into_iter()
            .filter(|tool| policy.authorize_tool(&tool.name, false).is_ok())
            .collect()
    }

    async fn execute_tool(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        self.tools.execute(name, params, ctx).await
    }

    fn update_progress(&self, progress: &mut AgentProgress) {
        let (lines_added, lines_removed) = self.context.get_line_diff();
        progress.lines_added = lines_added;
        progress.lines_removed = lines_removed;
    }

    fn cleanup(&self) {
        self.context.release_all_locks(&self.task.id);
    }
}

/// Unified single-child config. The parent's instructions define the work;
/// the delegated policy defines the exact read/write/execute tool surface.
pub(crate) struct SingleChildConfig {
    policy: DelegationPolicy,
    registry: Arc<ToolRegistry>,
    ai_tools: Vec<AiTool>,
    project_context: String,
}

impl SingleChildConfig {
    pub async fn new(
        registry: Arc<ToolRegistry>,
        policy: DelegationPolicy,
        project_context: String,
    ) -> Self {
        let ai_tools = registry.get_ai_tools_filtered(&policy).await;
        Self {
            policy,
            registry,
            ai_tools,
            project_context,
        }
    }
}

#[async_trait::async_trait]
impl AgentConfig for SingleChildConfig {
    fn system_prompt(&self, _turn: usize) -> String {
        let mut prompt = String::from(
            "You are a delegated child agent of the parent Agent in Mitsuro. \
Complete only the task the parent assigned. You are not a fixed specialty — \
follow the parent instructions exactly.\n\n\
## Non-Negotiable Rules\n\
1. You are NOT the main agent. Do NOT address the end user directly.\n\
2. Do NOT spawn further sub-agents or call the agent tool.\n\
3. Stay within the parent's objective and any stated scope.\n\
4. Use tools for evidence. Prefer dedicated read/search tools over bash when possible.\n\
5. Stop when the objective is answered; return a concise report to the parent.\n\
6. Honor capability limits: if write tools are unavailable, do not attempt edits.\n\n\
## Report\n\
End with a short structured summary: what you did, key paths, outcome, and blockers.\n",
        );

        if !self.project_context.is_empty() {
            prompt.push('\n');
            prompt.push_str(&self.project_context);
            prompt.push('\n');
        }

        prompt
    }

    fn timeout_secs(&self) -> u64 {
        120
    }

    fn api_call_timeout(&self) -> Duration {
        crate::agent::constants::timeouts::SINGLE_EXPLORER_API_CALL
    }

    fn max_tokens(&self) -> usize {
        16384
    }

    fn get_ai_tools(&self) -> Vec<AiTool> {
        self.ai_tools.clone()
    }

    async fn execute_tool(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        if let Err(reason) = self
            .policy
            .authorize_tool_call(name, &params, ctx.plan_mode)
        {
            return Some(ToolResult::error(reason));
        }
        self.registry.execute(name, params, ctx).await
    }

    fn update_progress(&self, _progress: &mut AgentProgress) {}

    fn cleanup(&self) {}

    fn use_explorer_heuristics(&self) -> bool {
        false
    }
}

/// Compatibility name for callers in the legacy explorer pool.
pub(crate) type SingleExplorerConfig = SingleChildConfig;

/// Generate write-capable child system prompt with context injection.
fn builder_system_prompt(working_dir: &std::path::Path) -> String {
    format!(
        r#"You are a delegated child agent of the parent Agent in Mitsuro with write access.
Complete only the task the parent assigned. You are not a fixed specialty — follow the parent instructions exactly.

## Working Directory
{}

## Non-Negotiable Rules
1. You are NOT the main agent. Do NOT address the end user directly.
2. Do NOT spawn further sub-agents or call the agent tool.
3. Stay within the parent's objective and any stated file/component scope.
4. ALWAYS read files before editing — other children may have modified them.
5. Prefer targeted edits over wholesale rewrites.
6. Do NOT claim completion if edits failed or required checks are broken.
7. Follow repository instructions (AGENTS.md / project conventions) and any [CONVENTIONS] from the parent.
8. If a file lock wait exceeds 30 seconds, skip that file and note it in your report.

## Process
1. Locate relevant files (glob/grep/read)
2. Make the assigned changes (write/edit)
3. Lightly validate when the parent asked for it
4. Return a concise report to the parent

## Report
End with: files changed, outcome, validation, blockers.
"#,
        working_dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::PermissionMode;
    use std::collections::HashSet;

    #[test]
    fn builder_schema_intersects_with_exact_parent_execution_scope() {
        let scope = HashSet::from(["agent".to_string(), "read".to_string()]);
        let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, Some(7))
            .with_execution_tool_allowlist(Some(&scope));
        let task = SubAgentTask::new("builder", "work").with_delegation_policy(policy);
        let config = BuilderConfig::new(task, Arc::new(SharedBuildContext::new()));

        let names = config
            .get_ai_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["read"]);
        assert!(!names
            .iter()
            .any(|name| { matches!(name.as_str(), "bash" | "write" | "edit" | "apply_patch") }));
    }

    #[test]
    fn builder_schema_is_empty_for_agent_only_parent_scope() {
        let scope = HashSet::from(["agent".to_string()]);
        let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, Some(7))
            .with_execution_tool_allowlist(Some(&scope));
        let task = SubAgentTask::new("builder", "work").with_delegation_policy(policy);
        let config = BuilderConfig::new(task, Arc::new(SharedBuildContext::new()));

        assert!(config.get_ai_tools().is_empty());
    }

    #[test]
    fn builder_schema_preserves_explicit_empty_scope_as_tool_free() {
        let scope = HashSet::new();
        let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, Some(7))
            .with_execution_tool_allowlist(Some(&scope));
        let task = SubAgentTask::new("builder", "work").with_delegation_policy(policy.clone());
        let config = BuilderConfig::new(task, Arc::new(SharedBuildContext::new()));

        assert_eq!(policy.execution_tool_allowlist, Some(Default::default()));
        assert!(config.get_ai_tools().is_empty());
    }
    #[test]
    fn builder_system_prompt_stays_stable_while_coordination_updates() {
        let context = Arc::new(SharedBuildContext::new());
        let task = SubAgentTask::new("builder", "work")
            .with_working_dir(std::path::PathBuf::from("/workspace"));
        let config = BuilderConfig::new(task, context.clone());

        let original_prompt = config.system_prompt(1);
        let original_context = config.dynamic_context();
        context.set_conventions(vec!["Keep provider prefixes stable".to_string()]);
        let updated_context = config.dynamic_context();

        assert_eq!(config.system_prompt(2), original_prompt);
        assert_ne!(updated_context, original_context);
        assert!(updated_context
            .as_deref()
            .is_some_and(|value| value.contains("Keep provider prefixes stable")));
    }
}
