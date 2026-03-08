//! Tool execution for the agentic loop.
//!
//! Handles:
//! - Permission-based approval workflow (supervised mode)
//! - Special tool dispatch (mode switch, plan tasks)
//! - Regular tool execution via `ToolRegistry::execute()`
//! - Output truncation
//! - Tool output streaming via `ToolOutputChunk` → `LoopEvent::ToolOutputDelta`

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::types::{AiToolCall, Content};
use crate::process::ProcessRegistry;
use crate::storage::WorkMode;
use crate::tools::registry::{
    tool_category, PermissionMode, ToolCategory, ToolContext, ToolRegistry,
};

use super::loop_events::{LoopEvent, LoopInput};
use super::plan_handler;

const MAX_TOOL_OUTPUT_CHARS: usize = 30_000;
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Execute a batch of tool calls, emitting LoopEvents and receiving LoopInputs
/// for the approval workflow.
///
/// Returns `(tool_results, next_work_mode)`.
pub(crate) async fn execute_tools(
    tool_calls: &[AiToolCall],
    tool_registry: &Arc<ToolRegistry>,
    working_dir: &Path,
    process_registry: &Arc<ProcessRegistry>,
    session_id: &str,
    db_path: &Path,
    user_id: Option<&str>,
    permission_mode: PermissionMode,
    current_mode: WorkMode,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
) -> (Vec<Content>, WorkMode) {
    let mut work_mode = current_mode;
    let mut results = Vec::new();

    for call in tool_calls {
        let category = tool_category(&call.name);

        // ── Supervised approval ────────────────────────────────────
        if permission_mode == PermissionMode::Supervised && category == ToolCategory::Write {
            let _ = event_tx.send(LoopEvent::ToolApprovalRequired {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });

            let approved = wait_for_approval(call, event_tx, input_rx).await;

            if !approved {
                let denied = crate::tools::registry::ToolResult::error_with_code(
                    "permission_denied",
                    "Tool execution denied by user",
                );
                let output = truncate_output(&denied.output);
                let _ = event_tx.send(LoopEvent::ToolDenied {
                    id: call.id.clone(),
                });
                let _ = event_tx.send(LoopEvent::ToolResult {
                    id: call.id.clone(),
                    output: output.clone(),
                    is_error: denied.is_error,
                });
                results.push(Content::ToolResult {
                    tool_use_id: call.id.clone(),
                    output: serde_json::Value::String(output),
                    is_error: Some(denied.is_error),
                });
                continue;
            }

            let _ = event_tx.send(LoopEvent::ToolApproved {
                id: call.id.clone(),
            });
        }

        let _ = event_tx.send(LoopEvent::ToolExecuting {
            id: call.id.clone(),
            name: call.name.clone(),
        });

        // ── Mode switch tools ──────────────────────────────────────
        if call.name == "set_work_mode" || call.name == "enter_plan_mode" {
            let switch = plan_handler::handle_mode_switch(call, session_id, db_path, work_mode);
            work_mode = switch.next_mode;

            if let Some(reason) = switch.mode_change_reason {
                let _ = event_tx.send(LoopEvent::ModeChange {
                    mode: work_mode.to_string(),
                    reason: Some(reason),
                });
            }

            let output = truncate_output(&switch.tool_result.output);
            let _ = event_tx.send(LoopEvent::ToolResult {
                id: call.id.clone(),
                output: output.clone(),
                is_error: switch.tool_result.is_error,
            });
            results.push(Content::ToolResult {
                tool_use_id: call.id.clone(),
                output: serde_json::Value::String(output),
                is_error: if switch.tool_result.is_error {
                    Some(true)
                } else {
                    None
                },
            });
            continue;
        }

        // ── Plan task tools ────────────────────────────────────────
        if matches!(
            call.name.as_str(),
            "task_start" | "task_complete" | "add_subtask" | "set_dependency"
        ) {
            let result = plan_handler::handle_plan_task(call, session_id, db_path);
            let output = truncate_output(&result.output);
            let _ = event_tx.send(LoopEvent::ToolResult {
                id: call.id.clone(),
                output: output.clone(),
                is_error: result.is_error,
            });
            results.push(Content::ToolResult {
                tool_use_id: call.id.clone(),
                output: serde_json::Value::String(output),
                is_error: if result.is_error { Some(true) } else { None },
            });
            continue;
        }

        // ── Regular tool execution ─────────────────────────────────
        let (output_tx, mut output_rx) =
            mpsc::unbounded_channel::<crate::tools::registry::ToolOutputChunk>();

        let forwarder_event_tx = event_tx.clone();
        let forwarder_tool_id = call.id.clone();
        let forwarder_tool_name = call.name.clone();
        let forwarder_handle = tokio::spawn(async move {
            let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            heartbeat_interval.tick().await;

            loop {
                tokio::select! {
                    chunk = output_rx.recv() => {
                        match chunk {
                            Some(chunk) => {
                                if !chunk.chunk.is_empty() {
                                    let _ = forwarder_event_tx.send(LoopEvent::ToolOutputDelta {
                                        id: forwarder_tool_id.clone(),
                                        delta: chunk.chunk,
                                    });
                                }
                                if chunk.is_complete {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    _ = heartbeat_interval.tick() => {
                        let _ = forwarder_event_tx.send(LoopEvent::ToolExecuting {
                            id: forwarder_tool_id.clone(),
                            name: forwarder_tool_name.clone(),
                        });
                    }
                }
            }
        });

        let ctx = ToolContext {
            working_dir: working_dir.to_path_buf(),
            process_registry: Some(process_registry.clone()),
            plan_mode: work_mode == WorkMode::Plan,
            user_id: user_id.map(ToString::to_string),
            sandbox_root: Some(working_dir.to_path_buf()),
            ..Default::default()
        }
        .with_output_stream(output_tx, call.id.clone());

        let result = tool_registry
            .execute(&call.name, call.arguments.clone(), &ctx)
            .await
            .unwrap_or_else(|| {
                crate::tools::registry::ToolResult::error_with_code(
                    "unknown_tool",
                    format!("Unknown tool: {}", call.name),
                )
            });

        drop(ctx);
        let _ = forwarder_handle.await;

        let output = truncate_output(&result.output);

        let _ = event_tx.send(LoopEvent::ToolResult {
            id: call.id.clone(),
            output: output.clone(),
            is_error: result.is_error,
        });

        results.push(Content::ToolResult {
            tool_use_id: call.id.clone(),
            output: serde_json::Value::String(output),
            is_error: if result.is_error { Some(true) } else { None },
        });
    }

    (results, work_mode)
}

/// Wait for a tool approval via the LoopInput channel.
async fn wait_for_approval(
    call: &AiToolCall,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
) -> bool {
    let deadline = tokio::time::Instant::now() + APPROVAL_TIMEOUT;

    loop {
        match tokio::time::timeout_at(deadline, input_rx.recv()).await {
            Ok(Some(LoopInput::ToolApproval {
                tool_call_id,
                approved,
            })) if tool_call_id == call.id => {
                return approved;
            }
            Ok(Some(LoopInput::Cancel)) => return false,
            Ok(Some(_)) => continue,  // ignore unrelated inputs
            Ok(None) => return false, // channel closed
            Err(_) => {
                let timeout_result = crate::tools::registry::ToolResult::error_with_code(
                    "timeout",
                    "Tool approval timed out after 5 minutes",
                );
                // Timeout
                let _ = event_tx.send(LoopEvent::ToolDenied {
                    id: call.id.clone(),
                });
                let _ = event_tx.send(LoopEvent::ToolResult {
                    id: call.id.clone(),
                    output: timeout_result.output,
                    is_error: timeout_result.is_error,
                });
                return false;
            }
        }
    }
}

pub(crate) fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_TOOL_OUTPUT_CHARS {
        return output.to_string();
    }

    let truncated_len = floor_char_boundary(output, MAX_TOOL_OUTPUT_CHARS);
    let truncated = &output[..truncated_len];
    let break_point = truncated.rfind('\n').unwrap_or(truncated_len);
    let clean = &output[..break_point];
    format!(
        "{}\n\n[... OUTPUT TRUNCATED: {} chars -> {} chars ...]",
        clean,
        output.len(),
        clean.len()
    )
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut boundary = index.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}
