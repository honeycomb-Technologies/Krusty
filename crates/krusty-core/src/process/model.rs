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
    pub(super) output: Arc<Mutex<ProcessOutputBuffer>>,
    /// Keep handle alive to prevent task cancellation
    pub(super) _handle: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug, Default)]
pub(super) struct ProcessOutputBuffer {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}
