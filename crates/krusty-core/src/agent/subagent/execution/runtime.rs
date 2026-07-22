use std::collections::HashSet;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::compaction::is_context_overflow_error;
use crate::agent::constants::subagent;
use crate::agent::history_policy::build_history_tool_result;
use crate::agent::progress::LoopGuard;
use crate::agent::RunProvenance;
use crate::ai::client::AiClient;
use crate::ai::types::{AiToolCall, Content, ModelMessage, Role};
use crate::tools::ToolResult;

use super::super::types::{
    parse_explore_report, AgentProgress, AgentProgressStatus, DelegatedProcessArtifact,
    SubAgentResult, SubAgentTask,
};
use super::api::{call_subagent_api, parse_response, parse_response_usage};
use super::config::AgentConfig;
use super::explorer::{
    collect_paths_from_tool_result, completion_summary_preview, normalize_explorer_result,
    relative_or_display, synthesized_explorer_output, text_claims_tool_empty,
    timeout_partial_output, tool_result_has_positive_evidence,
};
use super::governance::{build_subagent_tool_context, delegated_is_explore, delegated_turn_budget};

const MAX_DELEGATED_POLICY_VIOLATIONS: usize = 3;
const EXPLORER_STALE_SEQUENCE_THRESHOLD: usize = 3;
const EXPLORER_SYNTHESIS_FILE_THRESHOLD: usize = 8;

fn delegated_process_artifact(
    tool_name: &str,
    input: &serde_json::Value,
    result: &ToolResult,
    working_dir: &std::path::Path,
) -> Option<DelegatedProcessArtifact> {
    if tool_name != "bash" || result.is_error {
        return None;
    }

    let parsed = serde_json::from_str::<serde_json::Value>(&result.output).ok()?;
    let payload = parsed.get("data").unwrap_or(&parsed);
    let process_id = payload.get("process_id")?.as_str()?.trim();
    if process_id.is_empty() {
        return None;
    }
    let status = payload
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let command = input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let endpoint_hints = payload
        .get("endpoint_hints")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(DelegatedProcessArtifact {
        process_id: process_id.to_string(),
        status,
        command,
        working_dir: working_dir.display().to_string(),
        endpoint_hints,
        reused_existing: payload
            .get("reused_existing")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

fn record_delegated_process(
    processes: &mut Vec<DelegatedProcessArtifact>,
    process: DelegatedProcessArtifact,
) {
    if let Some(existing) = processes
        .iter_mut()
        .find(|existing| existing.process_id == process.process_id)
    {
        *existing = process;
    } else {
        processes.push(process);
    }
}

fn compact_delegated_history(
    messages: &mut Vec<ModelMessage>,
    files_examined: &[String],
    final_output: &str,
    trigger: &str,
) -> bool {
    if messages.len() <= 3 {
        return false;
    }

    // Retain a bounded complete tail beginning at an assistant turn so tool
    // result messages never survive without their corresponding tool calls.
    let mut tail_start = messages.len().saturating_sub(16).max(1);
    while tail_start < messages.len() && !matches!(messages[tail_start].role, Role::Assistant) {
        tail_start += 1;
    }
    if tail_start <= 1 {
        return false;
    }

    let removed = messages.drain(1..tail_start).count();
    let paths = files_examined
        .iter()
        .take(32)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let last_output = final_output.chars().take(2_000).collect::<String>();
    messages.insert(
        1,
        ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: format!(
                    "[DELEGATED COMPACTION CHECKPOINT]\nTrigger: {trigger}\nCompacted messages: {removed}\nEvidence paths: {paths}\nLatest synthesis: {last_output}\nContinue the original objective from this checkpoint; do not repeat completed work.\n[/DELEGATED COMPACTION CHECKPOINT]"
                ),
            }],
        },
    );
    true
}

