use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};

use super::model::{elapsed_millis_u64, ProcessEntry, ProcessId, ProcessInfo, ProcessStatus};
use super::signals::{resume_process_tree, suspend_process_tree, terminate_process_tree};

/// Default user ID for single-tenant mode
const DEFAULT_USER: &str = "default";
const STDERR_TAIL_BYTES: usize = 8 * 1024;

fn append_stderr_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    tail.extend_from_slice(bytes);
    if tail.len() > STDERR_TAIL_BYTES {
        tail.drain(..tail.len() - STDERR_TAIL_BYTES);
    }
}

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
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let pid = child.id();
        let stderr = child.stderr.take();
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let stderr_tail_for_reader = Arc::clone(&stderr_tail);
        let stderr_handle = tokio::spawn(async move {
            let Some(mut stderr) = stderr else {
                return;
            };
            let mut chunk = [0_u8; 1024];
            loop {
                match stderr.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(read) => {
                        let mut tail = stderr_tail_for_reader.lock().await;
                        append_stderr_tail(&mut tail, &chunk[..read]);
                    }
                }
            }
        });

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

        // Insert before the monitor starts. Fast startup failures (for example,
        // a preview server binding an occupied port) must not race their status
        // update against registration and remain falsely marked as running.
        {
            let entry = ProcessEntry {
                info,
                _handle: None,
            };
            let mut processes = self.processes.write().await;
            Self::ensure_user_map(&mut processes, user_id).insert(id.clone(), entry);
        }

        let registry = self.clone();
        let process_id = id.clone();
        let owner_id = user_id.to_string();
        let start_time = Instant::now();
        let handle = tokio::spawn(async move {
            let result = child.wait().await;
            let _ = stderr_handle.await;
            let duration_ms = elapsed_millis_u64(start_time);
            let stderr = String::from_utf8_lossy(&stderr_tail.lock().await)
                .trim()
                .to_string();

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
                            error: if stderr.is_empty() {
                                format!("Exit code: {}", code)
                            } else {
                                format!("Exit code: {}: {}", code, stderr)
                            },
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
                .update_status_from_monitor_for_user(&owner_id, &process_id, status)
                .await;
        });

        let mut processes = self.processes.write().await;
        if let Some(entry) = Self::ensure_user_map(&mut processes, user_id).get_mut(&id) {
            entry._handle = Some(handle);
        }

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

        let entry = user_map
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("process {id} not found for user {user_id}"))?;
        anyhow::ensure!(
            entry.info.is_active(),
            "process {id} cannot be killed because it is {}",
            entry.info.display_status()
        );
        let pid = entry
            .info
            .pid
            .ok_or_else(|| anyhow::anyhow!("process {id} has no OS process ID to signal"))?;

        // A stopped process cannot act on SIGTERM until it receives SIGCONT.
        // Resume it first, and reflect that successful transition even if the
        // subsequent termination signal fails.
        if entry.info.is_suspended() {
            resume_process_tree(pid).map_err(|error| {
                anyhow::anyhow!(
                    "failed to resume suspended process {id} before termination: {error:#}"
                )
            })?;
            entry.info.status = ProcessStatus::Running;
        }

        terminate_process_tree(pid).map_err(|error| {
            anyhow::anyhow!("failed to terminate process {id} (OS pid {pid}): {error:#}")
        })?;

        let duration_ms = elapsed_millis_u64(entry.info.started_at);
        entry.info.status = ProcessStatus::Killed { duration_ms };

        tracing::info!(id = %id, user_id = %user_id, pid, "Process killed");
        Ok(())
    }

    pub async fn suspend(&self, id: &str) -> Result<()> {
        self.suspend_for_user(DEFAULT_USER, id).await
    }

    pub async fn suspend_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        let entry = user_map
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("process {id} not found for user {user_id}"))?;
        anyhow::ensure!(
            entry.info.is_running(),
            "process {id} cannot be suspended because it is {}",
            entry.info.display_status()
        );
        let pid = entry
            .info
            .pid
            .ok_or_else(|| anyhow::anyhow!("process {id} has no OS process ID to signal"))?;

        suspend_process_tree(pid).map_err(|error| {
            anyhow::anyhow!("failed to suspend process {id} (OS pid {pid}): {error:#}")
        })?;
        entry.info.status = ProcessStatus::Suspended;

        tracing::info!(id = %id, user_id = %user_id, pid, "Process suspended");
        Ok(())
    }

    pub async fn resume(&self, id: &str) -> Result<()> {
        self.resume_for_user(DEFAULT_USER, id).await
    }

    pub async fn resume_for_user(&self, user_id: &str, id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;

        let user_map = processes
            .get_mut(user_id)
            .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

        let entry = user_map
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("process {id} not found for user {user_id}"))?;
        anyhow::ensure!(
            entry.info.is_suspended(),
            "process {id} cannot be resumed because it is {}",
            entry.info.display_status()
        );
        let pid = entry
            .info
            .pid
            .ok_or_else(|| anyhow::anyhow!("process {id} has no OS process ID to signal"))?;

        resume_process_tree(pid).map_err(|error| {
            anyhow::anyhow!("failed to resume process {id} (OS pid {pid}): {error:#}")
        })?;
        entry.info.status = ProcessStatus::Running;

        tracing::info!(id = %id, user_id = %user_id, pid, "Process resumed");
        Ok(())
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

    async fn update_status_from_monitor_for_user(
        &self,
        user_id: &str,
        id: &str,
        status: ProcessStatus,
    ) {
        let mut processes = self.processes.write().await;
        if let Some(user_map) = processes.get_mut(user_id) {
            if let Some(entry) = user_map.get_mut(id) {
                if matches!(entry.info.status, ProcessStatus::Killed { .. }) {
                    tracing::debug!(
                        id = %id,
                        user_id = %user_id,
                        observed_status = ?status,
                        "Ignoring process monitor status after intentional kill"
                    );
                    return;
                }
                tracing::info!(id = %id, user_id = %user_id, status = ?status, "Process status updated by monitor");
                entry.info.status = status;
            }
        }
    }

    pub async fn kill_all(&self) {
        let processes = self.processes.read().await;
        let active: Vec<_> = processes
            .iter()
            .flat_map(|(user_id, user_map)| {
                user_map
                    .iter()
                    .filter(|(_, entry)| entry.info.is_active())
                    .map(|(id, _)| (user_id.clone(), id.clone()))
            })
            .collect();
        drop(processes);

        for (user_id, id) in active {
            if let Err(error) = self.kill_for_user(&user_id, &id).await {
                tracing::warn!(
                    id = %id,
                    user_id = %user_id,
                    error = %error,
                    "Failed to kill tracked process during shutdown"
                );
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

#[cfg(test)]
mod tests {
    use super::{ProcessRegistry, ProcessStatus};
    use tempfile::TempDir;

    #[cfg(unix)]
    fn heartbeat_len(path: &std::path::Path) -> u64 {
        std::fs::metadata(path).map_or(0, |metadata| metadata.len())
    }

    #[cfg(unix)]
    async fn wait_for_heartbeat(path: &std::path::Path, minimum: u64) {
        for _ in 0..100 {
            if heartbeat_len(path) >= minimum {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("heartbeat {} did not reach {minimum} bytes", path.display());
    }

    #[cfg(unix)]
    async fn wait_for_process_exit(pid: u32) {
        let pid = libc::pid_t::try_from(pid).expect("test process ID fits pid_t");
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("process {pid} remained alive after termination");
    }

    #[cfg(unix)]
    async fn spawn_heartbeat(
        registry: &ProcessRegistry,
        directory: &TempDir,
        name: &str,
    ) -> (String, u32, std::path::PathBuf) {
        let script = directory.path().join(format!("{name}.sh"));
        let heartbeat = directory.path().join(format!("{name}.heartbeat"));
        std::fs::write(
            &script,
            "#!/bin/sh\nwhile :; do\n  printf x >> \"$1\"\n  sleep 0.02\ndone\n",
        )
        .expect("write heartbeat script");

        let command = format!(
            "sh {} {}",
            script
                .file_name()
                .expect("script filename")
                .to_string_lossy(),
            heartbeat
                .file_name()
                .expect("heartbeat filename")
                .to_string_lossy()
        );
        let id = registry
            .spawn(
                command,
                directory.path().to_path_buf(),
                Some(format!("{name} lifecycle test")),
            )
            .await
            .expect("spawn heartbeat process");
        let pid = registry
            .get(&id)
            .await
            .and_then(|process| process.pid)
            .expect("spawned process has PID");
        wait_for_heartbeat(&heartbeat, 3).await;
        (id, pid, heartbeat)
    }

    #[tokio::test]
    async fn fast_background_failure_is_registered_with_stderr() {
        let registry = ProcessRegistry::new();
        let directory = TempDir::new().expect("temp dir");
        let id = registry
            .spawn(
                "echo 'bind failed: address already in use' >&2; exit 7".to_string(),
                directory.path().to_path_buf(),
                Some("failed preview".to_string()),
            )
            .await
            .expect("process should spawn");

        let mut observed = None;
        for _ in 0..20 {
            let process = registry.get(&id).await.expect("registered process");
            if !process.is_running() {
                observed = Some(process.status);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        match observed.expect("fast failure should become visible") {
            ProcessStatus::Failed { error, .. } => {
                assert!(error.contains("Exit code: 7"));
                assert!(error.contains("address already in use"));
            }
            status => panic!("expected failed status, got {status:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn suspend_resume_and_kill_follow_the_real_process_lifecycle() {
        let registry = ProcessRegistry::new();
        let directory = TempDir::new().expect("temp dir");
        let (id, pid, heartbeat) = spawn_heartbeat(&registry, &directory, "lifecycle").await;

        registry.suspend(&id).await.expect("deliver SIGSTOP");
        assert!(
            registry.get(&id).await.expect("process").is_suspended(),
            "status changes only after SIGSTOP succeeds"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let stopped_at = heartbeat_len(&heartbeat);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            heartbeat_len(&heartbeat),
            stopped_at,
            "SIGSTOP must pause the entire process group"
        );

        registry.resume(&id).await.expect("deliver SIGCONT");
        assert!(registry.get(&id).await.expect("process").is_running());
        wait_for_heartbeat(&heartbeat, stopped_at + 2).await;

        registry.kill(&id).await.expect("deliver SIGTERM");
        wait_for_process_exit(pid).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(matches!(
            registry.get(&id).await.expect("process").status,
            ProcessStatus::Killed { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_resumes_and_terminates_a_suspended_process() {
        let registry = ProcessRegistry::new();
        let directory = TempDir::new().expect("temp dir");
        let (id, pid, _) = spawn_heartbeat(&registry, &directory, "suspended-kill").await;

        registry.suspend(&id).await.expect("suspend process");
        registry
            .kill(&id)
            .await
            .expect("resume and terminate suspended process");
        wait_for_process_exit(pid).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(matches!(
            registry.get(&id).await.expect("process").status,
            ProcessStatus::Killed { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_all_includes_running_and_suspended_processes() {
        let registry = ProcessRegistry::new();
        let directory = TempDir::new().expect("temp dir");
        let (running_id, running_pid, _) =
            spawn_heartbeat(&registry, &directory, "kill-all-running").await;
        let (suspended_id, suspended_pid, _) =
            spawn_heartbeat(&registry, &directory, "kill-all-suspended").await;
        registry
            .suspend(&suspended_id)
            .await
            .expect("suspend second process");

        registry.kill_all().await;
        wait_for_process_exit(running_pid).await;
        wait_for_process_exit(suspended_pid).await;

        for id in [running_id, suspended_id] {
            assert!(matches!(
                registry.get(&id).await.expect("process").status,
                ProcessStatus::Killed { .. }
            ));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_signal_delivery_does_not_advance_registry_state() {
        let registry = ProcessRegistry::new();
        let directory = TempDir::new().expect("temp dir");
        let id = "invalid-signal-target".to_string();
        registry
            .register_external(
                id.clone(),
                "not actually running".to_string(),
                None,
                Some(u32::MAX),
                directory.path().to_path_buf(),
            )
            .await;

        let suspend_error = registry
            .suspend(&id)
            .await
            .expect_err("invalid SIGSTOP target must fail");
        assert!(suspend_error.to_string().contains("failed to suspend"));
        assert!(suspend_error.to_string().contains("does not fit"));
        assert!(registry.get(&id).await.expect("process").is_running());

        registry.update_status(&id, ProcessStatus::Suspended).await;
        let resume_error = registry
            .resume(&id)
            .await
            .expect_err("invalid SIGCONT target must fail");
        assert!(resume_error.to_string().contains("failed to resume"));
        assert!(resume_error.to_string().contains("does not fit"));
        assert!(registry.get(&id).await.expect("process").is_suspended());

        let kill_error = registry
            .kill(&id)
            .await
            .expect_err("invalid suspended target must not be marked killed");
        assert!(kill_error
            .to_string()
            .contains("failed to resume suspended"));
        assert!(kill_error.to_string().contains("does not fit"));
        assert!(registry.get(&id).await.expect("process").is_suspended());
    }
}
