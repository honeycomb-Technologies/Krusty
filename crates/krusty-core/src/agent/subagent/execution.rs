//! Sub-agent execution loop
//!
//! Unified agentic loop for both explorer and builder agents.

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::build_context::SharedBuildContext;
use crate::agent::cache::SharedExploreCache;
use crate::agent::constants::subagent;
use crate::agent::history_policy::build_history_tool_result;
use crate::agent::AgentConfig as RuntimeAgentConfig;
use crate::ai::client::AiClient;
use crate::ai::retry::{with_retry, RetryConfig};
use crate::ai::types::{AiTool, Content, ModelMessage, Role};
use crate::tools::registry::{ToolContext, ToolResult};

use super::tools::{BuilderTools, SubAgentTools};
use super::types::{
    AgentProgress, AgentProgressStatus, SubAgentApiError, SubAgentResult, SubAgentTask, ToolCall,
};

const MAX_DELEGATED_POLICY_VIOLATIONS: usize = 3;

/// Configuration trait for agent behavior - abstracts explorer vs builder differences
#[async_trait::async_trait]
pub(crate) trait AgentConfig: Send + Sync {
    /// Get system prompt (can be static or dynamic per turn)
    fn system_prompt(&self, turn: usize) -> String;

    /// Tool timeout in seconds
    fn timeout_secs(&self) -> u64;

    /// Per-turn API call timeout
    fn api_call_timeout(&self) -> Duration {
        crate::agent::constants::timeouts::EXPLORER_API_CALL
    }

    /// Max tokens for API calls
    fn max_tokens(&self) -> usize;

    /// Get tool definitions for AI
    fn get_ai_tools(&self) -> Vec<AiTool>;

    /// Execute a tool call
    async fn execute_tool(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult>;

    /// Format action description for progress reporting
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

    /// Update progress with agent-specific metadata (e.g., line counts for builders)
    fn update_progress(&self, progress: &mut AgentProgress);

    /// Cleanup on exit (e.g., release locks for builders)
    fn cleanup(&self);

    /// Check if a file was read (for tracking files examined)
    fn is_read_tool(&self, name: &str) -> bool {
        name == "read"
    }
}

/// Explorer configuration - read-only, cached
pub(crate) struct ExplorerConfig {
    task: SubAgentTask,
    tools: SubAgentTools,
}

impl ExplorerConfig {
    pub fn new(task: SubAgentTask, cache: Arc<SharedExploreCache>) -> Self {
        Self {
            task,
            tools: SubAgentTools::new(cache),
        }
    }
}

#[async_trait::async_trait]
impl AgentConfig for ExplorerConfig {
    fn system_prompt(&self, _turn: usize) -> String {
        self.task.system_prompt()
    }

    fn timeout_secs(&self) -> u64 {
        30
    }

    fn max_tokens(&self) -> usize {
        // High limit - let the model stop naturally when done
        16384
    }

    fn get_ai_tools(&self) -> Vec<AiTool> {
        self.tools.get_ai_tools()
    }

    async fn execute_tool(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        self.tools.execute(name, params, ctx).await
    }

    fn update_progress(&self, _progress: &mut AgentProgress) {
        // Explorer doesn't track lines
    }

    fn cleanup(&self) {
        // Explorer has no locks to release
    }
}

/// Builder configuration - read-write, coordinated
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
        // Dynamic - refreshed each turn to include latest context
        builder_system_prompt(&self.task.working_dir, &self.context)
    }

    fn timeout_secs(&self) -> u64 {
        120 // Builders get more time
    }

    fn api_call_timeout(&self) -> Duration {
        crate::agent::constants::timeouts::BUILDER_API_CALL
    }

    fn max_tokens(&self) -> usize {
        // High limit - let the model stop naturally when done
        16384
    }

    fn get_ai_tools(&self) -> Vec<AiTool> {
        self.tools.get_ai_tools()
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

fn delegated_turn_budget(task: &SubAgentTask) -> Option<usize> {
    task.max_turns_override
        .or(task
            .delegation_policy
            .as_ref()
            .and_then(|policy| policy.max_turns))
        .or(RuntimeAgentConfig::default().subagent_max_turns)
}

fn build_subagent_tool_context(task: &SubAgentTask, timeout_secs: u64) -> ToolContext {
    let mut ctx = ToolContext {
        working_dir: task.working_dir.clone(),
        timeout: Some(Duration::from_secs(timeout_secs)),
        ..Default::default()
    }
    .with_subagent_max_turns(delegated_turn_budget(task));

    if let Some(policy) = task.delegation_policy.clone() {
        ctx = ctx
            .with_permission_mode(policy.inherited_permission_mode)
            .with_delegation_policy(policy);
    }

    ctx
}

fn completion_summary_preview(text: &str) -> Option<String> {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return None;
    }

    let summary = lines.join(" ");
    if summary.len() <= 600 {
        Some(summary)
    } else {
        let mut end = 600;
        while end > 0 && !summary.is_char_boundary(end) {
            end -= 1;
        }
        Some(format!("{}...", &summary[..end]))
    }
}

