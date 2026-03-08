//! Bash tool - Execute shell commands with real-time output streaming

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, timeout};

use crate::tools::registry::{Tool, ToolOutputChunk};
use crate::tools::truncation;
use crate::tools::{parse_params, ToolContext, ToolResult};

const MAX_OUTPUT_LINES: usize = 2000;
const MAX_OUTPUT_BYTES: usize = 50_000; // 50KB

// Bounded raw capture for foreground execution. Final model output is additionally
// truncated by MAX_OUTPUT_LINES/MAX_OUTPUT_BYTES after ANSI stripping.
const RAW_CAPTURE_MAX_LINES: usize = 8_000;
const RAW_CAPTURE_MAX_BYTES: usize = 2_000_000; // 2MB
const READER_JOIN_TIMEOUT_MS: u64 = 2_000;
const TIMEOUT_KILL_GRACE_MS: u64 = 800;

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

#[derive(Clone)]
struct StreamContext {
    output_tx: mpsc::UnboundedSender<ToolOutputChunk>,
    tool_use_id: String,
}

struct BoundedOutputBuffer {
    lines: VecDeque<String>,
    total_bytes: usize,
    dropped_lines: usize,
    max_lines: usize,
    max_bytes: usize,
}

impl BoundedOutputBuffer {
    fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            total_bytes: 0,
            dropped_lines: 0,
            max_lines,
            max_bytes,
        }
    }

    fn push_line(&mut self, line: &str) {
        let mut kept = line.to_string();
        if kept.len() > self.max_bytes {
            kept = tail_by_bytes(&kept, self.max_bytes);
        }

        self.total_bytes = self.total_bytes.saturating_add(kept.len());
        self.lines.push_back(kept);

        while self.lines.len() > self.max_lines || self.total_bytes > self.max_bytes {
            if let Some(removed) = self.lines.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(removed.len());
                self.dropped_lines = self.dropped_lines.saturating_add(1);
            } else {
                break;
            }
        }
    }

    fn into_text(self) -> String {
        let mut out = self.lines.into_iter().collect::<Vec<_>>().join("\n");
        if self.dropped_lines > 0 {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!(
                "[... omitted {} earlier line(s) due to buffer limits ...]",
                self.dropped_lines
            ));
        }
        out
    }
}

/// Keep the tail of a string within `max_bytes`, preserving UTF-8 boundaries.
fn tail_by_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// Strip ANSI escape sequences from text
fn strip_ansi(text: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07|\x1b\[[\?0-9;]*[a-zA-Z]")
        .expect("valid regex");
    re.replace_all(text, "").into_owned()
}

/// Detect a trailing shell background operator (`&`) that is not quoted/escaped,
/// and return the command without it.
fn strip_shell_background_suffix(command: &str) -> Option<String> {
    let trimmed = command.trim_end();
    let (amp_idx, last_char) = trimmed.char_indices().last()?;
    if last_char != '&' {
        return None;
    }

    let prefix = trimmed[..amp_idx].trim_end();
    if prefix.is_empty() {
        return None;
    }

    // Reject `&&` and `|&` style endings.
    if matches!(prefix.chars().last(), Some('&' | '|')) {
        return None;
    }

    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (idx, ch) in trimmed.char_indices() {
        if idx == amp_idx {
            if in_single || in_double || escaped {
                return None;
            }
            break;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {}
        }
    }

    Some(prefix.to_string())
}

fn build_shell_command(command: &str, ctx: &ToolContext) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };

    cmd.env("NO_COLOR", "1");

    if let Some(ref identity) = ctx.git_identity {
        for (key, val) in identity.env_vars() {
            cmd.env(key, val);
        }
    }

    cmd.current_dir(&ctx.working_dir);
    cmd
}

fn configure_foreground_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
}

async fn collect_pipe_output<R>(
    pipe: Option<R>,
    stream: Option<StreamContext>,
    buffer: Arc<Mutex<BoundedOutputBuffer>>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let Some(pipe) = pipe else {
        return;
    };

    let mut reader = BufReader::new(pipe).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        if let Some(stream) = &stream {
            let _ = stream.output_tx.send(ToolOutputChunk {
                tool_use_id: stream.tool_use_id.clone(),
                chunk: format!("{}\n", line),
                is_complete: false,
                exit_code: None,
            });
        }

        buffer.lock().await.push_line(&line);
    }
}

async fn join_reader_with_timeout(mut handle: tokio::task::JoinHandle<()>) {
    if timeout(Duration::from_millis(READER_JOIN_TIMEOUT_MS), &mut handle)
        .await
        .is_err()
    {
        handle.abort();
    }

    let _ = handle.await;
}

#[cfg(unix)]
async fn terminate_unix_process_tree(pid: u32) {
    let pgid = format!("-{}", pid);

    let group_term_ok = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(&pgid)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !group_term_ok {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
    }

    sleep(Duration::from_millis(200)).await;

    let still_running = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if still_running {
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(&pgid)
            .status();
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status();
    }
}

#[cfg(windows)]
async fn terminate_windows_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output();
}