/// Delegated agents use a specialized non-streaming transport, while sharing
/// canonical governance, semantic progress, failure, compaction, and history
/// policies with the parent loop.
pub(crate) async fn execute_agent_loop<C: AgentConfig>(
    client: &AiClient,
    task: &SubAgentTask,
    model: &str,
    cancellation: CancellationToken,
    config: &C,
    progress_tx: Option<mpsc::UnboundedSender<AgentProgress>>,
) -> SubAgentResult {
    let provenance = RunProvenance::Delegated;
    info!(
        surface = provenance.as_str(),
        kernel = provenance.kernel().as_str(),
        task_id = %task.id,
        delegated_run_id = ?task.delegated_run_id,
        "Starting delegated agent kernel"
    );
    let start = Instant::now();
    let task_id = task.id.clone();
    let task_name = task.display_name();
    let plan_task_id = task.plan_task_id.clone();
    let cache_session_id = task
        .delegated_run_id
        .as_deref()
        .map(|run_id| format!("{run_id}:{task_id}"))
        .unwrap_or_else(|| task_id.clone());

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
    let mut background_processes = Vec::new();
    let mut loop_guard = LoopGuard::new();
    let mut overflow_compact_retry_attempted = false;
    let mut last_dynamic_context: Option<String> = None;

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
                identity: task.identity.clone(),
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
                background_processes: background_processes.clone(),
            };
        }

        let max_turns_budget = delegated_turn_budget(task);
        if let Some(mailbox) = task.mailbox.as_ref() {
            let parent_messages = mailbox.drain();
            if !parent_messages.is_empty() {
                loop_guard.reset_for_steering();
            }
            for message in parent_messages {
                messages.push(ModelMessage {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: format!("[PARENT MESSAGE]\n{message}\n[/PARENT MESSAGE]"),
                    }],
                });
            }
        }

        if let Some(max_turns) = max_turns_budget {
            if turns >= max_turns {
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
                    background_processes: background_processes.clone(),
                };
            }
        }
        turns += 1;

        if let Some(context) = config
            .dynamic_context()
            .filter(|context| !context.trim().is_empty())
        {
            if last_dynamic_context.as_deref() != Some(context.as_str()) {
                messages.push(ModelMessage {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: context.clone(),
                    }],
                });
                last_dynamic_context = Some(context);
            }
        }

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

        let provider_call_started = Instant::now();
        let api_future = call_subagent_api(
            client,
            model,
            &system_prompt,
            &messages,
            &ai_tools,
            config.max_tokens(),
            task.thinking_enabled,
            &cache_session_id,
        );

        let api_result = tokio::time::timeout(config.api_call_timeout(), api_future).await;
        let (provider_outcome, provider_usage) = match &api_result {
            Ok(Ok(response)) => ("completed", parse_response_usage(response)),
            Ok(Err(_)) => ("error", None),
            Err(_) => ("timeout", None),
        };
        if let Some(trace) = task.provider_call_trace.as_ref() {
            trace
                .record_delegated_call(
                    "delegated_agent_turn",
                    client.provider_id(),
                    model,
                    task.delegated_run_id.as_deref(),
                    &task_id,
                    turns,
                    provider_call_started,
                    provider_outcome,
                    provider_usage.clone(),
                )
                .await;
        }

        let response = match api_result {
            Ok(Ok(r)) => r,
            Ok(Err(e))
                if !overflow_compact_retry_attempted
                    && is_context_overflow_error(&e.to_string())
                    && compact_delegated_history(
                        &mut messages,
                        &files_examined,
                        &final_output,
                        "provider_overflow",
                    ) =>
            {
                overflow_compact_retry_attempted = true;
                turns = turns.saturating_sub(1);
                warn!(
                    task_id = %task_id,
                    "Provider rejected delegated context; compacted in place and retrying once"
                );
                send_progress(
                    AgentProgressStatus::Running,
                    "compacted context; retrying",
                    total_tool_calls,
                    estimated_tokens,
                    completion_summary_preview(&final_output),
                    config,
                );
                continue;
            }
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
                    background_processes: background_processes.clone(),
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
                    background_processes: background_processes.clone(),
                };
            }
        };

        if let Some(usage) = provider_usage {
            estimated_tokens = estimated_tokens.saturating_add(usage.logical_total_tokens());
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
                    background_processes: background_processes.clone(),
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
                background_processes: background_processes.clone(),
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

        let is_explore_delegation = delegated_is_explore(task);
        let all_read_only_tools = is_explore_delegation
            && tool_calls
                .iter()
                .all(|call| matches!(call.name.as_str(), "read" | "glob" | "grep" | "list"));

        if config.use_explorer_heuristics()
            && is_explore_delegation
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
                    background_processes: background_processes.clone(),
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

        let mut tool_results: Vec<Content> = vec![];
        let mut cycle_positive_evidence = false;

        for tc in &tool_calls {
            total_tool_calls += 1;
            if let Some(policy) = task.delegation_policy.as_ref() {
                if let Err(reason) = policy.authorize_tool_call(&tc.name, &tc.input, ctx.plan_mode)
                {
                    let violation = format!("{}: {}", tc.name, reason);
                    policy_violations.push(violation.clone());
                    tool_results.push(Content::ToolResult {
                        tool_use_id: tc.id.clone(),
                        output: build_history_tool_result(
                            &tc.name,
                            &crate::tools::registry::ToolResult::error_with_details(
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
                            background_processes: background_processes.clone(),
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
                Some(r) => {
                    if let Some(process) =
                        delegated_process_artifact(&tc.name, &tc.input, &r, &task.working_dir)
                    {
                        record_delegated_process(&mut background_processes, process);
                    }
                    (r.output, r.is_error)
                }
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

        let progress_calls = tool_calls
            .iter()
            .map(|call| AiToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.input.clone(),
            })
            .collect::<Vec<_>>();
        let guard = loop_guard.evaluate(&progress_calls, &tool_results);
        let progress_telemetry = guard.progress;
        let guard_diagnostic = guard
            .repeated_failure
            .or(guard.repeated_validation)
            .or_else(|| {
                progress_telemetry
                    .as_ref()
                    .and_then(|telemetry| telemetry.diagnostic())
            });

        messages.push(ModelMessage {
            role: Role::User,
            content: tool_results,
        });
        if let Some(instruction) = progress_telemetry
            .as_ref()
            .and_then(|telemetry| telemetry.replan_instruction())
        {
            messages.push(ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: instruction.to_string(),
                }],
            });
        }
        if let Some(diagnostic) = guard_diagnostic {
            warn!(
                task_id = %task_id,
                turns,
                diagnostic = %diagnostic,
                "Delegated semantic progress guard stopped the run"
            );
            send_progress(
                AgentProgressStatus::Failed,
                "no semantic progress",
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
                error: Some(diagnostic),
                policy_violations,
                background_processes: background_processes.clone(),
            };
        }
        last_cycle_positive_evidence = cycle_positive_evidence;

        if config.use_explorer_heuristics() && is_explore_delegation && all_read_only_tools {
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
                        background_processes: background_processes.clone(),
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

        if messages.len() > subagent::MAX_MESSAGES
            && compact_delegated_history(
                &mut messages,
                &files_examined,
                &final_output,
                "history_budget",
            )
        {
            tracing::debug!(
                task_id = %task_id,
                remaining = messages.len(),
                "Compacted delegated history in place"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegated_compaction_keeps_objective_checkpoint_and_complete_tail() {
        let mut messages = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "original objective".to_string(),
            }],
        }];
        for index in 0..20 {
            messages.push(ModelMessage {
                role: if index % 2 == 0 {
                    Role::Assistant
                } else {
                    Role::User
                },
                content: vec![Content::Text {
                    text: format!("turn {index}"),
                }],
            });
        }
        let original_len = messages.len();

        assert!(compact_delegated_history(
            &mut messages,
            &["src/lib.rs".to_string()],
            "latest finding",
            "test",
        ));
        assert!(messages.len() < original_len);
        assert!(matches!(messages[0].role, Role::User));
        assert!(matches!(messages[1].role, Role::User));
        assert!(matches!(messages[2].role, Role::Assistant));
        let checkpoint = match &messages[1].content[0] {
            Content::Text { text } => text,
            _ => panic!("checkpoint must be text"),
        };
        assert!(checkpoint.contains("DELEGATED COMPACTION CHECKPOINT"));
        assert!(checkpoint.contains("src/lib.rs"));
        assert!(checkpoint.contains("latest finding"));
    }
}
