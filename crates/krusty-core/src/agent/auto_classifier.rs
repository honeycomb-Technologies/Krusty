use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tracing::info;

use crate::agent::hooks::{HookResult, PreToolHook};
use crate::agent::loop_events::LoopEvent;
use crate::ai::client::AiClient;
use crate::tools::registry::{PermissionMode, ToolContext};

const SAFE_TOOLS: &[&str] = &[
    "read",
    "grep",
    "glob",
    "list",
    "memory",
    "skill",
    "send_user_message",
    "sleep",
    "ask_user",
    "create_task",
    "update_task",
    "list_tasks",
    "create_report",
    "list_reports",
    "read_report",
    "set_work_mode",
    "enter_plan_mode",
    "set_workspace_context",
];

const CLASSIFIER_PROMPT: &str = r#"You are a safety classifier for an AI coding agent operating in autonomous mode. Your job is to decide whether a tool call should be ALLOWED or BLOCKED.

## ALLOW when:
- Editing, creating, or deleting files within the project directory
- Running tests, builds, linters, formatters, or type checkers
- Git read operations: status, diff, log, show, rev-parse, ls-files
- Git write operations: add, commit, branch, checkout, merge, rebase, stash
- Reading environment variables or config files within the project
- Installing project dependencies via lockfile (npm ci, cargo build, pip install -r)

## BLOCK when:
- Destructive operations targeting paths outside the project (rm -rf /, modifying /etc, /usr, $HOME)
- Network requests to unknown or suspicious hosts (curl to arbitrary URLs, wget piped to shell)
- System package installation (apt, brew, pacman, yum) or global tool installs (npm -g, pip install without -r)
- Privilege escalation (sudo, doas, su, chmod 777)
- Accessing credentials, secrets, or tokens outside the project (.ssh, .aws, .gnupg, /etc/shadow)
- Git force push, branch -D on main/master, or reset --hard to remote
- Spawning background daemons, opening network listeners, or modifying system services

Respond with exactly one line: ALLOW or BLOCK followed by a one-sentence reason.
Example: ALLOW Editing a source file within the project.
Example: BLOCK Attempting to force-push to the main branch."#;

const FAST_MAX_TOKENS: usize = 64;
const THINKING_MAX_TOKENS: usize = 4096;
const SANITIZED_ARG_LIMIT: usize = 2048;

pub struct AutoClassifierHook {
    ai_client: Arc<AiClient>,
}

impl AutoClassifierHook {
    pub fn new(ai_client: Arc<AiClient>) -> Self {
        Self { ai_client }
    }

    fn is_safe_tool(name: &str) -> bool {
        SAFE_TOOLS.contains(&name)
    }

    fn sanitize_args(params: &Value) -> String {
        let raw = params.to_string();
        if raw.len() <= SANITIZED_ARG_LIMIT {
            raw
        } else {
            format!("{}...(truncated)", &raw[..SANITIZED_ARG_LIMIT])
        }
    }

    fn parse_verdict(response: &str) -> Option<bool> {
        let normalized = response.trim_start().to_ascii_uppercase();
        if normalized.starts_with("ALLOW") {
            Some(true)
        } else if normalized.starts_with("BLOCK") {
            Some(false)
        } else {
            None
        }
    }

    fn build_user_prompt(name: &str, sanitized_args: &str) -> String {
        format!("Tool: {name}\nArguments: {sanitized_args}")
    }

    fn emit_decision(
        ctx: &ToolContext,
        tool_name: &str,
        decision: &str,
        reason: String,
        stage: u8,
    ) {
        if let Some(tx) = ctx.loop_event_tx.as_ref() {
            let _ = tx.send(LoopEvent::ClassifierDecision {
                tool_name: tool_name.to_string(),
                decision: decision.to_string(),
                reason,
                stage,
            });
        }
    }

    async fn classify(&self, name: &str, params: &Value, ctx: &ToolContext) -> HookResult {
        let sanitized = Self::sanitize_args(params);
        let user_prompt = Self::build_user_prompt(name, &sanitized);
        let model = &self.ai_client.config().model;

        // Stage 1: fast classification
        match self
            .ai_client
            .call_simple(model, CLASSIFIER_PROMPT, &user_prompt, FAST_MAX_TOKENS)
            .await
        {
            Ok(response) => match Self::parse_verdict(&response) {
                Some(true) => {
                    let reason = response.trim().to_string();
                    info!(
                        tool = name,
                        verdict = "ALLOW",
                        stage = 1,
                        reason = %reason,
                        "Classifier approved tool call"
                    );
                    Self::emit_decision(ctx, name, "allow", reason, 1);
                    return HookResult::Continue;
                }
                Some(false) => {
                    info!(
                        tool = name,
                        verdict = "BLOCK",
                        stage = 1,
                        "Stage 1 blocked, escalating to stage 2"
                    );
                }
                None => {
                    info!(tool = name, response = %response, "Stage 1 ambiguous, escalating to stage 2");
                }
            },
            Err(e) => {
                info!(tool = name, error = %e, "Stage 1 failed, escalating to stage 2");
            }
        }

        // Stage 2: thinking classification (more tokens to reason about edge cases)
        match self
            .ai_client
            .call_simple(model, CLASSIFIER_PROMPT, &user_prompt, THINKING_MAX_TOKENS)
            .await
        {
            Ok(response) => match Self::parse_verdict(&response) {
                Some(true) => {
                    let reason = response.trim().to_string();
                    info!(
                        tool = name,
                        verdict = "ALLOW",
                        stage = 2,
                        reason = %reason,
                        "Classifier approved tool call on appeal"
                    );
                    Self::emit_decision(ctx, name, "allow", reason, 2);
                    HookResult::Continue
                }
                Some(false) => {
                    let reason = response.trim().to_string();
                    info!(tool = name, verdict = "BLOCK", stage = 2, reason = %reason, "Classifier blocked tool call");
                    Self::emit_decision(ctx, name, "block", reason.clone(), 2);
                    HookResult::Block {
                        reason: format!("Auto-classifier blocked: {reason}"),
                    }
                }
                None => {
                    let reason =
                        "Auto-classifier: ambiguous verdict, defaulting to deny".to_string();
                    info!(tool = name, response = %response, reason = %reason, "Stage 2 ambiguous, denying");
                    Self::emit_decision(ctx, name, "block", reason.clone(), 2);
                    HookResult::Block { reason }
                }
            },
            Err(e) => {
                let reason = format!("Auto-classifier error, defaulting to deny: {e}");
                info!(tool = name, error = %e, reason = %reason, "Stage 2 failed, denying");
                Self::emit_decision(ctx, name, "block", reason.clone(), 2);
                HookResult::Block { reason }
            }
        }
    }
}

