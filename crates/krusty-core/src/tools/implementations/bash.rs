//! Bash tool - Execute shell commands with real-time output streaming

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use crate::tools::registry::{Tool, ToolOutputChunk};
use crate::tools::{parse_params, ToolContext, ToolResult};

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
        "Execute shell commands for git, build tools (cargo/npm/make), and system utilities. \
         For file operations use specialized tools: Read, Write, Edit, Glob, Grep. \
         Set run_in_background:true for servers/watchers."
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

        // Validate working_dir is within sandbox (multi-tenant isolation)
        if let Some(ref sandbox) = ctx.sandbox_root {
            if let Ok(canonical) = ctx.working_dir.canonicalize() {
                if !canonical.starts_with(sandbox) {
                    return ToolResult::error(
                        "Access denied: working directory is outside workspace".to_string(),
                    );
                }
            }
        }

        // Build command based on platform
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&params.command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&params.command);
            c
        };

        cmd.current_dir(&ctx.working_dir);

        // Detect shell background syntax (command ending with &)
        // Treat this the same as run_in_background: true
        let command_trimmed = params.command.trim();
        let is_shell_backgrounded =
            command_trimmed.ends_with('&') && !command_trimmed.ends_with("&&"); // Don't match && (logical AND)

        // Handle background execution (explicit param OR shell & syntax)
        if params.run_in_background.unwrap_or(false) || is_shell_backgrounded {
            // Strip trailing & if present (we'll handle backgrounding ourselves)
            let clean_command = if is_shell_backgrounded {
                command_trimmed.trim_end_matches('&').trim().to_string()
            } else {
                params.command.clone()
            };
            // Use process registry if available for tracking
            if let Some(ref registry) = ctx.process_registry {
                // Use user_id for multi-tenant isolation
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
                        return ToolResult::success(
                            json!({
                                "output": "Process started in background",
                                "processId": process_id,
                                "status": "running"
                            })
                            .to_string(),
                        );
                    }
                    Err(e) => {
                        return ToolResult::error(format!("Failed to start: {}", e));
                    }
                }
            } else {
                // Fallback to legacy background execution without tracking
                return execute_background(cmd).await;
            }
        }

        // Foreground execution with streaming output
        cmd.kill_on_drop(true);
        cmd.stdin(Stdio::null()); // Prevent hanging on input
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let timeout_ms = params.timeout.unwrap_or(30_000).min(600_000); // 30s default
        let timeout_duration = Duration::from_millis(timeout_ms);

        // Check if we have streaming output channel
        let has_streaming = ctx.output_tx.is_some() && ctx.tool_use_id.is_some();

        if has_streaming {
            // Streaming mode: spawn and read lines incrementally
            execute_streaming(cmd, timeout_duration, ctx).await
        } else {
            // Legacy mode: wait for completion
            execute_blocking(cmd, timeout_duration).await
        }
    }
}

