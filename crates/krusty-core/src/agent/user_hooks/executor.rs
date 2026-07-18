use super::manager::UserHookManager;
use super::model::{UserHook, UserHookResult, UserHookSource, UserHookType};

const MAX_HOOK_STDERR_BYTES: usize = 16 * 1024;
const HOOK_PROCESS_REAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

struct BoundedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

#[cfg(unix)]
struct HookProcessGroupGuard {
    leader_pid: u32,
    armed: bool,
}

#[cfg(unix)]
impl HookProcessGroupGuard {
    fn new(leader_pid: u32) -> Self {
        Self {
            leader_pid,
            armed: true,
        }
    }

    fn disarm_if_gone(&mut self) {
        if matches!(
            crate::process::signals::process_group_exists(self.leader_pid),
            Ok(false)
        ) {
            self.armed = false;
        }
    }

    fn terminate_remaining(&mut self) {
        match crate::process::signals::process_group_exists(self.leader_pid) {
            Ok(false) => self.armed = false,
            Ok(true) => {
                let _ = crate::process::signals::signal_process_group(
                    self.leader_pid,
                    libc::SIGKILL,
                    "SIGKILL",
                );
            }
            Err(error) => {
                tracing::debug!(
                    pid = self.leader_pid,
                    %error,
                    "Failed to inspect hook process group after command completion"
                );
            }
        }
    }
}

#[cfg(unix)]
impl Drop for HookProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = crate::process::signals::signal_process_group(
                self.leader_pid,
                libc::SIGKILL,
                "SIGKILL",
            );
        }
    }
}

async fn terminate_hook_process_tree(child: &mut tokio::process::Child) {
    let pid = child.id();

    #[cfg(unix)]
    if let Some(pid) = pid {
        if let Err(error) =
            crate::process::signals::signal_process_group(pid, libc::SIGKILL, "SIGKILL")
        {
            tracing::debug!(pid, %error, "Failed to kill hook process group");
        }
    }

    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = tokio::time::timeout(
            HOOK_PROCESS_REAP_TIMEOUT,
            tokio::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output(),
        )
        .await;
    }

    if let Err(error) = child.start_kill() {
        tracing::debug!(?pid, %error, "Failed to kill hook process directly");
    }
    if tokio::time::timeout(HOOK_PROCESS_REAP_TIMEOUT, child.wait())
        .await
        .is_err()
    {
        tracing::warn!(?pid, "Timed out while reaping terminated hook process");
    }
}

async fn read_bounded_stderr(
    mut stderr: tokio::process::ChildStderr,
) -> std::io::Result<BoundedOutput> {
    use tokio::io::AsyncReadExt;

    let mut bytes = Vec::with_capacity(MAX_HOOK_STDERR_BYTES);
    let mut buffer = [0_u8; 4096];
    let mut truncated = false;
    loop {
        let count = stderr.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = MAX_HOOK_STDERR_BYTES.saturating_sub(bytes.len());
        let retained = remaining.min(count);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < count;
    }
    Ok(BoundedOutput { bytes, truncated })
}

/// Executor for user hooks. Runs shell commands and interprets results.
pub struct UserHookExecutor;