/// Unified agentic loop - replaces separate explorer/builder implementations
pub(crate) async fn execute_agent_loop<C: AgentConfig>(
    client: &AiClient,
    task: &SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    config: &C,
    progress_tx: Option<mpsc::UnboundedSender<AgentProgress>>,
) -> SubAgentResult {
    let start = Instant::now();
    let task_id = task.id.clone();
    let task_name = task.name.clone();
    let plan_task_id = task.plan_task_id.clone();

    let ai_tools = config.get_ai_tools();

    let ctx = build_subagent_tool_context(task, config.timeout_secs());

    let mut messages: Vec<ModelMessage> = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: task.prompt.clone(),
        }],
    }];

    let mut files_examined: Vec<String> = vec![];
    let mut turns = 0;
    let mut total_tool_calls = 0;
    let mut estimated_tokens: usize = 0;
    let mut final_output = String::new();
    let mut last_action = "starting...".to_string();
    let mut policy_violations: Vec<String> = Vec::new();

    // Helper to send progress
    let send_progress = |status: AgentProgressStatus,
                         action: &str,
                         tool_count: usize,
                         tokens: usize,
                         completion_summary: Option<String>,
                         config: &C| {
        if let Some(ref tx) = progress_tx {
            let is_complete = status == AgentProgressStatus::Complete;
            let mut progress = AgentProgress {
                task_id: task_id.clone(),
                name: task_name.clone(),
                status,
                tool_count,
                tokens,
                current_action: Some(action.to_string()),
                completion_summary,
                completed_plan_task: if is_complete {
                    plan_task_id.clone()
                } else {
                    None
                },
                ..Default::default()
            };
            config.update_progress(&mut progress);
            let _ = tx.send(progress);
        }
    };

    // Send initial progress
    send_progress(
        AgentProgressStatus::Running,
        &last_action,
        0,
        0,
        None,
        config,
    );

    loop {
        if cancellation.is_cancelled() {
            info!(task_id = %task_id, "Agent cancelled");
            send_progress(
                AgentProgressStatus::Failed,
                "cancelled",
                total_tool_calls,
                estimated_tokens,
                None,
                config,
            );
            config.cleanup();
            return SubAgentResult {
                task_id,
                success: false,
                output: String::new(),
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: Some("Cancelled".to_string()),
                policy_violations,
            };
        }

        turns += 1;

        let max_turns_budget = delegated_turn_budget(task);
        if let Some(max_turns) = max_turns_budget {
            if turns > max_turns {
                warn!(
                    task_id = %task_id,
                    turns = turns,
                    max_turns = max_turns,
                    "Sub-agent exceeded max turns"
                );
                send_progress(
                    AgentProgressStatus::Failed,
                    "max turns reached",
                    total_tool_calls,
                    estimated_tokens,
                    None,
                    config,
                );
                config.cleanup();
                return SubAgentResult {
                    task_id,
                    success: false,
                    output: final_output,
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: Some(format!(
                        "Sub-agent exceeded configured turn budget ({})",
                        max_turns
                    )),
                    policy_violations,
                };
            }
        }

        // Get system prompt (may be dynamic for builders)
        let system_prompt = config.system_prompt(turns);

        let thinking_action = if total_tool_calls > 0 {
            format!("{}...", last_action)
        } else {
            "thinking...".to_string()
        };
        send_progress(
            AgentProgressStatus::Running,
            &thinking_action,
            total_tool_calls,
            estimated_tokens,
            None,
            config,
        );

        let api_future = call_subagent_api(
            client,
            model,
            &system_prompt,
            &messages,
            &ai_tools,
            config.max_tokens(),
            task.thinking_enabled,
        );

        let response = match tokio::time::timeout(config.api_call_timeout(), api_future).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                send_progress(
                    AgentProgressStatus::Failed,
                    "error",
                    total_tool_calls,
                    estimated_tokens,
                    None,
                    config,
                );
                config.cleanup();
                return SubAgentResult {
                    task_id,
                    success: false,
                    output: String::new(),
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: Some(e.to_string()),
                    policy_violations,
                };
            }
            Err(_) => {
                warn!(
                    task_id = %task_id,
                    turn = turns,
                    timeout_secs = config.api_call_timeout().as_secs(),
                    "Sub-agent API call timed out"
                );
                send_progress(
                    AgentProgressStatus::Failed,
                    "timeout",
                    total_tool_calls,
                    estimated_tokens,
                    None,
                    config,
                );
                config.cleanup();
                return SubAgentResult {
                    task_id,
                    success: false,
                    output: final_output,
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: Some(format!(
                        "API call timed out after {}s on turn {}",
                        config.api_call_timeout().as_secs(),
                        turns
                    )),
                    policy_violations,
                };
            }
        };

        // Estimate tokens from response
        if let Some(usage) = response.get("usage") {
            if let Some(input) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                estimated_tokens += input as usize;
            }
            if let Some(output) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                estimated_tokens += output as usize;
            }
        }

        let (text_parts, tool_calls, stop_reason) = parse_response(&response);

        if !text_parts.is_empty() {
            final_output = text_parts.join("\n");
        }

        if tool_calls.is_empty() || stop_reason == "end_turn" {
            info!(task_id = %task_id, turns = turns, output_len = final_output.len(), "Agent completed successfully");
            send_progress(
                AgentProgressStatus::Complete,
                "complete",
                total_tool_calls,
                estimated_tokens,
                completion_summary_preview(&final_output),
                config,
            );
            config.cleanup();
            return SubAgentResult {
                task_id,
                success: true,
                output: final_output,
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: None,
                policy_violations,
            };
        }

        // Add assistant message
        let mut assistant_content: Vec<Content> = text_parts
            .iter()
            .map(|t| Content::Text { text: t.clone() })
            .collect();

        for tc in &tool_calls {
            assistant_content.push(Content::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }

        messages.push(ModelMessage {
            role: Role::Assistant,
            content: assistant_content,
        });

        // Execute tools
        let mut tool_results: Vec<Content> = vec![];

        for tc in &tool_calls {
            total_tool_calls += 1;
            if let Some(policy) = task.delegation_policy.as_ref() {
                if let Err(reason) = policy.authorize_tool(&tc.name, ctx.plan_mode) {
                    let violation = format!("{}: {}", tc.name, reason);
                    policy_violations.push(violation.clone());
                    tool_results.push(Content::ToolResult {
                        tool_use_id: tc.id.clone(),
                        output: build_history_tool_result(
                            &tc.name,
                            &ToolResult::error_with_details(
                                "delegated_policy_block",
                                reason,
                                None,
                                Some(policy.audit_json()),
                            )
                            .output,
                            true,
                        ),
                        is_error: Some(true),
                    });

                    if policy_violations.len() >= MAX_DELEGATED_POLICY_VIOLATIONS {
                        send_progress(
                            AgentProgressStatus::Failed,
                            "delegated policy blocked repeated tool calls",
                            total_tool_calls,
                            estimated_tokens,
                            None,
                            config,
                        );
                        config.cleanup();
                        return SubAgentResult {
                            task_id,
                            success: false,
                            output: final_output,
                            files_examined,
                            duration_ms: start.elapsed().as_millis() as u64,
                            turns_used: turns,
                            error: Some(
                                "Delegated policy containment triggered after repeated blocked tool attempts"
                                    .to_string(),
                            ),
                            policy_violations,
                        };
                    }
                    continue;
                }
            }

            // Track files examined
            if config.is_read_tool(&tc.name) {
                if let Some(path) = tc.input.get("file_path").and_then(|v| v.as_str()) {
                    files_examined.push(path.to_string());
                }
            }

            last_action = config.format_action(&tc.name, &tc.input);
            send_progress(
                AgentProgressStatus::Running,
                &last_action,
                total_tool_calls,
                estimated_tokens,
                None,
                config,
            );

            let result = config.execute_tool(&tc.name, tc.input.clone(), &ctx).await;

            let (output, is_error) = match result {
                Some(r) => (r.output, r.is_error),
                None => (format!("Unknown tool: {}", tc.name), true),
            };

            tool_results.push(Content::ToolResult {
                tool_use_id: tc.id.clone(),
                output: build_history_tool_result(&tc.name, &output, is_error),
                is_error: Some(is_error),
            });
        }

        messages.push(ModelMessage {
            role: Role::User,
            content: tool_results,
        });

        // Prune messages to keep context size manageable
        // Keep first message (user prompt) and recent messages
        if messages.len() > subagent::MAX_MESSAGES {
            let to_remove = messages.len() - subagent::MAX_MESSAGES;
            // Remove messages after the first one (index 1) up to to_remove count
            // This preserves the original prompt and keeps recent context
            for _ in 0..to_remove {
                messages.remove(1);
            }
            tracing::debug!(
                task_id = %task_id,
                removed = to_remove,
                remaining = messages.len(),
                "Pruned messages to stay within limit"
            );
        }
    }
}

