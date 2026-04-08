//! Sub-agent execution loop
//!
//! Unified agentic loop for both explorer and builder agents.

use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::build_context::SharedBuildContext;
use crate::agent::constants::subagent;
use crate::agent::history_policy::build_history_tool_result;
use crate::agent::AgentConfig as RuntimeAgentConfig;
use crate::ai::client::AiClient;
use crate::ai::retry::{with_retry, RetryConfig};
use crate::ai::types::{AiTool, Content, ModelMessage, Role};
use crate::tools::registry::{DelegationPolicy, ToolContext, ToolRegistry, ToolResult};

use super::tools::BuilderTools;
use super::types::{
    parse_explore_report, render_explore_report, summary_looks_non_substantive,
    synthesize_explore_report, synthesize_explore_report_from_paths, AgentProgress,
    AgentProgressStatus, SubAgentApiError, SubAgentResult, SubAgentTask, ToolCall,
};

const MAX_DELEGATED_POLICY_VIOLATIONS: usize = 3;
const EXPLORER_STALE_SEQUENCE_THRESHOLD: usize = 3;
const EXPLORER_SYNTHESIS_FILE_THRESHOLD: usize = 8;

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

    /// Whether to apply explorer-specific heuristics (forced reports, stale cycle detection).
    /// The legacy multi-agent pool ExplorerConfig returns true; the new SingleExplorerConfig
    /// returns false since the capable model manages its own completion.
    fn use_explorer_heuristics(&self) -> bool {
        true
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

/// Single-agent explorer config — uses the real tool registry filtered by delegation policy.
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
        if let Err(reason) = self.policy.authorize_tool(name, ctx.plan_mode) {
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

/// Execute a single explorer agent using the real tool registry.
pub async fn execute_single_explorer(
    client: Arc<AiClient>,
    task: SubAgentTask,
    registry: Arc<ToolRegistry>,
    policy: DelegationPolicy,
    project_context: String,
    model: String,
    cancellation: CancellationToken,
    progress_tx: Option<mpsc::UnboundedSender<AgentProgress>>,
) -> SubAgentResult {
    let config = SingleExplorerConfig::new(registry, policy, project_context).await;
    execute_agent_loop(&client, &task, &model, cancellation, &config, progress_tx).await
}

/// Execute any agent type via the standard agent loop.
///
/// This is the generic version — specific helpers like `execute_single_explorer`
/// construct their config internally and call this.
pub(crate) async fn execute_single_agent<C: AgentConfig>(
    client: &AiClient,
    task: SubAgentTask,
    config: C,
    model: &str,
    cancellation: CancellationToken,
    progress_tx: Option<mpsc::UnboundedSender<AgentProgress>>,
) -> SubAgentResult {
    execute_agent_loop(client, &task, model, cancellation, &config, progress_tx).await
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

    if delegated_is_explore(task) {
        ctx = ctx.with_sandbox(task.working_dir.clone());
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

fn delegated_is_explore(task: &SubAgentTask) -> bool {
    task.delegation_policy
        .as_ref()
        .is_some_and(|policy| policy.read_only_only)
}

fn should_replace_forced_summary(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    if let Some(report) = parse_explore_report(trimmed) {
        return summary_looks_non_substantive(&report.summary)
            || report
                .key_findings
                .iter()
                .all(|finding| summary_looks_non_substantive(finding));
    }

    summary_looks_non_substantive(trimmed)
}

fn synthesized_explorer_output(
    task_name: &str,
    final_output: &str,
    files_examined: &[String],
) -> String {
    if final_output.trim().is_empty() || should_replace_forced_summary(final_output) {
        synthesize_explore_report_from_paths(task_name, files_examined)
            .map(|report| render_explore_report(&report))
            .unwrap_or_else(|| fallback_explorer_summary(files_examined))
    } else {
        final_output.to_string()
    }
}

fn normalize_explorer_result(mut result: SubAgentResult, task: &SubAgentTask) -> SubAgentResult {
    if result.delegated_run_id.is_none() {
        result.delegated_run_id = task.delegated_run_id.clone();
    }
    if delegated_is_explore(task) {
        if let Some(report) = parse_explore_report(&result.output) {
            for path in report.paths_examined {
                if !path.trim().is_empty()
                    && !result
                        .files_examined
                        .iter()
                        .any(|existing| existing == &path)
                {
                    result.files_examined.push(path);
                }
            }
            for file in report.files_examined {
                if !file.trim().is_empty()
                    && !result
                        .files_examined
                        .iter()
                        .any(|existing| existing == &file)
                {
                    result.files_examined.push(file);
                }
            }
            if !result.has_usable_evidence() {
                if let Some(report) =
                    synthesize_explore_report_from_paths(&task.name, &result.files_examined)
                {
                    let synthesized_paths = report.paths_examined.clone();
                    result.output = render_explore_report(&report);
                    result.files_examined = synthesized_paths;
                }
            }
        } else if result.success {
            if let Some(report) = synthesize_explore_report(&result.output, &result.files_examined)
            {
                let synthesized_paths = report.paths_examined.clone();
                result.output = render_explore_report(&report);
                result.files_examined = synthesized_paths;
            } else if let Some(report) =
                synthesize_explore_report_from_paths(&task.name, &result.files_examined)
            {
                let synthesized_paths = report.paths_examined.clone();
                result.output = render_explore_report(&report);
                result.files_examined = synthesized_paths;
            } else {
                result.success = false;
                result.error = Some(
                    "Delegated exploration finished without the required structured explore report"
                        .to_string(),
                );
            }
        }
    }

    if delegated_is_explore(task) && result.success && !result.has_usable_evidence() {
        result.success = false;
        result.error = Some("Delegated exploration completed without usable evidence".to_string());
    }
    result
}

fn relative_or_display(path: &str, working_dir: &Path) -> String {
    let candidate = std::path::Path::new(path);
    candidate
        .strip_prefix(working_dir)
        .unwrap_or(candidate)
        .display()
        .to_string()
}

fn collect_paths_from_tool_result(name: &str, output: &str, working_dir: &Path) -> Vec<String> {
    let parsed =
        serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()));
    let payload = parsed
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&parsed);

    let mut paths = Vec::new();
    match name {
        "read" => {
            if let Some(path) = payload.get("file_path").and_then(|value| value.as_str()) {
                paths.push(relative_or_display(path, working_dir));
            }
        }
        "glob" => {
            if let Some(matches) = payload.get("matches").and_then(|value| value.as_array()) {
                for entry in matches.iter().take(12) {
                    if let Some(path) = entry.as_str() {
                        paths.push(relative_or_display(path, working_dir));
                    }
                }
            }
        }
        "grep" => {
            if let Some(matches) = payload.get("matches").and_then(|value| value.as_array()) {
                for entry in matches.iter().take(12) {
                    if let Some(path) = entry.get("file").and_then(|value| value.as_str()) {
                        paths.push(relative_or_display(path, working_dir));
                    }
                }
            }
        }
        "list" => {
            if let Some(output) = payload.get("output").and_then(|value| value.as_str()) {
                for line in output.lines().take(12) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        paths.push(trimmed.to_string());
                    }
                }
            }
        }
        _ => {}
    }

    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

