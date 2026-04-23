//! Bash tool - Execute shell commands with real-time output streaming.

mod execution;
mod shell;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;

use crate::tools::registry::Tool;
use crate::tools::truncation;
use crate::tools::{parse_params, ToolContext, ToolResult};

use execution::{execute_background, execute_foreground, StreamContext};
use shell::{
    build_shell_command, configure_foreground_process_group, strip_ansi,
    strip_shell_background_suffix,
};

pub(super) const MAX_OUTPUT_LINES: usize = 2000;
pub(super) const MAX_OUTPUT_BYTES: usize = 50_000; // 50KB

// Bounded raw capture for foreground execution. Final model output is additionally
// truncated by MAX_OUTPUT_LINES/MAX_OUTPUT_BYTES after ANSI stripping.
pub(super) const RAW_CAPTURE_MAX_LINES: usize = 8_000;
pub(super) const RAW_CAPTURE_MAX_BYTES: usize = 2_000_000; // 2MB
pub(super) const READER_JOIN_TIMEOUT_MS: u64 = 2_000;
pub(super) const TIMEOUT_KILL_GRACE_MS: u64 = 800;

pub struct BashTool;

#[derive(Deserialize)]
struct Params {
    command: String,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    run_in_background: Option<bool>,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute shell commands for git, build tools (cargo/bun/make), and system utilities. \
         For file operations use specialized tools: Read, Write, Edit, Glob, Grep. \
         Set run_in_background:true for servers/watchers."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Do not use bash for file reading (cat/head/tail), editing (sed/awk), or searching (grep/find/rg) — use the dedicated Read, Edit, Grep, and Glob tools instead. Bash is for git, build systems, package managers, compilers, and system utilities.

Chain dependent commands with `&&`. For independent commands, make parallel tool calls instead of chaining. Never use trailing `&` for background processes — set `run_in_background:true` instead.

Default timeout is 30 seconds. Set `timeout` explicitly for long-running commands (max 600000ms / 10 minutes). For servers, watchers, and long builds, use `run_in_background:true`.

Always include a `description` parameter with a clear 5-10 word summary of what the command does (e.g., "Install npm dependencies", "Run test suite"). This is used for logging and progress display.

Use absolute paths for file arguments. The working directory resets between calls, so `cd` is unreliable — prefer absolute paths or chain `cd dir && command`.

Avoid interactive commands (requiring stdin). Avoid `sudo` unless the user explicitly requests it. Prefer `--yes`/`-y` flags for package managers to avoid interactive prompts."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in milliseconds (max 600000)"
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does in 5-10 words"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Set to true to run this command in the background"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        match &params.description {
            Some(desc) => {
                tracing::info!(command = %params.command, description = %desc, "Executing bash command")
            }
            None => tracing::info!(command = %params.command, "Executing bash command"),
        }

        if let Some(ref sandbox) = ctx.sandbox_root {
            let canonical = match ctx.working_dir.canonicalize() {
                Ok(c) => c,
                Err(_) => {
                    return ToolResult::error(
                        "Access denied: cannot verify working directory".to_string(),
                    );
                }
            };
            if !canonical.starts_with(sandbox) {
                return ToolResult::error(
                    "Access denied: working directory is outside workspace".to_string(),
                );
            }
        }

        let effective_command = if let Some(ref identity) = ctx.git_identity {
            identity.apply_to_command(&params.command)
        } else {
            params.command.clone()
        };

        let inferred_background_command = strip_shell_background_suffix(&effective_command);
        let inferred_from_shell_suffix = inferred_background_command.is_some();

        if params.run_in_background.unwrap_or(false) || inferred_from_shell_suffix {
            let clean_command =
                inferred_background_command.unwrap_or_else(|| effective_command.clone());
            let warnings = if inferred_from_shell_suffix {
                vec![
                    "Background mode inferred from trailing '&'; prefer run_in_background:true for clarity."
                        .to_string(),
                ]
            } else {
                Vec::new()
            };

            if let Some(ref registry) = ctx.process_registry {
                let spawn_result = match ctx.user_id.as_deref() {
                    Some(uid) => {
                        registry
                            .spawn_for_user(
                                uid,
                                clean_command.clone(),
                                ctx.working_dir.clone(),
                                params.description.clone(),
                            )
                            .await
                    }
                    None => {
                        registry
                            .spawn(
                                clean_command.clone(),
                                ctx.working_dir.clone(),
                                params.description.clone(),
                            )
                            .await
                    }
                };
                match spawn_result {
                    Ok(process_id) => {
                        return ToolResult::success_data_with(
                            json!({
                                "message": "Process started in background",
                                "process_id": process_id,
                                "status": "running"
                            }),
                            warnings,
                            None,
                            None,
                        );
                    }
                    Err(e) => {
                        return ToolResult::error(format!("Failed to start: {}", e));
                    }
                }
            } else {
                let background_cmd = build_shell_command(&clean_command, ctx);
                return execute_background(background_cmd, warnings).await;
            }
        }

        let mut cmd = build_shell_command(&effective_command, ctx);
        configure_foreground_process_group(&mut cmd);
        cmd.kill_on_drop(true);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let timeout_ms = params.timeout.unwrap_or(30_000).min(600_000);
        let timeout_duration = Duration::from_millis(timeout_ms);

        let stream = match (ctx.output_tx.as_ref(), ctx.tool_use_id.as_ref()) {
            (Some(tx), Some(id)) => Some(StreamContext {
                output_tx: tx.clone(),
                tool_use_id: id.clone(),
            }),
            (None, None) => None,
            _ => return ToolResult::error("Streaming context incomplete for bash tool"),
        };

        execute_foreground(cmd, timeout_duration, stream).await
    }
}

/// Apply ANSI stripping and truncation to the final output sent to the AI model.
fn process_output(combined: String) -> String {
    let stripped = strip_ansi(&combined);
    let result = truncation::truncate_tail(&stripped, MAX_OUTPUT_LINES, MAX_OUTPUT_BYTES);
    if let Some(notice) = result.notice() {
        format!("{}{}", result.text, notice)
    } else {
        result.text
    }
}
