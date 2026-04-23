use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::RwLock;

use super::model::{elapsed_millis_u64, ProcessEntry, ProcessId, ProcessInfo, ProcessStatus};
use super::signals::{resume_process_tree, suspend_process_tree, terminate_process_tree};

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

    fn ensure_user_map<'a>(
        map: &'a mut HashMap<String, HashMap<ProcessId, ProcessEntry>>,
        user_id: &str,
    ) -> &'a mut HashMap<ProcessId, ProcessEntry> {
        map.entry(user_id.to_string()).or_default()
    }

    pub async fn spawn(
        &self,
        command: String,
        working_dir: PathBuf,
        description: Option<String>,
    ) -> Result<ProcessId> {
        self.spawn_for_user(DEFAULT_USER, command, working_dir, description)
            .await
    }

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

    pub async fn kill(&self, id: &str) -> Result<()> {
        self.kill_for_user(DEFAULT_USER, id).await
    }

    pub async fn kill_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        if let Some(entry) = user_map.get_mut(id) {
            if entry.info.is_running() {
                if let Some(pid) = entry.info.pid {
                    terminate_process_tree(pid);
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

    pub async fn suspend(&self, id: &str) -> Result<()> {
        self.suspend_for_user(DEFAULT_USER, id).await
    }

    pub async fn suspend_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        if let Some(entry) = user_map.get_mut(id) {
            if matches!(entry.info.status, ProcessStatus::Running) {
                if let Some(pid) = entry.info.pid {
                    suspend_process_tree(pid)?;
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

    pub async fn resume(&self, id: &str) -> Result<()> {
        self.resume_for_user(DEFAULT_USER, id).await
    }

    pub async fn resume_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        if let Some(entry) = user_map.get_mut(id) {
            if matches!(entry.info.status, ProcessStatus::Suspended) {
                if let Some(pid) = entry.info.pid {
                    resume_process_tree(pid)?;
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

    pub async fn list(&self) -> Vec<ProcessInfo> {
        self.processes
            .read()
            .await
            .values()
            .flat_map(|user_map| user_map.values().map(|e| e.info.clone()))
            .collect()
    }

    pub async fn list_for_user(&self, user_id: &str) -> Vec<ProcessInfo> {
        self.processes
            .read()
            .await
            .get(user_id)
            .map(|user_map| user_map.values().map(|e| e.info.clone()).collect())
            .unwrap_or_default()
    }

    pub fn try_running_count(&self) -> Option<usize> {
        self.processes.try_read().ok().map(|guard| {
            guard
                .values()
                .flat_map(|user_map| user_map.values())
                .filter(|e| e.info.is_running())
                .count()
        })
    }

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

    pub fn try_list(&self) -> Option<Vec<ProcessInfo>> {
        self.processes.try_read().ok().map(|guard| {
            guard
                .values()
                .flat_map(|user_map| user_map.values().map(|e| e.info.clone()))
                .collect()
        })
    }

    pub async fn get(&self, id: &str) -> Option<ProcessInfo> {
        self.processes
            .read()
            .await
            .values()
            .find_map(|user_map| user_map.get(id).map(|e| e.info.clone()))
    }

    pub async fn get_for_user(&self, user_id: &str, id: &str) -> Option<ProcessInfo> {
        self.processes
            .read()
            .await
            .get(user_id)
            .and_then(|user_map| user_map.get(id).map(|e| e.info.clone()))
    }

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

    pub async fn update_status_for_user(&self, user_id: &str, id: &str, status: ProcessStatus) {
        let mut processes = self.processes.write().await;
        if let Some(user_map) = processes.get_mut(user_id) {
            if let Some(entry) = user_map.get_mut(id) {
                tracing::info!(id = %id, user_id = %user_id, status = ?status, "Process status updated");
                entry.info.status = status;
            }
        }
    }

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
                terminate_process_tree(pid);
                tracing::info!(id = %id, pid = pid, "Killed process on shutdown");
            }
        }
    }

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

    pub async fn unregister(&self, id: &str) {
        let mut processes = self.processes.write().await;
        for user_map in processes.values_mut() {
            if let Some(entry) = user_map.remove(id) {
                tracing::info!(id = %id, status = ?entry.info.status, "Process unregistered");
                return;
            }
        }
    }

    pub async fn unregister_for_user(&self, user_id: &str, id: &str) {
        let mut processes = self.processes.write().await;
        if let Some(user_map) = processes.get_mut(user_id) {
            if let Some(entry) = user_map.remove(id) {
                tracing::info!(id = %id, user_id = %user_id, status = ?entry.info.status, "Process unregistered");
            }
        }
    }
}
