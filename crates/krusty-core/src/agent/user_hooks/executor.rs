use super::manager::UserHookManager;
use super::model::{UserHook, UserHookResult, UserHookType};

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
        });

        let input_str = match serde_json::to_string(&input) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(hook_id = %hook.id, "Failed to serialize hook input: {}", e);
                return UserHookResult::Continue;
            }
        };

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&hook.command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(hook_id = %hook.id, command = %hook.command, "Failed to spawn hook: {}", e);
                return UserHookResult::Warn {
                    message: format!("Hook failed to spawn: {}", e),
                };
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(input_str.as_bytes()).await {
                tracing::warn!(hook_id = %hook.id, "Failed to write to hook stdin: {}", e);
            }
        }

        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            child.wait_with_output(),
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                tracing::warn!(hook_id = %hook.id, "Hook execution failed: {}", e);
                return UserHookResult::Warn {
                    message: format!("Hook execution failed: {}", e),
                };
            }
            Err(_) => {
                tracing::warn!(hook_id = %hook.id, "Hook timed out after 30s");
                return UserHookResult::Warn {
                    message: "Hook timed out after 30 seconds".to_string(),
                };
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_trimmed = stderr.trim();

        tracing::debug!(
            hook_id = %hook.id,
            exit_code,
            stderr_len = stderr.len(),
            "Hook execution complete"
        );

        match exit_code {
            0 => UserHookResult::Continue,
            2 => {
                let reason = if stderr_trimmed.is_empty() {
                    "Hook blocked execution".to_string()
                } else {
                    stderr_trimmed.to_string()
                };
                UserHookResult::Block { reason }
            }
            _ => {
                let message = if stderr_trimmed.is_empty() {
                    format!("Hook exited with code {}", exit_code)
                } else {
                    stderr_trimmed.to_string()
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
        let hooks: Vec<UserHook> = manager
            .matching_hooks(hook_type, tool_name)
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
    use super::UserHookExecutor;
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
            UserHookResult::Continue { .. } | UserHookResult::Warn { .. }
        ));
    }
}
