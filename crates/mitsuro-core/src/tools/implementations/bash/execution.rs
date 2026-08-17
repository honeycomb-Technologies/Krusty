use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, timeout};

#[cfg(unix)]
use crate::process::signals::{
    descendant_processes, process_group_exists, signal_process, signal_process_group,
};
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
    spool: Option<Arc<Mutex<File>>>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let Some(pipe) = pipe else {
        return;
    };

    let mut reader = BufReader::new(pipe).lines();
    // Avoid one filesystem task and mutex handoff per output line. Besides
    // being expensive for compiler/test output, that could leave the reader
    // draining after the child had exited long enough for the bounded join to
    // abort it, losing the final lines from both the preview and recovery log.
    let mut spool_buffer = Vec::with_capacity(64 * 1024);
    while let Ok(Some(line)) = reader.next_line().await {
        if spool.is_some() {
            spool_buffer.extend_from_slice(line.as_bytes());
            spool_buffer.push(b'\n');
            if spool_buffer.len() >= 64 * 1024 {
                if let Some(spool) = &spool {
                    let mut file = spool.lock().await;
                    let _ = file.write_all(&spool_buffer).await;
                }
                spool_buffer.clear();
            }
        }

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

    if !spool_buffer.is_empty() {
        if let Some(spool) = &spool {
            let mut file = spool.lock().await;
            let _ = file.write_all(&spool_buffer).await;
        }
    }
}

const TOOL_OUTPUT_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

async fn cleanup_old_outputs(directory: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(TOOL_OUTPUT_RETENTION)
        .unwrap_or(std::time::UNIX_EPOCH);
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("tool_") && name.ends_with(".log"))
        {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

async fn create_output_spool(path: &Path) -> Option<Arc<Mutex<File>>> {
    let directory = path.parent()?;
    if let Err(error) = tokio::fs::create_dir_all(directory).await {
        tracing::warn!(%error, path = %directory.display(), "Could not create tool-output store");
        return None;
    }
    cleanup_old_outputs(directory).await;

    let file = match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await
    {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "Could not create tool-output spool");
            return None;
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
    }

    Some(Arc::new(Mutex::new(file)))
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
const PROCESS_GROUP_TERM_GRACE_MS: u64 = 200;

/// Last-resort cleanup for a foreground command's process group.
///
/// The agent executor cancels an in-flight tool by dropping its future. Tokio's
/// `kill_on_drop` only targets the direct child, so descendants would otherwise
/// survive. This guard deliberately uses a synchronous group-wide SIGKILL on
/// drop; the normal timeout/completion path below still gets a graceful SIGTERM
/// window and reaps the direct child asynchronously.
#[cfg(unix)]
struct ProcessGroupDropGuard {
    leader_pid: u32,
    armed: bool,
}

#[cfg(unix)]
impl ProcessGroupDropGuard {
    fn new(leader_pid: u32) -> Self {
        Self {
            leader_pid,
            armed: true,
        }
    }

    fn leader_pid(&self) -> u32 {
        self.leader_pid
    }

    fn disarm_if_gone(&mut self) {
        match process_group_exists(self.leader_pid) {
            Ok(false) => self.armed = false,
            Ok(true) => {}
            Err(error) => {
                tracing::debug!(
                    pid = self.leader_pid,
                    %error,
                    "Could not verify foreground process-group cleanup"
                );
            }
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupDropGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        match process_group_exists(self.leader_pid) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                tracing::debug!(
                    pid = self.leader_pid,
                    %error,
                    "Could not inspect foreground process group during drop cleanup"
                );
            }
        }

        let descendants = descendant_processes(self.leader_pid);

        if let Err(error) = signal_process_group(self.leader_pid, libc::SIGKILL, "SIGKILL") {
            // Exiting between the liveness probe and signal is harmless. Keep
            // this at debug level because Drop must remain best-effort.
            tracing::debug!(
                pid = self.leader_pid,
                %error,
                "Could not kill foreground process group during drop cleanup"
            );
        }
        for pid in descendants {
            let _ = signal_process(pid, libc::SIGKILL, "SIGKILL");
        }
    }
}

