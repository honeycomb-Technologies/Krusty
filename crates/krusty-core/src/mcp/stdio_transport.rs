//! Bounded stdio transport for local MCP child processes.
//!
//! rmcp's stock child transport uses an unlimited JSONL decoder and only
//! guarantees cleanup of the direct child. Local MCP servers are untrusted
//! processes, so Krusty owns the child, bounds each inbound JSON-RPC record,
//! and retains enough process-tree identity to clean up descendants.

use std::future::Future;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rmcp::service::{RoleClient, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::async_rw::JsonRpcMessageCodec;
use rmcp::transport::Transport;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio_util::codec::{FramedRead, FramedWrite};

/// Maximum newline-delimited JSON-RPC record accepted from a local MCP
/// server. The decoder rejects the record before retaining more than this
/// amount plus its small framing lookahead.
pub(crate) const MAX_MCP_STDIO_JSON_RPC_LINE_BYTES: usize = 8 * 1024 * 1024;

const MCP_CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(2);

type StdioReader = FramedRead<ChildStdout, JsonRpcMessageCodec<RxJsonRpcMessage<RoleClient>>>;
type StdioWriter = FramedWrite<ChildStdin, JsonRpcMessageCodec<TxJsonRpcMessage<RoleClient>>>;

/// Tracks the OS process tree independently of the direct child handle.
///
/// Tokio's `kill_on_drop` only covers the direct child. On Unix, every MCP
/// server starts as a process-group leader so the retained PID can still kill
/// descendants after that leader exits. Windows uses `taskkill /T /F` as the
/// closest available tree-aware fallback without a Job Object handle.
struct McpChildProcessTree {
    leader_pid: Option<u32>,
    armed: bool,
}

impl McpChildProcessTree {
    fn new(leader_pid: Option<u32>) -> Self {
        Self {
            leader_pid,
            armed: leader_pid.is_some(),
        }
    }

    fn force_kill_sync(&self) {
        if !self.armed {
            return;
        }
        let Some(pid) = self.leader_pid else {
            return;
        };

        #[cfg(unix)]
        if let Err(error) =
            crate::process::signals::signal_process_group(pid, libc::SIGKILL, "SIGKILL")
        {
            tracing::debug!(pid, %error, "Failed to kill MCP child process group");
        }

        #[cfg(windows)]
        {
            // Drop cannot await. Launch a detached tree-aware cleanup while
            // the direct `Child` handle below remains a second safety net.
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }

        #[cfg(not(any(unix, windows)))]
        let _ = pid;
    }

    #[cfg(windows)]
    async fn force_kill_windows(&self) -> bool {
        let Some(pid) = self.leader_pid else {
            return true;
        };
        matches!(
            tokio::time::timeout(
                MCP_CHILD_REAP_TIMEOUT,
                Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status(),
            )
            .await,
            Ok(Ok(status)) if status.success()
        )
    }

    #[cfg(unix)]
    fn disarm_if_gone(&mut self) {
        let Some(pid) = self.leader_pid else {
            self.armed = false;
            return;
        };
        if matches!(
            crate::process::signals::process_group_exists(pid),
            Ok(false)
        ) {
            self.armed = false;
        }
    }

    #[cfg(not(unix))]
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for McpChildProcessTree {
    fn drop(&mut self) {
        self.force_kill_sync();
    }
}

/// An rmcp client transport with a bounded reader and owned child lifecycle.
pub(crate) struct BoundedStdioTransport {
    read: StdioReader,
    write: Arc<Mutex<Option<StdioWriter>>>,
    child: Option<Child>,
    process_tree: McpChildProcessTree,
}

impl BoundedStdioTransport {
    pub(crate) fn spawn(mut command: Command) -> std::io::Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        #[cfg(unix)]
        command.process_group(0);

        let mut child = command.spawn()?;
        let process_tree = McpChildProcessTree::new(child.id());
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("MCP child stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("MCP child stdout unavailable"))?;

        let read = FramedRead::new(
            stdout,
            JsonRpcMessageCodec::new_with_max_length(MAX_MCP_STDIO_JSON_RPC_LINE_BYTES),
        );
        let write = Arc::new(Mutex::new(Some(FramedWrite::new(
            stdin,
            JsonRpcMessageCodec::default(),
        ))));

        Ok(Self {
            read,
            write,
            child: Some(child),
            process_tree,
        })
    }

    async fn terminate_child(&mut self) {
        #[cfg(unix)]
        self.process_tree.force_kill_sync();

        #[cfg(windows)]
        let tree_killed = self.process_tree.force_kill_windows().await;

        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(MCP_CHILD_REAP_TIMEOUT, child.wait()).await;
        }

        #[cfg(unix)]
        self.process_tree.disarm_if_gone();

        #[cfg(windows)]
        if tree_killed {
            self.process_tree.disarm();
        }

        #[cfg(not(any(unix, windows)))]
        self.process_tree.disarm();
    }
}

impl Transport<RoleClient> for BoundedStdioTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let write = Arc::clone(&self.write);
        async move {
            let mut write = write.lock().await;
            let Some(writer) = write.as_mut() else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "MCP stdio transport is closed",
                ));
            };
            writer.send(item).await.map_err(Into::into)
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        match self.read.next().await {
            Some(Ok(message)) => Some(message),
            Some(Err(error)) => {
                tracing::warn!(
                    max_line_bytes = MAX_MCP_STDIO_JSON_RPC_LINE_BYTES,
                    %error,
                    "Rejected MCP stdio JSON-RPC record and terminating its process tree"
                );
                self.terminate_child().await;
                None
            }
            None => {
                // EOF can follow a server crash while descendants remain.
                // Preserve the same tree-cleanup invariant as protocol errors.
                self.terminate_child().await;
                None
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let mut write = self.write.lock().await;
        drop(write.take());
        drop(write);
        self.terminate_child().await;
        Ok(())
    }
}