/// Make a non-streaming API call for sub-agent with retry logic
pub(crate) async fn call_subagent_api(
    client: &AiClient,
    model: &str,
    system: &str,
    messages: &[ModelMessage],
    tools: &[AiTool],
    max_tokens: usize,
    thinking_enabled: bool,
) -> Result<Value, SubAgentApiError> {
    info!(
        model = model,
        msg_count = messages.len(),
        "SubAgent API call starting"
    );
    let start = Instant::now();

    // Build messages JSON (outside retry loop since it's deterministic)
    let messages_json: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "user",
                Role::Tool => "user",
            };

            let content: Vec<Value> = m
                .content
                .iter()
                .map(|c| match c {
                    Content::Text { text } => json!({"type": "text", "text": text}),
                    Content::ToolUse { id, name, input } => {
                        json!({"type": "tool_use", "id": id, "name": name, "input": input})
                    }
                    Content::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                    } => json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": output,
                        "is_error": is_error.unwrap_or(false)
                    }),
                    _ => json!({"type": "text", "text": "[unsupported content]"}),
                })
                .collect();

            json!({"role": role, "content": content})
        })
        .collect();

    // Build tools JSON — sorted by name for deterministic ordering.
    // Tool order is part of the cached prefix; non-deterministic order breaks caching.
    let mut sorted_tools: Vec<_> = tools.iter().collect();
    sorted_tools.sort_by(|a, b| a.name.cmp(&b.name));
    let tools_json: Vec<Value> = sorted_tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })
        })
        .collect();

    // Use aggressive retry for sub-agents (8 retries, 2s-60s backoff)
    // This handles rate limiting better, especially for providers with lower limits
    let config = RetryConfig::aggressive();

    let result = with_retry(&config, || async {
        client
            .call_with_tools(
                model,
                system,
                messages_json.clone(),
                tools_json.clone(),
                max_tokens,
                thinking_enabled,
            )
            .await
            .map_err(SubAgentApiError::from)
    })
    .await;

    let elapsed = start.elapsed();
    info!(
        elapsed_ms = elapsed.as_millis() as u64,
        success = result.is_ok(),
        "SubAgent API call completed"
    );
    result
}

