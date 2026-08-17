use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

pub type ProcessId = String;

/// Information about a tracked process
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub id: ProcessId,
    pub command: String,
    pub description: Option<String>,
    pub pid: Option<u32>,
    pub started_at: Instant,
    pub status: ProcessStatus,
    /// Stored for potential future use (e.g., restart)
    pub _working_dir: PathBuf,
    /// Optional parent chat/code session that started this background job.
    /// Used to wake the session when the process reaches a terminal state.
    pub session_id: Option<String>,
    /// True after a completion wake has been emitted for a terminal status.
    /// Prevents duplicate steering on status re-writes.
    pub completion_notified: bool,
}

/// Terminal-process completion payload for session wake hooks.
#[derive(Debug, Clone)]
pub struct ProcessCompletionEvent {
    pub user_id: String,
    pub process_id: ProcessId,
    pub session_id: Option<String>,
    pub command: String,
    pub description: Option<String>,
    pub status: ProcessStatus,
    pub output_preview: Option<String>,
}

/// Status of a tracked process
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessStatus {
    Running,
    Suspended,
    Completed { exit_code: i32, duration_ms: u64 },
    Failed { error: String, duration_ms: u64 },
    Killed { duration_ms: u64 },
}

impl ProcessInfo {
    pub fn is_running(&self) -> bool {
        matches!(self.status, ProcessStatus::Running)
    }

    pub fn is_suspended(&self) -> bool {
        matches!(self.status, ProcessStatus::Suspended)
    }

    pub fn is_active(&self) -> bool {
        self.is_running() || self.is_suspended()
    }

    pub fn duration(&self) -> std::time::Duration {
        match &self.status {
            ProcessStatus::Running | ProcessStatus::Suspended => self.started_at.elapsed(),
            ProcessStatus::Completed { duration_ms, .. } => {
                std::time::Duration::from_millis(*duration_ms)
            }
            ProcessStatus::Failed { duration_ms, .. } => {
                std::time::Duration::from_millis(*duration_ms)
            }
            ProcessStatus::Killed { duration_ms } => std::time::Duration::from_millis(*duration_ms),
        }
    }

    pub fn display_status(&self) -> &'static str {
        match &self.status {
            ProcessStatus::Running => "running",
            ProcessStatus::Suspended => "suspended",
            ProcessStatus::Completed { .. } => "done",
            ProcessStatus::Failed { .. } => "failed",
            ProcessStatus::Killed { .. } => "killed",
        }
    }
}

pub(super) fn elapsed_millis_u64(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(super) struct ProcessEntry {
    pub(super) info: ProcessInfo,
    /// Hash-only environment identity used to prevent cross-policy reuse.
    pub(super) environment_fingerprint: String,
    pub(super) output: Arc<Mutex<ProcessOutputBuffer>>,
    /// Keep handle alive to prevent task cancellation
    pub(super) _handle: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug, Default)]
pub(super) struct ProcessOutputBuffer {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}