impl Drop for BoundedStdioTransport {
    fn drop(&mut self) {
        // This path covers startup timeout/cancellation, where rmcp may drop
        // the transport without ever awaiting `close`.
        self.process_tree.force_kill_sync();
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            // Reap when Drop runs inside the normal Tokio runtime. If no
            // runtime is active, `kill_on_drop` still protects the direct
            // child and the synchronous process-tree kill already ran.
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let _reaper = runtime.spawn(async move {
                    let _ = tokio::time::timeout(MCP_CHILD_REAP_TIMEOUT, child.wait()).await;
                });
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn oversized_record_is_rejected_and_entire_process_group_is_terminated() {
        let Ok(python) = which::which("python3") else {
            return;
        };
        let temp = tempfile::tempdir().expect("create test directory");
        let pid_file = temp.path().join("pids");
        let script = format!(
            r#"
import os
import subprocess
import sys
import time

descendant = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    handle.write(f"{{os.getpid()}} {{descendant.pid}}")
    handle.flush()
    os.fsync(handle.fileno())
sys.stdout.buffer.write(b"x" * {} + b"\n")
sys.stdout.buffer.flush()
time.sleep(30)
"#,
            MAX_MCP_STDIO_JSON_RPC_LINE_BYTES + 1
        );
        let mut command = Command::new(python);
        command.arg("-c").arg(script).arg(&pid_file);
        let mut transport = BoundedStdioTransport::spawn(command).expect("spawn MCP test child");

        let started = tokio::time::Instant::now();
        assert!(transport.receive().await.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "oversized input should terminate without waiting for the child sleep"
        );

        let pids = tokio::fs::read_to_string(&pid_file)
            .await
            .expect("read child process IDs");
        let leader_pid = pids
            .split_whitespace()
            .next()
            .expect("leader PID")
            .parse::<u32>()
            .expect("numeric leader PID");
        for _ in 0..100 {
            if matches!(
                crate::process::signals::process_group_exists(leader_pid),
                Ok(false)
            ) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !crate::process::signals::process_group_exists(leader_pid)
                .expect("inspect MCP child process group"),
            "oversized input left an MCP descendant running"
        );
    }
}