/// Parse API response to extract text, tool calls, and stop reason
pub(crate) fn parse_response(response: &Value) -> (Vec<String>, Vec<ToolCall>, String) {
    let mut texts = vec![];
    let mut tool_calls = vec![];

    let stop_reason = response
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
        for block in content {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match block_type {
                "text" => {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        texts.push(text.to_string());
                    }
                }
                "tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(json!({}));

                    tool_calls.push(ToolCall { id, name, input });
                }
                _ => {}
            }
        }
    }

    (texts, tool_calls, stop_reason)
}

/// Generate builder system prompt with context injection
fn builder_system_prompt(working_dir: &std::path::Path, context: &SharedBuildContext) -> String {
    let context_injection = context.generate_context_injection();
    format!(
        r#"You are a builder agent. Your task is to implement code changes.

## Working Directory
{}

## Available Tools
1. **glob** - Find files by pattern (e.g., `**/*.rs`)
2. **grep** - Search file contents with regex
3. **read** - Read file contents (ALWAYS read before editing)
4. **write** - Write new files
5. **edit** - Edit existing files (requires reading first)
6. **bash** - Run shell commands

## Rules
1. ALWAYS read files before editing - other builders may have modified them
2. Create your OWN files for new components when possible
3. File locks are automatic - brief wait if another builder is writing
4. Follow [CONVENTIONS] if provided below
5. Be precise with edits - match exact strings

## Process
1. Use glob/grep to find relevant files
2. Read files you need to modify
3. Make your changes with write/edit
4. Summarize what you created/modified

{}

Build your component, then summarize what you created with file paths."#,
        working_dir.display(),
        context_injection
    )
}

