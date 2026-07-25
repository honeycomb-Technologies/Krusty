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
    /// The legacy multi-agent pool ExplorerConfig returns true; the new SingleExplorerConfig
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

/// Single-agent explorer config. Uses the real tool registry filtered by delegation policy.
/// Unlike ExplorerConfig which uses reimplemented SubAgentTools, this delegates to the
/// same tools the parent agent uses, with project context injection.
pub(crate) struct SingleExplorerConfig {
    policy: DelegationPolicy,
    registry: Arc<ToolRegistry>,
    ai_tools: Vec<AiTool>,
    project_context: String,
}

impl SingleExplorerConfig {
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
impl AgentConfig for SingleExplorerConfig {
    fn system_prompt(&self, _turn: usize) -> String {
        let mut prompt = String::from(
            "You are a codebase explorer. Investigate the codebase thoroughly and report your findings in clear natural language.\n\n\
             You have read-only access to tools. Use them to search, read, and understand the code.\n\n\
             ## Non-Negotiable Rules\n\
             1. You are NOT the main agent. Do NOT address the user directly.\n\
             2. Do NOT attempt to spawn sub-agents or call explore/build tools. You have no such capability.\n\
             3. Stay strictly within the directive's scope. Do not wander into unrelated modules.\n\
             4. Use tools (glob, grep, read, list) to gather evidence. Do not speculate without evidence.\n\
             5. Stop when you have enough evidence to answer thoroughly. Do not over-explore.\n\n\
             ## Strategy\n\
             1. Start with glob/grep to find relevant files and patterns.\n\
             2. Read key files to understand architecture and implementation.\n\
             3. Follow references across modules to build complete understanding.\n\
             4. Report findings with specific file paths and line references.\n\
             5. Stop when you have enough evidence to answer thoroughly.\n\n\
             ## Structured Report Format\n\
             Always structure your final response using this format:\n\n\
             ```\n\
             ### Scope\n\
             What you investigated and why.\n\n\
             ### Findings\n\
             Your discoveries with specific file paths and line references.\n\n\
             ### Key Files\n\
             Bullet list of the most important files examined.\n\n\
             ### Concerns\n\
             Any issues, risks, or gaps found (or \"None\" if clean).\n\
             ```\n\n\
             ## Constraint\n\
             Keep your report under 500 words. Be specific, not vague.\n",
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

/// Generate builder system prompt with context injection.
fn builder_system_prompt(working_dir: &std::path::Path) -> String {
    format!(
        r#"You are a builder agent. Your task is to implement code changes.

## Working Directory
{}

## Non-Negotiable Rules
1. You are NOT the main agent. Do NOT address the user directly.
2. Do NOT attempt to spawn sub-agents.
3. Stay strictly within your assigned component's scope.
4. ALWAYS read files before editing — other builders may have modified them.
5. Create your OWN files for new components when possible to minimize conflicts.
6. Be precise with edits — match exact strings from the file you just read.
7. Do NOT claim completion if edits failed or builds are broken.
8. Do NOT rewrite files wholesale — use edit for targeted changes.
9. Follow all [CONVENTIONS] specified below.
10. If a file lock wait exceeds 30 seconds, skip that file and note it in your report.

## Available Tools
1. **glob** - Find files by pattern (e.g., `**/*.rs`)
2. **grep** - Search file contents with regex
3. **read** - Read file contents (ALWAYS read before editing)
4. **write** - Write new files
5. **edit** - Edit existing files (requires reading first)
6. **bash** - Run shell commands

## Process
1. Use glob/grep to find relevant files
2. Read files you need to modify
3. Make your changes with write/edit
4. Summarize what you created/modified

## Structured Report Format
Always end with this structured summary:

```
### Files Changed
- path/to/file.rs: what you changed and why

### Files Created
- path/to/new_file.rs: purpose

### Issues
Any problems encountered (or "None")
```

Build your component, then summarize what you created with file paths."#,
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