fn text_claims_tool_empty(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    let markers = [
        "no output",
        "nothing is working",
        "nothing worked",
        "returned no results",
        "returning no results",
        "returning empty results",
        "tools are returning empty",
        "every tool is returning empty",
        "unable to locate any files",
        "no files found",
    ];

    markers.iter().any(|marker| normalized.contains(marker))
}

fn tool_result_has_positive_evidence(name: &str, output: &str, is_error: bool) -> bool {
    if is_error {
        return false;
    }

    let parsed =
        serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()));
    let payload = parsed
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&parsed);

    match name {
        "read" => payload
            .get("content")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty()),
        "glob" => payload
            .get("count")
            .and_then(|value| value.as_u64())
            .is_some_and(|count| count > 0),
        "grep" => {
            payload
                .get("count")
                .and_then(|value| value.as_u64())
                .is_some_and(|count| count > 0)
                || payload
                    .get("total_matches")
                    .and_then(|value| value.as_u64())
                    .is_some_and(|count| count > 0)
        }
        "list" => payload
            .get("total_entries")
            .and_then(|value| value.as_u64())
            .is_some_and(|count| count > 0),
        "bash" => payload
            .get("output_preview")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty()),
        _ => false,
    }
}

fn timeout_partial_output(final_output: &str, files_examined: &[String]) -> String {
    if final_output.trim().is_empty() && !files_examined.is_empty() {
        format!(
            "Sub-agent timed out before producing final output. {} files were examined: {}",
            files_examined.len(),
            files_examined
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        final_output.to_string()
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
    let mut unique_files_examined: HashSet<String> = HashSet::new();
    let mut stale_readonly_cycles = 0usize;
    let mut forced_summary_requested = false;
    let mut last_cycle_positive_evidence = false;
    let mut tool_truth_corrections = 0usize;
    let mut forced_read_before_completion = false;
    let mut structured_report_repair_requested = false;

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
                delegated_run_id: task.delegated_run_id.clone(),
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
                agent_name: task_name.clone(),
                delegated_run_id: task.delegated_run_id.clone(),
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
                    agent_name: task_name.clone(),
                    delegated_run_id: task.delegated_run_id.clone(),
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
                    agent_name: task_name.clone(),
                    delegated_run_id: task.delegated_run_id.clone(),
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
                    files_examined = files_examined.len(),
                    "Sub-agent API call timed out"
                );
                let output = timeout_partial_output(&final_output, &files_examined);
                let has_evidence = !files_examined.is_empty();
                send_progress(
                    if has_evidence {
                        AgentProgressStatus::Complete
                    } else {
                        AgentProgressStatus::Failed
                    },
                    if has_evidence {
                        "timeout (partial)"
                    } else {
                        "timeout"
                    },
                    total_tool_calls,
                    estimated_tokens,
                    completion_summary_preview(&output),
                    config,
                );
                config.cleanup();
                return SubAgentResult {
                    task_id,
                    agent_name: task_name.clone(),
                    delegated_run_id: task.delegated_run_id.clone(),
                    success: has_evidence,
                    output,
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: if has_evidence {
                        None
                    } else {
                        Some(format!(
                            "API call timed out after {}s on turn {}",
                            config.api_call_timeout().as_secs(),
                            turns
                        ))
                    },
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

        if config.use_explorer_heuristics()
            && delegated_is_explore(task)
            && last_cycle_positive_evidence
            && text_claims_tool_empty(&final_output)
        {
            if tool_truth_corrections >= 1 {
                send_progress(
                    AgentProgressStatus::Failed,
                    "misread tool output",
                    total_tool_calls,
                    estimated_tokens,
                    completion_summary_preview(&final_output),
                    config,
                );
                config.cleanup();
                return SubAgentResult {
                    task_id,
                    agent_name: task_name.clone(),
                    delegated_run_id: task.delegated_run_id.clone(),
                    success: false,
                    output: final_output,
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: Some(
                        "Misread successful tool output after correction; delegated exploration is no longer trustworthy"
                            .to_string(),
                    ),
                    policy_violations,
                };
            }

            tool_truth_corrections += 1;
            messages.push(ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: "Correction: the previous tool calls returned real data. You must treat successful tool output as ground truth and use it directly. Do not claim the results were empty or that nothing worked. Summarize the evidence from the returned paths/content instead of rechecking blindly.".to_string(),
                }],
            });
            send_progress(
                AgentProgressStatus::Running,
                "correcting tool interpretation",
                total_tool_calls,
                estimated_tokens,
                completion_summary_preview(&final_output),
                config,
            );
            continue;
        }

        if tool_calls.is_empty() || stop_reason == "end_turn" {
            if config.use_explorer_heuristics() && delegated_is_explore(task) {
                let missing_report = parse_explore_report(&final_output).is_none();

                if files_examined.is_empty() && !forced_read_before_completion {
                    forced_read_before_completion = true;
                    messages.push(ModelMessage {
                        role: Role::User,
                        content: vec![Content::Text {
                            text: "You have not gathered any real path evidence yet. Before you finish, use the current working directory to inspect the most relevant files or directories, then produce the required <explore_report> with those paths in `paths_examined` and any concrete reads in `files_examined`. Do not stop yet.".to_string(),
                        }],
                    });
                    send_progress(
                        AgentProgressStatus::Running,
                        "requiring path evidence",
                        total_tool_calls,
                        estimated_tokens,
                        completion_summary_preview(&final_output),
                        config,
                    );
                    continue;
                }

                if missing_report && !structured_report_repair_requested {
                    structured_report_repair_requested = true;
                    messages.push(ModelMessage {
                        role: Role::User,
                        content: vec![Content::Text {
                            text: "Your previous response did not include the required <explore_report> JSON block. Using only the evidence you already gathered, reply now with exactly one valid <explore_report> block and no extra prose. Include all supporting paths in `paths_examined` and any concrete reads in `files_examined`. Do not call more tools unless a single critical read is required.".to_string(),
                        }],
                    });
                    send_progress(
                        AgentProgressStatus::Running,
                        "repairing structured report",
                        total_tool_calls,
                        estimated_tokens,
                        completion_summary_preview(&final_output),
                        config,
                    );
                    continue;
                }
            }

            let raw_result = SubAgentResult {
                task_id: task_id.clone(),
                agent_name: task_name.clone(),
                delegated_run_id: task.delegated_run_id.clone(),
                success: true,
                output: final_output,
                files_examined,
                duration_ms: start.elapsed().as_millis() as u64,
                turns_used: turns,
                error: None,
                policy_violations,
            };
            let result = if config.use_explorer_heuristics() {
                normalize_explorer_result(raw_result, task)
            } else {
                raw_result
            };
            info!(
                task_id = %result.task_id,
                turns = turns,
                output_len = result.output.len(),
                success = result.success,
                "Agent completed"
            );
            send_progress(
                if result.success {
                    AgentProgressStatus::Complete
                } else {
                    AgentProgressStatus::Failed
                },
                if result.success {
                    "complete"
                } else {
                    "degraded completion"
                },
                total_tool_calls,
                estimated_tokens,
                completion_summary_preview(&result.output),
                config,
            );
            config.cleanup();
            return result;
        }

        let delegated_is_explore = delegated_is_explore(task);
        let all_read_only_tools = delegated_is_explore
            && tool_calls
                .iter()
                .all(|call| matches!(call.name.as_str(), "read" | "glob" | "grep" | "list"));

        if config.use_explorer_heuristics()
            && delegated_is_explore
            && forced_summary_requested
            && all_read_only_tools
        {
            let synthesized =
                synthesized_explorer_output(&task.name, &final_output, &files_examined);
            let result = normalize_explorer_result(
                SubAgentResult {
                    task_id: task_id.clone(),
                    agent_name: task_name.clone(),
                    delegated_run_id: task.delegated_run_id.clone(),
                    success: true,
                    output: synthesized,
                    files_examined,
                    duration_ms: start.elapsed().as_millis() as u64,
                    turns_used: turns,
                    error: None,
                    policy_violations,
                },
                task,
            );
            send_progress(
                if result.success {
                    AgentProgressStatus::Complete
                } else {
                    AgentProgressStatus::Failed
                },
                if result.success {
                    "forced summary"
                } else {
                    "forced summary degraded"
                },
                total_tool_calls,
                estimated_tokens,
                completion_summary_preview(&result.output),
                config,
            );
            config.cleanup();
            return result;
        }

        // Add assistant message
        let mut assistant_content: Vec<Content> = text_parts
            .iter()
            .map(|t| Content::Text { text: t.clone() })
            .collect();

        let mut cycle_new_files = 0usize;

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
        let mut cycle_positive_evidence = false;

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
                            agent_name: task_name.clone(),
                            delegated_run_id: task.delegated_run_id.clone(),
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

            if tool_result_has_positive_evidence(&tc.name, &output, is_error) {
                cycle_positive_evidence = true;
            }

            if config.is_read_tool(&tc.name) {
                if let Some(path) = tc.input.get("file_path").and_then(|value| value.as_str()) {
                    let normalized = relative_or_display(path, &task.working_dir);
                    if unique_files_examined.insert(normalized.clone()) {
                        cycle_new_files += 1;
                        files_examined.push(normalized);
                    }
                }
            }

            for path in collect_paths_from_tool_result(&tc.name, &output, &task.working_dir) {
                if unique_files_examined.insert(path.clone()) {
                    cycle_new_files += 1;
                    files_examined.push(path);
                }
            }

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
        last_cycle_positive_evidence = cycle_positive_evidence;

        if config.use_explorer_heuristics() && delegated_is_explore && all_read_only_tools {
            let produced_new_files = cycle_new_files > 0;
            let has_sufficient_evidence = !final_output.trim().is_empty()
                && unique_files_examined.len() >= EXPLORER_SYNTHESIS_FILE_THRESHOLD;

            if has_sufficient_evidence || !produced_new_files {
                stale_readonly_cycles += 1;
            } else {
                stale_readonly_cycles = 0;
            }

            if stale_readonly_cycles >= EXPLORER_STALE_SEQUENCE_THRESHOLD {
                if !forced_summary_requested {
                    forced_summary_requested = true;
                    stale_readonly_cycles = 0;
                    messages.push(ModelMessage {
                        role: Role::User,
                        content: vec![Content::Text {
                            text: "You now have enough evidence to answer the investigation. Stop exploring. Do not call more tools unless a critical gap prevents answering. Provide a concise summary covering architecture, key modules, design patterns, main concerns, and the most important files examined.".to_string(),
                        }],
                    });
                    send_progress(
                        AgentProgressStatus::Running,
                        "synthesizing findings",
                        total_tool_calls,
                        estimated_tokens,
                        completion_summary_preview(&final_output),
                        config,
                    );
                    continue;
                }

                let synthesized =
                    synthesized_explorer_output(&task.name, &final_output, &files_examined);
                let result = normalize_explorer_result(
                    SubAgentResult {
                        task_id: task_id.clone(),
                        agent_name: task_name.clone(),
                        delegated_run_id: task.delegated_run_id.clone(),
                        success: true,
                        output: synthesized,
                        files_examined,
                        duration_ms: start.elapsed().as_millis() as u64,
                        turns_used: turns,
                        error: None,
                        policy_violations,
                    },
                    task,
                );
                send_progress(
                    if result.success {
                        AgentProgressStatus::Complete
                    } else {
                        AgentProgressStatus::Failed
                    },
                    if result.success {
                        "converged"
                    } else {
                        "converged degraded"
                    },
                    total_tool_calls,
                    estimated_tokens,
                    completion_summary_preview(&result.output),
                    config,
                );
                config.cleanup();
                return result;
            }
        } else {
            stale_readonly_cycles = 0;
        }

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