/// Execute a sub-agent with agentic tool loop (no progress reporting)
pub(crate) async fn execute_subagent_with_tools(
    client: &AiClient,
    task: SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    cache: Arc<SharedExploreCache>,
) -> SubAgentResult {
    tracing::debug!(task_id = %task.id, model = %model, "Starting sub-agent");
    let config = ExplorerConfig::new(task.clone(), cache);
    execute_agent_loop(client, &task, model, cancellation, &config, None).await
}

/// Execute a sub-agent with progress reporting
pub(crate) async fn execute_subagent_with_progress(
    client: &AiClient,
    task: SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    cache: Arc<SharedExploreCache>,
    progress_tx: mpsc::UnboundedSender<AgentProgress>,
) -> SubAgentResult {
    let config = ExplorerConfig::new(task.clone(), cache);
    execute_agent_loop(
        client,
        &task,
        model,
        cancellation,
        &config,
        Some(progress_tx),
    )
    .await
}

/// Execute a builder agent with progress reporting
pub(crate) async fn execute_builder_with_progress(
    client: &AiClient,
    task: SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    context: Arc<SharedBuildContext>,
    progress_tx: mpsc::UnboundedSender<AgentProgress>,
) -> SubAgentResult {
    let config = BuilderConfig::new(task.clone(), context);
    execute_agent_loop(
        client,
        &task,
        model,
        cancellation,
        &config,
        Some(progress_tx),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{build_subagent_tool_context, delegated_turn_budget};
    use crate::agent::subagent::SubAgentTask;
    use crate::agent::AgentConfig as RuntimeAgentConfig;
    use crate::tools::registry::{DelegationPolicy, PermissionMode};
    use std::path::PathBuf;

    #[test]
    fn delegated_turn_budget_prefers_task_override_then_policy_then_runtime_default() {
        let budget_from_task = delegated_turn_budget(
            &SubAgentTask::new("task", "prompt")
                .with_delegation_policy(DelegationPolicy::for_subagent_build(
                    PermissionMode::Autonomous,
                    Some(11),
                ))
                .with_max_turns(7),
        );
        assert_eq!(budget_from_task, Some(7));

        let budget_from_policy =
            delegated_turn_budget(&SubAgentTask::new("task", "prompt").with_delegation_policy(
                DelegationPolicy::for_subagent_explore(PermissionMode::Autonomous, Some(11)),
            ));
        assert_eq!(budget_from_policy, Some(11));

        assert_eq!(
            delegated_turn_budget(&SubAgentTask::new("task", "prompt")),
            RuntimeAgentConfig::default().subagent_max_turns
        );
    }

    #[test]
    fn build_subagent_tool_context_inherits_delegated_policy_contract() {
        let working_dir = PathBuf::from("/tmp/krusty-subagent");
        let policy = DelegationPolicy::for_subagent_build(PermissionMode::Autonomous, Some(17));
        let task = SubAgentTask::new("task", "prompt")
            .with_working_dir(working_dir.clone())
            .with_delegation_policy(policy.clone());

        let ctx = build_subagent_tool_context(&task, 45);

        assert_eq!(ctx.working_dir, working_dir);
        assert_eq!(ctx.timeout.map(|timeout| timeout.as_secs()), Some(45));
        assert_eq!(ctx.permission_mode, PermissionMode::Autonomous);
        assert_eq!(ctx.subagent_max_turns, Some(17));
        assert_eq!(ctx.delegation_policy, Some(policy));
    }
}
