//! Background process management
//!
//! Tracks spawned background processes for visibility and control

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::RwLock;

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

    pub fn duration(&self) -> std::time::Duration {
        // For finished processes, use the captured duration; for running/suspended, use elapsed
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

fn elapsed_millis_u64(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct ProcessEntry {
    info: ProcessInfo,
    /// Keep handle alive to prevent task cancellation
    _handle: Option<tokio::task::JoinHandle<()>>,
}

/// Default user ID for single-tenant mode
const DEFAULT_USER: &str = "default";

/// Registry for tracking background processes, scoped by user for multi-tenant isolation
#[derive(Clone)]
pub struct ProcessRegistry {
    /// Outer key: user_id, Inner key: process_id
    processes: Arc<RwLock<HashMap<String, HashMap<ProcessId, ProcessEntry>>>>,
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create user's process map
    fn ensure_user_map<'a>(
        map: &'a mut HashMap<String, HashMap<ProcessId, ProcessEntry>>,
        user_id: &str,
    ) -> &'a mut HashMap<ProcessId, ProcessEntry> {
        map.entry(user_id.to_string()).or_default()
    }

    /// Spawn a new background process and track it (single-tenant compatibility)
    pub async fn spawn(
        &self,
        command: String,
        working_dir: PathBuf,
        description: Option<String>,
    ) -> Result<ProcessId> {
        self.spawn_for_user(DEFAULT_USER, command, working_dir, description)
            .await
    }

    /// Spawn a new background process for a specific user (multi-tenant)
    pub async fn spawn_for_user(
        &self,
        user_id: &str,
        command: String,
        working_dir: PathBuf,
        description: Option<String>,
    ) -> Result<ProcessId> {
        let id = uuid::Uuid::new_v4().to_string();

        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&command);
            // Create new process group so we can kill all children
            #[cfg(unix)]
            {
                c.process_group(0);
            }
            c
        };

        cmd.current_dir(&working_dir);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn()?;
        let pid = child.id();

        let info = ProcessInfo {
            id: id.clone(),
            command: command.clone(),
            description,
            pid,
            started_at: Instant::now(),
            status: ProcessStatus::Running,
            _working_dir: working_dir,
        };

        tracing::info!(id = %id, user_id = %user_id, pid = ?pid, command = %command, "Process spawned");

        // Spawn task to monitor process completion
        let registry = self.clone();
        let process_id = id.clone();
        let owner_id = user_id.to_string();
        let start_time = info.started_at;
        let handle = tokio::spawn(async move {
            let result = child.wait().await;
            let duration_ms = elapsed_millis_u64(start_time);

            let status = match result {
                Ok(exit_status) => {
                    let code = exit_status.code().unwrap_or(-1);
                    if exit_status.success() {
                        ProcessStatus::Completed {
                            exit_code: code,
                            duration_ms,
                        }
                    } else {
                        ProcessStatus::Failed {
                            error: format!("Exit code: {}", code),
                            duration_ms,
                        }
                    }
                }
                Err(e) => ProcessStatus::Failed {
                    error: e.to_string(),
                    duration_ms,
                },
            };

            registry
                .update_status_for_user(&owner_id, &process_id, status)
                .await;
        });

        let entry = ProcessEntry {
            info,
            _handle: Some(handle),
        };

        let mut processes = self.processes.write().await;
        Self::ensure_user_map(&mut processes, user_id).insert(id.clone(), entry);

        Ok(id)
    }

    /// Kill a process by ID (single-tenant compatibility)
    pub async fn kill(&self, id: &str) -> Result<()> {
        self.kill_for_user(DEFAULT_USER, id).await
    }

    /// Kill a process for a specific user (multi-tenant)
    pub async fn kill_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        if let Some(entry) = user_map.get_mut(id) {
            if entry.info.is_running() {
                if let Some(pid) = entry.info.pid {
                    #[cfg(unix)]
                    {
                        // Kill entire process group (negative PID)
                        // The process was started with process_group(0) making it a group leader
                        let pgid = format!("-{}", pid);
                        let result = std::process::Command::new("kill")
                            .arg("-TERM")
                            .arg(&pgid)
                            .output();

                        if result.is_err() {
                            // Fallback: kill just the process
                            let _ = std::process::Command::new("kill")
                                .arg("-TERM")
                                .arg(pid.to_string())
                                .output();
                        }
                    }
                    #[cfg(windows)]
                    {
                        // /T kills the process tree
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/T", "/F"])
                            .output();
                    }
                }

                let duration_ms = elapsed_millis_u64(entry.info.started_at);
                entry.info.status = ProcessStatus::Killed { duration_ms };

                tracing::info!(id = %id, user_id = %user_id, "Process killed");
                Ok(())
            } else {
                anyhow::bail!("Process not running")
            }
        } else {
            anyhow::bail!("Process not found")
        }
    }

    /// Suspend a process by ID (single-tenant compatibility)
    pub async fn suspend(&self, id: &str) -> Result<()> {
        self.suspend_for_user(DEFAULT_USER, id).await
    }

    /// Suspend a process for a specific user (multi-tenant)
    pub async fn suspend_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        if let Some(entry) = user_map.get_mut(id) {
            if matches!(entry.info.status, ProcessStatus::Running) {
                if let Some(pid) = entry.info.pid {
                    #[cfg(unix)]
                    {
                        // Suspend entire process group (negative PID)
                        let pgid = format!("-{}", pid);
                        let result = std::process::Command::new("kill")
                            .arg("-STOP")
                            .arg(&pgid)
                            .output();

                        if result.is_err() {
                            // Fallback: suspend just the process
                            let _ = std::process::Command::new("kill")
                                .arg("-STOP")
                                .arg(pid.to_string())
                                .output();
                        }
                    }
                    #[cfg(windows)]
                    {
                        anyhow::bail!("Suspend not supported on Windows");
                    }
                }

                entry.info.status = ProcessStatus::Suspended;

                tracing::info!(id = %id, user_id = %user_id, "Process suspended");
                Ok(())
            } else {
                anyhow::bail!("Process not running")
            }
        } else {
            anyhow::bail!("Process not found")
        }
    }

    /// Resume a suspended process (single-tenant compatibility)
    pub async fn resume(&self, id: &str) -> Result<()> {
        self.resume_for_user(DEFAULT_USER, id).await
    }

    /// Resume a suspended process for a specific user (multi-tenant)
    pub async fn resume_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        if let Some(entry) = user_map.get_mut(id) {
            if matches!(entry.info.status, ProcessStatus::Suspended) {
                if let Some(pid) = entry.info.pid {
                    #[cfg(unix)]
                    {
                        // Resume entire process group (negative PID)
                        let pgid = format!("-{}", pid);
                        let result = std::process::Command::new("kill")
                            .arg("-CONT")
                            .arg(&pgid)
                            .output();

                        if result.is_err() {
                            // Fallback: resume just the process
                            let _ = std::process::Command::new("kill")
                                .arg("-CONT")
                                .arg(pid.to_string())
                                .output();
                        }
                    }
                    #[cfg(windows)]
                    {
                        anyhow::bail!("Resume not supported on Windows");
                    }
                }

                entry.info.status = ProcessStatus::Running;

                tracing::info!(id = %id, user_id = %user_id, "Process resumed");
                Ok(())
            } else {
                anyhow::bail!("Process not suspended")
            }
        } else {
            anyhow::bail!("Process not found")
        }
    }

    /// List all processes for all users (single-tenant compatibility)
    pub async fn list(&self) -> Vec<ProcessInfo> {
        self.processes
            .read()
            .await
            .values()
            .flat_map(|user_map| user_map.values().map(|e| e.info.clone()))
            .collect()
    }

    /// List processes for a specific user (multi-tenant)
    pub async fn list_for_user(&self, user_id: &str) -> Vec<ProcessInfo> {
        self.processes
            .read()
            .await
            .get(user_id)
            .map(|user_map| user_map.values().map(|e| e.info.clone()).collect())
            .unwrap_or_default()
    }

    /// Count running processes across all users (non-blocking)
    pub fn try_running_count(&self) -> Option<usize> {
        self.processes.try_read().ok().map(|guard| {
            guard
                .values()
                .flat_map(|user_map| user_map.values())
                .filter(|e| e.info.is_running())
                .count()
        })
    }

    /// Get elapsed time of oldest running process across all users (non-blocking)
    pub fn try_oldest_running_elapsed(&self) -> Option<std::time::Duration> {
        self.processes.try_read().ok().and_then(|guard| {
            guard
                .values()
                .flat_map(|user_map| user_map.values())
                .filter(|e| e.info.is_running())
                .map(|e| e.info.started_at.elapsed())
                .max()
        })
    }

    /// List all processes across all users (non-blocking)
    pub fn try_list(&self) -> Option<Vec<ProcessInfo>> {
        self.processes.try_read().ok().map(|guard| {
            guard
                .values()
                .flat_map(|user_map| user_map.values().map(|e| e.info.clone()))
                .collect()
        })
    }

    /// Get a specific process (single-tenant compatibility, searches all users)
    pub async fn get(&self, id: &str) -> Option<ProcessInfo> {
        self.processes
            .read()
            .await
            .values()
            .find_map(|user_map| user_map.get(id).map(|e| e.info.clone()))
    }

    /// Get a specific process for a user (multi-tenant)
    pub async fn get_for_user(&self, user_id: &str, id: &str) -> Option<ProcessInfo> {
        self.processes
            .read()
            .await
            .get(user_id)
            .and_then(|user_map| user_map.get(id).map(|e| e.info.clone()))
    }

    /// Update process status (single-tenant compatibility, searches all users)
    pub async fn update_status(&self, id: &str, status: ProcessStatus) {
        let mut processes = self.processes.write().await;
        for user_map in processes.values_mut() {
            if let Some(entry) = user_map.get_mut(id) {
                tracing::info!(id = %id, status = ?status, "Process status updated");
                entry.info.status = status;
                return;
            }
        }
    }

    /// Update process status for a specific user (multi-tenant)
    pub async fn update_status_for_user(&self, user_id: &str, id: &str, status: ProcessStatus) {
        let mut processes = self.processes.write().await;
        if let Some(user_map) = processes.get_mut(user_id) {
            if let Some(entry) = user_map.get_mut(id) {
                tracing::info!(id = %id, user_id = %user_id, status = ?status, "Process status updated");
                entry.info.status = status;
            }
        }
    }

    /// Kill all running processes across all users (called on app shutdown)
    pub async fn kill_all(&self) {
        let processes = self.processes.read().await;
        let running: Vec<_> = processes
            .values()
            .flat_map(|user_map| user_map.iter())
            .filter(|(_, e)| e.info.is_running())
            .map(|(id, e)| (id.clone(), e.info.pid))
            .collect();
        drop(processes);

        for (id, pid) in running {
            if let Some(pid) = pid {
                #[cfg(unix)]
                {
                    // Kill entire process group
                    let pgid = format!("-{}", pid);
                    let result = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(&pgid)
                        .output();

                    if result.is_err() {
                        let _ = std::process::Command::new("kill")
                            .arg("-TERM")
                            .arg(pid.to_string())
                            .output();
                    }
                }
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .output();
                }
                tracing::info!(id = %id, pid = pid, "Killed process on shutdown");
            }
        }
    }

    /// Register an external process (single-tenant compatibility)
    pub async fn register_external(
        &self,
        id: ProcessId,
        command: String,
        description: Option<String>,
        pid: Option<u32>,
        working_dir: PathBuf,
    ) {
        self.register_external_for_user(DEFAULT_USER, id, command, description, pid, working_dir)
            .await;
    }

    /// Register an external process for a specific user (multi-tenant)
    pub async fn register_external_for_user(
        &self,
        user_id: &str,
        id: ProcessId,
        command: String,
        description: Option<String>,
        pid: Option<u32>,
        working_dir: PathBuf,
    ) {
        let info = ProcessInfo {
            id: id.clone(),
            command,
            description,
            pid,
            started_at: Instant::now(),
            status: ProcessStatus::Running,
            _working_dir: working_dir,
        };
        let entry = ProcessEntry {
            info,
            _handle: None,
        };
        let mut processes = self.processes.write().await;
        Self::ensure_user_map(&mut processes, user_id).insert(id.clone(), entry);
        tracing::info!(id = %id, user_id = %user_id, pid = ?pid, "External process registered");
    }

    /// Unregister a process (single-tenant compatibility, searches all users)
    pub async fn unregister(&self, id: &str) {
        let mut processes = self.processes.write().await;
        for user_map in processes.values_mut() {
            if let Some(entry) = user_map.remove(id) {
                tracing::info!(id = %id, status = ?entry.info.status, "Process unregistered");
                return;
            }
        }
    }

    /// Unregister a process for a specific user (multi-tenant)
    pub async fn unregister_for_user(&self, user_id: &str, id: &str) {
        let mut processes = self.processes.write().await;
        if let Some(user_map) = processes.get_mut(user_id) {
            if let Some(entry) = user_map.remove(id) {
                tracing::info!(id = %id, user_id = %user_id, status = ?entry.info.status, "Process unregistered");
            }
        }
    }
}
