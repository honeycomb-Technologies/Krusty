//! Processes tool - Manage background processes

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::process::{ProcessInfo, ProcessStatus};
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct ProcessesTool;

const MODEL_OUTPUT_TAIL_CHARS: usize = 8_000;

fn bounded_output_for_model(output: String, already_truncated: bool) -> (String, bool) {
    if output.chars().count() <= MODEL_OUTPUT_TAIL_CHARS {
        return (output, already_truncated);
    }

    let mut recent = output
        .chars()
        .rev()
        .take(MODEL_OUTPUT_TAIL_CHARS)
        .collect::<Vec<_>>();
    recent.reverse();
    (recent.into_iter().collect(), true)
}

fn process_error(process: &ProcessInfo) -> Option<&str> {
    match &process.status {
        ProcessStatus::Failed { error, .. } => Some(error.as_str()),
        _ => None,
    }
}

fn process_exit_code(process: &ProcessInfo) -> Option<i32> {
    match process.status {
        ProcessStatus::Completed { exit_code, .. } => Some(exit_code),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Params {
    action: String,
    #[serde(default)]
    process_id: Option<String>,
}

#[async_trait]
impl Tool for ProcessesTool {
    fn name(&self) -> &str {
        "processes"
    }

    fn description(&self) -> &str {
        "Manage background processes. Actions: list (show all), kill (stop by ID), status (check by ID and read its recent output)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "kill", "status"],
                    "description": "Action to perform"
                },
                "process_id": {
                    "type": "string",
                    "description": "Process ID (required for kill/status)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let Some(registry) = &ctx.process_registry else {
            return ToolResult::error("Process registry not available");
        };

        // Delegated runtimes receive a task-scoped process owner while normal
        // parent sessions retain user-scoped process isolation.
        let user_id = ctx.effective_process_owner_id();

        match params.action.as_str() {
            "list" => {
                let processes = match user_id {
                    Some(uid) => registry.list_for_user(uid).await,
                    None => registry.list().await,
                };
                let output: Vec<Value> = processes
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id,
                            "command": p.command,
                            "description": p.description,
                            "status": p.display_status(),
                            "duration_seconds": p.duration().as_secs(),
                            "pid": p.pid,
                            "error": process_error(p),
                            "exit_code": process_exit_code(p),
                        })
                    })
                    .collect();

                ToolResult::success_data(json!({
                    "processes": output,
                    "count": processes.len()
                }))
            }
            "kill" => {
                let Some(id) = params.process_id else {
                    return ToolResult::invalid_parameters("process_id required for kill");
                };

                let result = match user_id {
                    Some(uid) => registry.kill_for_user(uid, &id).await,
                    None => registry.kill(&id).await,
                };

                match result {
                    Ok(_) => ToolResult::success_data(json!({
                        "success": true,
                        "message": "Process killed",
                        "process_id": id
                    })),
                    Err(e) => ToolResult::error(e.to_string()),
                }
            }
            "status" => {
                let Some(id) = params.process_id else {
                    return ToolResult::invalid_parameters("process_id required for status");
                };

                let process = match user_id {
                    Some(uid) => registry.get_for_user(uid, &id).await,
                    None => registry.get(&id).await,
                };

                match process {
                    Some(p) => {
                        let output = match user_id {
                            Some(uid) => registry.output_for_user(uid, &id).await,
                            None => registry.output(&id).await,
                        }
                        .unwrap_or_default();
                        let (output_tail, output_truncated) =
                            bounded_output_for_model(output.0, output.1);
                        ToolResult::success_data(json!({
                            "id": p.id,
                            "status": p.display_status(),
                            "command": p.command,
                            "duration_seconds": p.duration().as_secs(),
                            "error": process_error(&p),
                            "exit_code": process_exit_code(&p),
                            "output_tail": output_tail,
                            "output_truncated": output_truncated,
                        }))
                    }
                    None => ToolResult::error("Process not found"),
                }
            }
            _ => ToolResult::invalid_parameters("Unknown action. Use 'list', 'kill', or 'status'"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_output_for_model, MODEL_OUTPUT_TAIL_CHARS};

    #[test]
    fn model_output_tail_is_bounded_without_splitting_unicode() {
        let output = format!("old:{}recent", "🦀".repeat(MODEL_OUTPUT_TAIL_CHARS));
        let (tail, truncated) = bounded_output_for_model(output, false);

        assert!(truncated);
        assert_eq!(tail.chars().count(), MODEL_OUTPUT_TAIL_CHARS);
        assert!(tail.ends_with("recent"));
    }
}