#[async_trait]
impl PreToolHook for AutoClassifierHook {
    async fn before_execute(&self, name: &str, params: &Value, ctx: &ToolContext) -> HookResult {
        if ctx.permission_mode != PermissionMode::Autonomous {
            return HookResult::Continue;
        }

        if Self::is_safe_tool(name) {
            let reason = format!("Safe tool bypass: '{name}' is on the autonomous allowlist");
            info!(tool = name, reason = %reason, "Classifier bypass: safe tool");
            Self::emit_decision(ctx, name, "allow", reason, 0);
            return HookResult::Continue;
        }

        self.classify(name, params, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn safe_tool_recognized() {
        for tool in SAFE_TOOLS {
            assert!(
                AutoClassifierHook::is_safe_tool(tool),
                "{tool} should be safe"
            );
        }
        assert!(!AutoClassifierHook::is_safe_tool("bash"));
        assert!(!AutoClassifierHook::is_safe_tool("write"));
        assert!(!AutoClassifierHook::is_safe_tool("apply_patch"));
    }

    #[test]
    fn parse_verdict_allow() {
        assert_eq!(
            AutoClassifierHook::parse_verdict("ALLOW editing file"),
            Some(true)
        );
        assert_eq!(
            AutoClassifierHook::parse_verdict("allow editing file"),
            Some(true)
        );
        assert_eq!(
            AutoClassifierHook::parse_verdict("  ALLOW reason"),
            Some(true)
        );
    }

    #[test]
    fn parse_verdict_block() {
        assert_eq!(
            AutoClassifierHook::parse_verdict("BLOCK dangerous"),
            Some(false)
        );
        assert_eq!(
            AutoClassifierHook::parse_verdict("block dangerous"),
            Some(false)
        );
        assert_eq!(
            AutoClassifierHook::parse_verdict("  BLOCK reason"),
            Some(false)
        );
    }

    #[test]
    fn parse_verdict_ambiguous() {
        assert_eq!(AutoClassifierHook::parse_verdict("maybe allow"), None);
        assert_eq!(AutoClassifierHook::parse_verdict(""), None);
        assert_eq!(
            AutoClassifierHook::parse_verdict("I think this is fine"),
            None
        );
    }

    #[test]
    fn sanitize_args_short() {
        let params = json!({"command": "ls -la"});
        let result = AutoClassifierHook::sanitize_args(&params);
        assert_eq!(result, params.to_string());
    }

    #[test]
    fn sanitize_args_truncated() {
        let long_value = "x".repeat(SANITIZED_ARG_LIMIT + 500);
        let params = json!({"data": long_value});
        let result = AutoClassifierHook::sanitize_args(&params);
        assert!(result.len() < params.to_string().len());
        assert!(result.ends_with("...(truncated)"));
    }

    #[tokio::test]
    async fn bypass_in_supervised_mode() {
        let config = crate::ai::client::AiClientConfig::default();
        let client = Arc::new(AiClient::new(config, "test-key".to_string()));
        let hook = AutoClassifierHook::new(client);
        let ctx = ToolContext::default(); // Supervised by default

        let result = hook
            .before_execute("bash", &json!({"command": "rm -rf /"}), &ctx)
            .await;
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn bypass_safe_tool_in_autonomous() {
        let config = crate::ai::client::AiClientConfig::default();
        let client = Arc::new(AiClient::new(config, "test-key".to_string()));
        let hook = AutoClassifierHook::new(client);
        let ctx = ToolContext {
            permission_mode: PermissionMode::Autonomous,
            ..Default::default()
        };

        let result = hook
            .before_execute("read", &json!({"path": "/etc/passwd"}), &ctx)
            .await;
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn safe_tool_bypass_emits_classifier_event() {
        let config = crate::ai::client::AiClientConfig::default();
        let client = Arc::new(AiClient::new(config, "test-key".to_string()));
        let hook = AutoClassifierHook::new(client);
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ToolContext {
            permission_mode: PermissionMode::Autonomous,
            loop_event_tx: Some(event_tx),
            ..Default::default()
        };

        let result = hook
            .before_execute("read", &json!({"path": "Cargo.toml"}), &ctx)
            .await;
        assert!(matches!(result, HookResult::Continue));

        let event = event_rx.recv().await.expect("classifier event");
        assert!(matches!(
            event,
            LoopEvent::ClassifierDecision {
                tool_name,
                decision,
                stage,
                ..
            } if tool_name == "read" && decision == "allow" && stage == 0
        ));
    }
}