fn fallback_explorer_summary(files_examined: &[String]) -> String {
    let files = files_examined
        .iter()
        .filter(|path| !path.trim().is_empty())
        .take(8)
        .cloned()
        .collect::<Vec<_>>();

    if files.is_empty() {
        "Exploration converged without a final synthesized answer. Summarize the evidence gathered so far before requesting a narrower follow-up.".to_string()
    } else {
        format!(
            "Exploration converged after repeated low-yield read-only cycles. Use the gathered evidence to summarize the codebase based on these key files: {}.",
            files.join(", ")
        )
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
                    } => {
                        // Anthropic API requires tool_result content to be a string
                        // or array of content blocks — never a raw JSON object.
                        let content_str = match output {
                            Value::String(s) => Value::String(s.clone()),
                            other => Value::String(other.to_string()),
                        };
                        json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content_str,
                            "is_error": is_error.unwrap_or(false)
                        })
                    }
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

{}

Build your component, then summarize what you created with file paths."#,
        working_dir.display(),
        context_injection
    )
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
    use super::{
        build_subagent_tool_context, collect_paths_from_tool_result, delegated_turn_budget,
        should_replace_forced_summary, synthesized_explorer_output, text_claims_tool_empty,
        timeout_partial_output, tool_result_has_positive_evidence,
    };
    use crate::agent::subagent::SubAgentTask;
    use crate::agent::AgentConfig as RuntimeAgentConfig;
    use crate::tools::registry::{DelegationPolicy, PermissionMode};
    use serde_json::json;
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
        assert!(ctx.sandbox_root.is_none());
    }

    #[test]
    fn explore_subagent_tool_context_sandboxes_to_assigned_scope() {
        let working_dir = PathBuf::from("/tmp/krusty-explore-scope");
        let policy = DelegationPolicy::for_subagent_explore(PermissionMode::Autonomous, Some(9));
        let task = SubAgentTask::new("task", "prompt")
            .with_working_dir(working_dir.clone())
            .with_delegation_policy(policy);

        let ctx = build_subagent_tool_context(&task, 30);

        assert_eq!(ctx.working_dir, working_dir);
        assert_eq!(
            ctx.sandbox_root,
            Some(PathBuf::from("/tmp/krusty-explore-scope"))
        );
    }

    #[test]
    fn text_claims_tool_empty_detects_known_misreads() {
        assert!(text_claims_tool_empty(
            "Every tool is returning empty results and nothing is working."
        ));
        assert!(!text_claims_tool_empty(
            "The glob returned 12 files and the read succeeded."
        ));
    }

    #[test]
    fn tool_result_positive_evidence_detects_real_read_and_glob_data() {
        let read_output = json!({
            "data": {
                "content": "fn main() {}"
            }
        })
        .to_string();
        let glob_output = json!({
            "count": 3,
            "matches": ["src/main.rs", "src/lib.rs", "src/app.rs"]
        })
        .to_string();

        assert!(tool_result_has_positive_evidence(
            "read",
            &read_output,
            false
        ));
        assert!(tool_result_has_positive_evidence(
            "glob",
            &glob_output,
            false
        ));
        assert!(!tool_result_has_positive_evidence(
            "glob",
            &glob_output,
            true
        ));
    }

    #[test]
    fn collect_paths_from_list_preserves_directory_markers() {
        let output = json!({
            "data": {
                "output": "agent/\nai/\nstorage/\nlib.rs",
                "total_entries": 4
            }
        })
        .to_string();

        let paths = collect_paths_from_tool_result("list", &output, PathBuf::from(".").as_path());
        assert_eq!(paths, vec!["agent/", "ai/", "storage/", "lib.rs"]);
    }

    #[test]
    fn placeholder_forced_summary_gets_replaced() {
        assert!(should_replace_forced_summary(
            "Let me try using glob to inspect this target."
        ));
        assert!(should_replace_forced_summary(
            "Let me read the main module file and get context on the architecture:"
        ));
        assert!(should_replace_forced_summary(
            "I'll start by exploring the directory structure and then read the key files to understand the architecture."
        ));
        assert!(should_replace_forced_summary(
            "<explore_report>{\"summary\":\"Let me inspect a few files first.\",\"paths_examined\":[],\"files_examined\":[],\"key_findings\":[],\"design_patterns\":[],\"concerns\":[],\"confidence\":\"low\"}</explore_report>"
        ));
    }

    #[test]
    fn substantive_forced_summary_is_preserved() {
        assert!(!should_replace_forced_summary(
            "<explore_report>{\"summary\":\"The module centers runtime orchestration in orchestrator.rs and failure containment in failure.rs.\",\"paths_examined\":[\"agent/\",\"orchestrator.rs\",\"failure.rs\"],\"files_examined\":[\"orchestrator.rs\",\"failure.rs\"],\"key_findings\":[\"The orchestrator owns the main loop and continuation handling.\"],\"design_patterns\":[\"Event-driven orchestration\"],\"concerns\":[\"Failure policy remains centralized.\"],\"confidence\":\"medium\"}</explore_report>"
        ));
    }

    #[test]
    fn synthesized_explorer_output_replaces_placeholder_with_report() {
        let files = vec!["src/lib.rs".to_string(), "src/main.rs".to_string()];

        let output =
            synthesized_explorer_output("explorer", "Let me inspect a few files first.", &files);

        assert!(output.contains("<explore_report>"));
        assert!(output.contains("src/lib.rs"));
        assert!(output.contains("src/main.rs"));
    }

    #[test]
    fn synthesized_explorer_output_preserves_substantive_summary() {
        let substantive = "<explore_report>{\"summary\":\"The module centers runtime orchestration in orchestrator.rs.\",\"paths_examined\":[\"orchestrator.rs\"],\"files_examined\":[\"orchestrator.rs\"],\"key_findings\":[\"The orchestrator owns the main loop.\"],\"design_patterns\":[\"Event-driven orchestration\"],\"concerns\":[],\"confidence\":\"medium\"}</explore_report>";

        assert_eq!(
            synthesized_explorer_output("explorer", substantive, &["orchestrator.rs".to_string()]),
            substantive
        );
    }

    #[test]
    fn timeout_partial_output_is_agent_neutral() {
        let output =
            timeout_partial_output("", &["src/lib.rs".to_string(), "src/main.rs".to_string()]);

        assert!(output.starts_with("Sub-agent timed out before producing final output."));
        assert!(output.contains("src/lib.rs"));
        assert!(!output.contains("Explorer timed out"));
    }
}