impl UserHookExecutor {
    /// Execute a hook command with JSON input.
    ///
    /// The command receives JSON on stdin with tool call details.
    /// Exit codes:
    /// - 0: Continue (stdout/stderr not shown)
    /// - 2: Block tool, show stderr to model
    /// - Other: Warn user with stderr, continue
    pub async fn execute(
        hook: &UserHook,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> UserHookResult {
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt;
        use tokio::process::Command;

        let input = serde_json::json!({
            "tool_name": tool_name,
            "tool_input": params,
            "hook_id": hook.id,
            "hook_type": hook.hook_type.display_name(),
            "plugin_id": hook.source.plugin_id(),
            "cwd": hook.working_dir().map(|path| path.to_string_lossy()),
        });

        let input_str = match serde_json::to_string(&input) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(hook_id = %hook.id, "Failed to serialize hook input: {}", e);
                return UserHookResult::Continue;
            }
        };

        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(&hook.command)
            .stdin(Stdio::piped())
            // Hook stdout is intentionally not surfaced. Discarding it avoids
            // retaining unbounded output while still preventing pipe blockage.
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(working_dir) = hook.working_dir() {
            command.current_dir(working_dir);
        }
        if let UserHookSource::Package {
            plugin_id,
            config_path,
        } = &hook.source
        {
            command
                .env("KRUSTY_PLUGIN_ID", plugin_id)
                .env("KRUSTY_HOOK_CONFIG", config_path);
            if let Some(package_root) = hook.working_dir() {
                command
                    .env("KRUSTY_PLUGIN_ROOT", package_root)
                    // Claude plugin command hooks conventionally resolve
                    // bundled scripts through this variable.
                    .env("CLAUDE_PLUGIN_ROOT", package_root);
            }
        }
        #[cfg(unix)]
        command.process_group(0);

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(hook_id = %hook.id, command = %hook.command, "Failed to spawn hook: {}", e);
                return UserHookResult::Warn {
                    message: format!("Hook failed to spawn: {}", e),
                };
            }
        };

        #[cfg(unix)]
        let mut process_group_guard = child.id().map(HookProcessGroupGuard::new);

        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(read_bounded_stderr(stderr)));

        let timeout_seconds = hook.timeout_seconds.clamp(1, 300);
        // One deadline covers both delivery of the JSON request and waiting for
        // the command. A hook that never reads stdin must not be able to block
        // before the process timeout starts.
        let execution = async {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(error) = stdin.write_all(input_str.as_bytes()).await {
                    tracing::warn!(hook_id = %hook.id, %error, "Failed to write to hook stdin");
                }
            }
            child.wait().await
        };
        let status =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds), execution)
                .await
            {
                Ok(Ok(status)) => status,
                Ok(Err(e)) => {
                    terminate_hook_process_tree(&mut child).await;
                    #[cfg(unix)]
                    if let Some(guard) = &mut process_group_guard {
                        guard.disarm_if_gone();
                    }
                    if let Some(stderr_task) = stderr_task {
                        stderr_task.abort();
                        let _ = stderr_task.await;
                    }
                    tracing::warn!(hook_id = %hook.id, "Hook execution failed: {}", e);
                    return UserHookResult::Warn {
                        message: format!("Hook execution failed: {}", e),
                    };
                }
                Err(_) => {
                    terminate_hook_process_tree(&mut child).await;
                    #[cfg(unix)]
                    if let Some(guard) = &mut process_group_guard {
                        guard.disarm_if_gone();
                    }
                    if let Some(stderr_task) = stderr_task {
                        stderr_task.abort();
                        let _ = stderr_task.await;
                    }
                    tracing::warn!(hook_id = %hook.id, timeout_seconds, "Hook timed out");
                    return UserHookResult::Warn {
                        message: format!("Hook timed out after {} seconds", timeout_seconds),
                    };
                }
            };

        #[cfg(unix)]
        if let Some(guard) = &mut process_group_guard {
            // A hook command is not a background-process launcher. If the
            // shell returned while descendants remain, terminate them before
            // waiting for inherited stderr pipes to close.
            guard.terminate_remaining();
        }
        let bounded_stderr = match stderr_task {
            Some(mut task) => {
                match tokio::time::timeout(std::time::Duration::from_secs(1), &mut task).await {
                    Ok(Ok(Ok(output))) => output,
                    Ok(Ok(Err(error))) => {
                        tracing::warn!(hook_id = %hook.id, %error, "Failed to read hook stderr");
                        BoundedOutput {
                            bytes: Vec::new(),
                            truncated: false,
                        }
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(hook_id = %hook.id, %error, "Hook stderr reader failed");
                        BoundedOutput {
                            bytes: Vec::new(),
                            truncated: false,
                        }
                    }
                    Err(_) => {
                        task.abort();
                        let _ = task.await;
                        tracing::warn!(hook_id = %hook.id, "Hook stderr pipe did not close");
                        BoundedOutput {
                            bytes: Vec::new(),
                            truncated: true,
                        }
                    }
                }
            }
            None => BoundedOutput {
                bytes: Vec::new(),
                truncated: false,
            },
        };
        #[cfg(unix)]
        if let Some(guard) = &mut process_group_guard {
            guard.disarm_if_gone();
        }
        let exit_code = status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&bounded_stderr.bytes);
        let stderr_trimmed = stderr.trim();
        let mut visible_end = stderr_trimmed.len().min(MAX_HOOK_STDERR_BYTES);
        while !stderr_trimmed.is_char_boundary(visible_end) {
            visible_end = visible_end.saturating_sub(1);
        }
        let visible_stderr = if bounded_stderr.truncated || visible_end < stderr_trimmed.len() {
            format!(
                "{}\n[hook stderr truncated]",
                &stderr_trimmed[..visible_end]
            )
        } else {
            stderr_trimmed[..visible_end].to_string()
        };

        tracing::debug!(
            hook_id = %hook.id,
            exit_code,
            stderr_len = bounded_stderr.bytes.len(),
            stderr_truncated = bounded_stderr.truncated,
            "Hook execution complete"
        );

        match exit_code {
            0 => UserHookResult::Continue,
            2 => {
                let reason = if visible_stderr.is_empty() {
                    "Hook blocked execution".to_string()
                } else {
                    visible_stderr
                };
                UserHookResult::Block { reason }
            }
            _ => {
                let message = if visible_stderr.is_empty() {
                    format!("Hook exited with code {}", exit_code)
                } else {
                    visible_stderr
                };
                UserHookResult::Warn { message }
            }
        }
    }

    /// Execute all matching hooks for a tool.
    ///
    /// Returns `Block` if any hook blocks, otherwise `Continue`.
    pub async fn execute_matching(
        manager: &mut UserHookManager,
        hook_type: UserHookType,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> UserHookResult {
        Self::execute_matching_for_user(manager, hook_type, tool_name, params, None).await
    }

    /// Execute only global/package hooks plus hooks owned by `user_id`.
    pub async fn execute_matching_for_user(
        manager: &mut UserHookManager,
        hook_type: UserHookType,
        tool_name: &str,
        params: &serde_json::Value,
        user_id: Option<&str>,
    ) -> UserHookResult {
        let hooks: Vec<UserHook> = manager
            .matching_hooks_for_user(hook_type, tool_name, user_id)
            .iter()
            .map(|h| (*h).clone())
            .collect();

        for hook in hooks {
            let result = Self::execute(&hook, tool_name, params).await;
            match result {
                UserHookResult::Block { reason } => {
                    tracing::info!(
                        hook_id = %hook.id,
                        tool = tool_name,
                        "User hook blocked execution: {}",
                        reason
                    );
                    return UserHookResult::Block { reason };
                }
                UserHookResult::Warn { message } => {
                    tracing::warn!(
                        hook_id = %hook.id,
                        tool = tool_name,
                        "User hook warning: {}",
                        message
                    );
                }
                UserHookResult::Continue => {}
            }
        }

        UserHookResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::{UserHookExecutor, MAX_HOOK_STDERR_BYTES};
    use crate::agent::user_hooks::{UserHook, UserHookResult, UserHookType};
    use serde_json::json;

    fn create_test_hook(hook_type: UserHookType, pattern: &str, command: &str) -> UserHook {
        UserHook::new(hook_type, pattern.to_string(), command.to_string())
    }

    #[tokio::test]
    async fn test_user_hook_executor_success() {
        let hook = create_test_hook(UserHookType::PreToolUse, "Write", "exit 0");
        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;
        assert!(matches!(result, UserHookResult::Continue));
    }

    #[tokio::test]
    async fn test_user_hook_executor_block() {
        let hook = create_test_hook(UserHookType::PreToolUse, "Write", "exit 2");
        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;
        assert!(matches!(result, UserHookResult::Block { .. }));
    }

    #[tokio::test]
    async fn test_user_hook_executor_warn() {
        let hook = create_test_hook(UserHookType::PreToolUse, "Write", "exit 1");
        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;
        assert!(matches!(result, UserHookResult::Warn { .. }));
    }

    #[tokio::test]
    async fn test_user_hook_executor_stderr_in_block_reason() {
        let hook = create_test_hook(
            UserHookType::PreToolUse,
            "Write",
            "echo 'Blocked because reason' >&2; exit 2",
        );

        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;

        if let UserHookResult::Block { reason } = result {
            assert!(reason.contains("Blocked because reason"));
        } else {
            panic!("Expected Block result");
        }
    }

    #[tokio::test]
    async fn test_user_hook_executor_empty_stderr_default_message() {
        let hook = create_test_hook(UserHookType::PreToolUse, "Write", "exit 2");
        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;

        if let UserHookResult::Block { reason } = result {
            assert_eq!(reason, "Hook blocked execution");
        } else {
            panic!("Expected Block result");
        }
    }

    #[tokio::test]
    async fn test_user_hook_executor_nonexistent_command_warns() {
        let hook = create_test_hook(
            UserHookType::PreToolUse,
            "Write",
            "this_command_does_not_exist_12345",
        );

        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;
        assert!(matches!(result, UserHookResult::Warn { .. }));
    }

    #[tokio::test]
    async fn test_user_hook_executor_json_input() {
        let hook = create_test_hook(
            UserHookType::PreToolUse,
            "Write",
            "cat | grep -q '\"test\"'",
        );

        let params = json!({"test": "value"});
        let result = UserHookExecutor::execute(&hook, "Write", &params).await;

        assert!(matches!(
            result,
            UserHookResult::Continue | UserHookResult::Warn { .. }
        ));
    }

    #[tokio::test]
    async fn test_user_hook_executor_bounds_stderr() {
        let hook = create_test_hook(
            UserHookType::PreToolUse,
            "Write",
            r#"i=0; while [ "$i" -lt 20000 ]; do printf x >&2; i=$((i + 1)); done; exit 2"#,
        );

        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;
        let UserHookResult::Block { reason } = result else {
            panic!("expected bounded block output");
        };
        assert!(reason.contains("[hook stderr truncated]"));
        assert!(reason.len() <= MAX_HOOK_STDERR_BYTES + 32);
    }

    #[tokio::test]
    async fn test_user_hook_executor_honors_timeout() {
        let mut hook = create_test_hook(UserHookType::PreToolUse, "Write", "sleep 5");
        hook.timeout_seconds = 1;
        let started = std::time::Instant::now();

        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;

        assert!(matches!(result, UserHookResult::Warn { .. }));
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[tokio::test]
    async fn test_user_hook_executor_times_out_while_stdin_is_blocked() {
        // `sleep` never reads stdin. This payload is deliberately larger than a
        // normal OS pipe, so the executor must time out the pending write and
        // terminate the hook process tree rather than waiting for `sleep`.
        let mut hook = create_test_hook(UserHookType::PreToolUse, "Write", "sleep 30");
        hook.timeout_seconds = 1;
        let params = json!({"content": "x".repeat(4 * 1024 * 1024)});
        let started = std::time::Instant::now();

        let result = UserHookExecutor::execute(&hook, "Write", &params).await;

        let UserHookResult::Warn { message } = result else {
            panic!("expected blocked stdin to time out");
        };
        assert!(message.contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_package_hook_runs_from_package_root() {
        let temp = tempfile::tempdir().expect("package root");
        std::fs::write(temp.path().join("marker"), "present").expect("write marker");
        let root = temp.path().canonicalize().expect("canonical package root");
        let hook = UserHook::new_package(
            "package:test:cwd".to_string(),
            UserHookType::PreToolUse,
            ".*".to_string(),
            r#"test -f marker && test "$KRUSTY_PLUGIN_ROOT" = "$(pwd)" && test "$CLAUDE_PLUGIN_ROOT" = "$(pwd)""#
                .to_string(),
            true,
            30,
            "test".to_string(),
            root.join("hooks.json"),
            root,
        );

        let result = UserHookExecutor::execute(&hook, "Write", &json!({})).await;

        assert!(matches!(result, UserHookResult::Continue));
    }
}
