//! Bash tool - Execute shell commands with real-time output streaming.

mod execution;
mod shell;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
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

fn output_spool_path(ctx: &ToolContext) -> PathBuf {
    let session = ctx
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("local")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    ctx.working_dir
        .join(".krusty")
        .join("tool-output")
        .join(session)
        .join(format!("tool_{}.log", uuid::Uuid::new_v4()))
}

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
            r#"Use bash for git, builds, package managers, compilers, servers/watchers, and system utilities. Use dedicated tools for routine file read/search/edit operations.

Chain dependent commands with `&&`; run independent commands as separate parallel tool calls. Use `run_in_background:true` for servers/watchers instead of a trailing `&`.

Default timeout is 30 seconds; set `timeout` explicitly for long-running commands (max 600000ms). Include a concise `description` for logging/progress.

Relative paths resolve from the tool working directory; chain `cd dir && command` when a command must run from a specific directory. Avoid interactive commands; avoid `sudo` unless explicitly requested.

If a validation/preflight command fails with actionable file diagnostics (for example `git diff --check` reporting trailing whitespace), fix the reported files with the dedicated edit tools, re-stage affected paths when needed, then rerun the check. If `git diff --cached --check` fails, remember that the staged index is stale until you run `git add <reported-path>` after the edit. Do not repeat the same failing command unchanged."#,
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
                    "type": "integer",
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
                    "Access denied: working directory is outside the configured filesystem access root".to_string(),
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

        execute_foreground(cmd, timeout_duration, stream, output_spool_path(ctx)).await
    }
}

/// Apply ANSI stripping and truncation to the final output sent to the AI model.
fn process_output(combined: String, full_output_path: Option<&std::path::Path>) -> String {
    let stripped = strip_ansi(&combined);
    let result = truncation::truncate_tail(&stripped, MAX_OUTPUT_LINES, MAX_OUTPUT_BYTES);
    if let Some(notice) = result.notice() {
        let recovery = full_output_path
            .map(|path| format!(" Full output saved to {}.", path.display()))
            .unwrap_or_default();
        format!("{}{}{}", result.text, notice, recovery)
    } else {
        result.text
    }
}