#[cfg(unix)]
async fn terminate_unix_process_group(pid: u32) {
    // Capture the full tree before signaling the outer group. Bubblewrap and
    // similar wrappers may create a nested session; once the wrapper exits,
    // those descendants are reparented and can no longer be discovered from
    // the original leader.
    let descendants = descendant_processes(pid);
    match process_group_exists(pid) {
        Ok(false) if descendants.is_empty() => return,
        Ok(true) => {}
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(pid, %error, "Could not inspect foreground process group");
        }
    }

    if let Err(error) = signal_process_group(pid, libc::SIGTERM, "SIGTERM") {
        tracing::debug!(pid, %error, "Could not gracefully terminate process group");
    }
    for descendant in &descendants {
        let _ = signal_process(*descendant, libc::SIGTERM, "SIGTERM");
    }

    sleep(Duration::from_millis(PROCESS_GROUP_TERM_GRACE_MS)).await;

    match process_group_exists(pid) {
        Ok(false) => {}
        Ok(true) => {
            if let Err(error) = signal_process_group(pid, libc::SIGKILL, "SIGKILL") {
                tracing::warn!(pid, %error, "Could not force-kill foreground process group");
            }
        }
        Err(error) => {
            tracing::warn!(pid, %error, "Could not verify foreground process-group termination");
            let _ = signal_process_group(pid, libc::SIGKILL, "SIGKILL");
        }
    }
    for descendant in descendants {
        let _ = signal_process(descendant, libc::SIGKILL, "SIGKILL");
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
    terminate_unix_process_group(pid).await;

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
    output_spool_path: PathBuf,
) -> ToolResult {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("Failed to spawn command: {}", e)),
    };

    #[cfg(unix)]
    let mut process_group_guard = child.id().map(ProcessGroupDropGuard::new);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let buffer = Arc::new(Mutex::new(BoundedOutputBuffer::new(
        RAW_CAPTURE_MAX_LINES,
        RAW_CAPTURE_MAX_BYTES,
    )));
    let spool = create_output_spool(&output_spool_path).await;

    let stdout_handle = tokio::spawn(collect_pipe_output(
        stdout,
        stream.clone(),
        Arc::clone(&buffer),
        spool.clone(),
    ));
    let stderr_handle = tokio::spawn(collect_pipe_output(
        stderr,
        stream.clone(),
        Arc::clone(&buffer),
        spool.clone(),
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

    // A foreground command can exit while leaving descendants behind. Clean
    // the configured process group before joining pipe readers so inherited
    // stdout/stderr handles cannot keep detached reader tasks alive. On an
    // ordinary command the group no longer exists and this is a cheap probe.
    #[cfg(unix)]
    if let Some(guard) = process_group_guard.as_ref() {
        terminate_unix_process_group(guard.leader_pid()).await;
    }

    join_reader_with_timeout(stdout_handle).await;
    join_reader_with_timeout(stderr_handle).await;

    #[cfg(unix)]
    if let Some(guard) = process_group_guard.as_mut() {
        guard.disarm_if_gone();
    }
    if let Some(spool) = &spool {
        let mut file = spool.lock().await;
        let _ = file.flush().await;
    }

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

    let stripped_output = super::strip_ansi(&combined_output);
    let truncated = crate::tools::truncation::truncate_tail(
        &stripped_output,
        super::MAX_OUTPUT_LINES,
        super::MAX_OUTPUT_BYTES,
    )
    .was_truncated;
    let retained_path = truncated.then_some(output_spool_path.as_path());
    let processed = process_output(combined_output, retained_path);
    drop(spool);
    if !truncated {
        let _ = tokio::fs::remove_file(&output_spool_path).await;
    }
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
