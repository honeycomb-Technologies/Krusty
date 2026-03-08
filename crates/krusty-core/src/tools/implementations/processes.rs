//! Processes tool - Manage background processes

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct ProcessesTool;

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
        "Manage background processes. Actions: list (show all), kill (stop by ID), status (check by ID)."
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

        // Use user_id for multi-tenant isolation when available
        let user_id = ctx.user_id.as_deref();

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
                    Some(p) => ToolResult::success_data(json!({
                        "id": p.id,
                        "status": p.display_status(),
                        "command": p.command,
                        "duration_seconds": p.duration().as_secs(),
                    })),
                    None => ToolResult::error("Process not found"),
                }
            }
            _ => ToolResult::invalid_parameters("Unknown action. Use 'list', 'kill', or 'status'"),
        }
    }
}