/// Execute command with real-time output streaming
async fn execute_streaming(
    mut cmd: Command,
    timeout_duration: Duration,
    ctx: &ToolContext,
) -> ToolResult {
    let output_tx = ctx.output_tx.as_ref().unwrap();
    let tool_use_id = ctx.tool_use_id.as_ref().unwrap().clone();

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("Failed to spawn command: {}", e)),
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mut combined_output = String::new();
    const MAX_OUTPUT_SIZE: usize = 10_000_000; // 10MB cap to prevent unbounded growth

    // Spawn tasks to read stdout and stderr concurrently
    let stdout_tx = output_tx.clone();
    let stdout_id = tool_use_id.clone();
    let stdout_handle = tokio::spawn(async move {
        let mut lines = Vec::new();
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                // Send line to UI immediately
                let _ = stdout_tx.send(ToolOutputChunk {
                    tool_use_id: stdout_id.clone(),
                    chunk: format!("{}\n", line),
                    is_complete: false,
                    exit_code: None,
                });
                lines.push(line);
            }
        }
        lines.join("\n")
    });

    let stderr_tx = output_tx.clone();
    let stderr_id = tool_use_id.clone();
    let stderr_handle = tokio::spawn(async move {
        let mut lines = Vec::new();
        if let Some(stderr) = stderr {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                // Send line to UI immediately (stderr)
                let _ = stderr_tx.send(ToolOutputChunk {
                    tool_use_id: stderr_id.clone(),
                    chunk: format!("{}\n", line),
                    is_complete: false,
                    exit_code: None,
                });
                lines.push(line);
            }
        }
        lines.join("\n")
    });

    // Wait for process with timeout
    let wait_result = timeout(timeout_duration, child.wait()).await;

    let (exit_code, killed) = match wait_result {
        Ok(Ok(status)) => {
            // Check for normal exit code first
            if let Some(code) = status.code() {
                (code, false)
            } else {
                // Process was killed by signal (Unix)
                // On Unix, check if it was a user-initiated signal (SIGINT=2, SIGTERM=15)
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    match status.signal() {
                        Some(2) | Some(15) => (0, false), // SIGINT/SIGTERM = user closed, treat as success
                        Some(sig) => {
                            tracing::debug!("Process killed by signal {}", sig);
                            (128 + sig, false) // Convention: 128 + signal number
                        }
                        None => (-1, false),
                    }
                }
                #[cfg(not(unix))]
                {
                    (-1, false)
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!("Process wait error: {}", e);
            (-1, false)
        }
        Err(_) => {
            // Timeout - kill the process
            let _ = child.kill().await;
            (-1, true)
        }
    };

    // Collect output from tasks
    if let Ok(stdout_output) = stdout_handle.await {
        if combined_output.len() + stdout_output.len() <= MAX_OUTPUT_SIZE {
            combined_output.push_str(&stdout_output);
        } else {
            let remaining = MAX_OUTPUT_SIZE.saturating_sub(combined_output.len());
            combined_output.push_str(&stdout_output[..remaining.min(stdout_output.len())]);
            combined_output.push_str("\n[OUTPUT TRUNCATED: exceeded size limit]");
        }
    }
    if let Ok(stderr_output) = stderr_handle.await {
        if combined_output.len() + stderr_output.len() <= MAX_OUTPUT_SIZE {
            if !combined_output.is_empty() && !stderr_output.is_empty() {
                combined_output.push('\n');
            }
            combined_output.push_str(&stderr_output);
        } else if combined_output.len() < MAX_OUTPUT_SIZE {
            let remaining = MAX_OUTPUT_SIZE.saturating_sub(combined_output.len());
            if !combined_output.is_empty() {
                combined_output.push('\n');
            }
            combined_output.push_str(&stderr_output[..remaining.min(stderr_output.len())]);
            combined_output.push_str("\n[OUTPUT TRUNCATED: exceeded size limit]");
        }
    }

    // Send completion signal
    let _ = output_tx.send(ToolOutputChunk {
        tool_use_id: tool_use_id.clone(),
        chunk: String::new(),
        is_complete: true,
        exit_code: Some(exit_code),
    });

    ToolResult {
        output: json!({
            "output": combined_output,
            "exitCode": exit_code,
            "killed": killed
        })
        .to_string(),
        is_error: exit_code != 0,
    }
}

/// Execute command blocking (legacy mode without streaming)
async fn execute_blocking(mut cmd: Command, timeout_duration: Duration) -> ToolResult {
    let timeout_ms = timeout_duration.as_millis();

    match timeout(timeout_duration, cmd.output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            let combined = match (stdout.is_empty(), stderr.is_empty()) {
                (true, true) => String::new(),
                (false, true) => stdout.into_owned(),
                (true, false) => stderr.into_owned(),
                (false, false) => format!("{}\n{}", stdout, stderr),
            };

            ToolResult {
                output: json!({
                    "output": combined,
                    "exitCode": exit_code,
                    "killed": false
                })
                .to_string(),
                is_error: exit_code != 0,
            }
        }
        Ok(Err(e)) => ToolResult::error(format!("Failed to execute command: {}", e)),
        Err(_) => ToolResult {
            output: json!({
                "output": format!("Command timed out after {} ms", timeout_ms),
                "exitCode": -1,
                "killed": true
            })
            .to_string(),
            is_error: true,
        },
    }
}

/// Execute command in background, return immediately with shell ID
async fn execute_background(mut cmd: Command) -> ToolResult {
    let shell_id = uuid::Uuid::new_v4().to_string();

    cmd.stdout(Stdio::null()).stderr(Stdio::null());

    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id().unwrap_or(0);
            tracing::info!(shell_id = %shell_id, pid = pid, "Started background process");

            // Detach - let the process run independently
            tokio::spawn(async move {
                let _ = child.wait_with_output().await;
            });

            ToolResult::success(
                json!({
                    "output": "Process started in background",
                    "exitCode": 0,
                    "killed": false,
                    "shellId": shell_id
                })
                .to_string(),
            )
        }
        Err(e) => ToolResult::error(format!("Failed to start background process: {}", e)),
    }
}