async fn terminate_process_tree(child: &mut Child) {
    let Some(pid) = child.id() else {
        let _ = child.kill().await;
        return;
    };

    #[cfg(unix)]
    terminate_unix_process_tree(pid).await;

    #[cfg(windows)]
    terminate_windows_process_tree(pid).await;

    if timeout(Duration::from_millis(TIMEOUT_KILL_GRACE_MS), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

async fn execute_foreground(
    mut cmd: Command,
    timeout_duration: Duration,
    stream: Option<StreamContext>,
) -> ToolResult {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("Failed to spawn command: {}", e)),
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let buffer = Arc::new(Mutex::new(BoundedOutputBuffer::new(
        RAW_CAPTURE_MAX_LINES,
        RAW_CAPTURE_MAX_BYTES,
    )));

    let stdout_handle = tokio::spawn(collect_pipe_output(
        stdout,
        stream.clone(),
        Arc::clone(&buffer),
    ));
    let stderr_handle = tokio::spawn(collect_pipe_output(
        stderr,
        stream.clone(),
        Arc::clone(&buffer),
    ));

    let wait_result = timeout(timeout_duration, child.wait()).await;
    let (exit_code, killed, timed_out) = match wait_result {
        Ok(Ok(status)) => {
            if let Some(code) = status.code() {
                (code, false, false)
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    match status.signal() {
                        Some(2) | Some(15) => (0, false, false),
                        Some(sig) => {
                            tracing::debug!("Process killed by signal {}", sig);
                            (128 + sig, false, false)
                        }
                        None => (-1, false, false),
                    }
                }
                #[cfg(not(unix))]
                {
                    (-1, false, false)
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!("Process wait error: {}", e);
            (-1, false, false)
        }
        Err(_) => {
            terminate_process_tree(&mut child).await;
            (-1, true, true)
        }
    };

    join_reader_with_timeout(stdout_handle).await;
    join_reader_with_timeout(stderr_handle).await;

    let combined_output = {
        let mut guard = buffer.lock().await;
        let captured = std::mem::replace(
            &mut *guard,
            BoundedOutputBuffer::new(RAW_CAPTURE_MAX_LINES, RAW_CAPTURE_MAX_BYTES),
        );
        captured.into_text()
    };

    if let Some(stream) = &stream {
        let _ = stream.output_tx.send(ToolOutputChunk {
            tool_use_id: stream.tool_use_id.clone(),
            chunk: String::new(),
            is_complete: true,
            exit_code: Some(exit_code),
        });
    }

    let processed = process_output(combined_output);
    let metadata = Some(json!({
        "exit_code": exit_code,
        "killed": killed,
    }));

    if timed_out {
        ToolResult::error_with_details(
            "timeout",
            format!(
                "Command timed out after {} ms",
                timeout_duration.as_millis()
            ),
            Some(json!({ "output": processed })),
            metadata,
        )
    } else if exit_code == 0 {
        ToolResult::success_data_with(json!({ "output": processed }), Vec::new(), None, metadata)
    } else {
        ToolResult::error_with_details(
            "command_failed",
            format!("Command exited with code {}", exit_code),
            Some(json!({ "output": processed })),
            metadata,
        )
    }
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

        // Apply git identity for commit attribution
        let effective_command = if let Some(ref identity) = ctx.git_identity {
            identity.apply_to_command(&params.command)
        } else {
            params.command.clone()
        };

        let inferred_background_command = strip_shell_background_suffix(&effective_command);
        let inferred_from_shell_suffix = inferred_background_command.is_some();

        // Handle background execution (explicit param OR safe shell suffix detection)
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

        // Foreground execution with bounded output capture.
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

/// Execute command in background, return immediately with shell ID
async fn execute_background(mut cmd: Command, warnings: Vec<String>) -> ToolResult {
    let shell_id = uuid::Uuid::new_v4().to_string();

    cmd.stdout(Stdio::null()).stderr(Stdio::null());

    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id().unwrap_or(0);
            tracing::info!(shell_id = %shell_id, pid = pid, "Started background process");

            tokio::spawn(async move {
                let _ = child.wait_with_output().await;
            });

            ToolResult::success_data_with(
                json!({
                    "message": "Process started in background",
                    "shell_id": shell_id,
                    "status": "running"
                }),
                warnings,
                None,
                Some(json!({
                    "exit_code": 0,
                    "killed": false
                })),
            )
        }
        Err(e) => ToolResult::error(format!("Failed to start background process: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_shell_background_suffix_accepts_simple_suffix() {
        let parsed = strip_shell_background_suffix("npm run dev &");
        assert_eq!(parsed.as_deref(), Some("npm run dev"));
    }

    #[test]
    fn strip_shell_background_suffix_rejects_quoted_ampersand() {
        let parsed = strip_shell_background_suffix("echo '&'");
        assert!(parsed.is_none());
    }

    #[test]
    fn strip_shell_background_suffix_rejects_escaped_ampersand() {
        let parsed = strip_shell_background_suffix(r"echo foo \&");
        assert!(parsed.is_none());
    }

    #[test]
    fn strip_shell_background_suffix_rejects_double_ampersand() {
        let parsed = strip_shell_background_suffix("echo hi &&");
        assert!(parsed.is_none());
    }

    #[test]
    fn bounded_output_buffer_keeps_recent_lines() {
        let mut buffer = BoundedOutputBuffer::new(3, 1024);
        buffer.push_line("l1");
        buffer.push_line("l2");
        buffer.push_line("l3");
        buffer.push_line("l4");

        let text = buffer.into_text();
        assert!(!text.contains("l1"));
        assert!(text.contains("l2"));
        assert!(text.contains("l3"));
        assert!(text.contains("l4"));
    }

    #[test]
    fn bounded_output_buffer_clips_to_max_bytes() {
        let mut buffer = BoundedOutputBuffer::new(100, 10);
        buffer.push_line("12345");
        buffer.push_line("67890");
        buffer.push_line("abcdef");

        let text = buffer.into_text();
        assert!(text.len() <= 200); // Includes optional omission notice.
        assert!(text.contains("abcdef") || text.contains("bcdef"));
    }
}
