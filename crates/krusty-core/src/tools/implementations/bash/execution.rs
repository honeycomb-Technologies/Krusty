use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, timeout};

use crate::tools::registry::ToolOutputChunk;
use crate::tools::ToolResult;

use super::{
    process_output, RAW_CAPTURE_MAX_BYTES, RAW_CAPTURE_MAX_LINES, READER_JOIN_TIMEOUT_MS,
    TIMEOUT_KILL_GRACE_MS,
};

#[derive(Clone)]
pub(super) struct StreamContext {
    pub(super) output_tx: mpsc::UnboundedSender<ToolOutputChunk>,
    pub(super) tool_use_id: String,
}

pub(super) struct BoundedOutputBuffer {
    lines: VecDeque<String>,
    total_bytes: usize,
    dropped_lines: usize,
    max_lines: usize,
    max_bytes: usize,
}

impl BoundedOutputBuffer {
    pub(super) fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            total_bytes: 0,
            dropped_lines: 0,
            max_lines,
            max_bytes,
        }
    }

    pub(super) fn push_line(&mut self, line: &str) {
        let mut kept = line.to_string();
        if kept.len() > self.max_bytes {
            let start = kept.len().saturating_sub(self.max_bytes);
            let mut boundary = start;
            while boundary < kept.len() && !kept.is_char_boundary(boundary) {
                boundary += 1;
            }
            kept = kept[boundary..].to_string();
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

    pub(super) fn into_text(self) -> String {
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

pub(super) async fn join_reader_with_timeout(mut handle: tokio::task::JoinHandle<()>) {
    match timeout(Duration::from_millis(READER_JOIN_TIMEOUT_MS), async {
        (&mut handle).await
    })
    .await
    {
        Ok(join_result) => {
            let _ = join_result;
        }
        Err(_) => {
            handle.abort();
            let _ = handle.await;
        }
    }
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

pub(super) async fn execute_foreground(
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

pub(super) async fn execute_background(mut cmd: Command, warnings: Vec<String>) -> ToolResult {
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
