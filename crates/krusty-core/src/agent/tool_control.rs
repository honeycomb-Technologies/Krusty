//! Centralized tool-control policy for the agent loop.
//!
//! Keeps approval, retry, and result-shaping behavior in one place so the
//! executor stays focused on dispatch.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::types::{AiToolCall, Content};
use crate::tools::registry::{authorize_tool_call, tool_policy, PermissionMode, ToolResult};

use super::history_policy::build_history_tool_result;
use super::loop_events::{LoopEvent, LoopInput};

const MAX_TOOL_OUTPUT_CHARS: usize = 30_000;
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
const READ_ONLY_TIMEOUT_RETRIES: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorizationDecision {
    Execute,
    Deny(ApprovalDenial),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalDenial {
    UserRejected,
    Cancelled,
    ChannelClosed,
    TimedOut,
}

impl ApprovalDenial {
    pub(crate) fn tool_result(self) -> ToolResult {
        match self {
            Self::UserRejected => {
                ToolResult::error_with_code("permission_denied", "Tool execution denied by user")
            }
            Self::Cancelled => ToolResult::error_with_code(
                "permission_denied",
                "Tool execution cancelled before approval",
            ),
            Self::ChannelClosed => ToolResult::error_with_code(
                "permission_denied",
                "Tool approval channel closed before a decision was received",
            ),
            Self::TimedOut => {
                ToolResult::error_with_code("timeout", "Tool approval timed out after 5 minutes")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDirective {
    Stop,
    RetryOnce { reason: &'static str },
}

pub(crate) struct ToolControl {
    permission_mode: PermissionMode,
    approval_timeout: Duration,
}

impl ToolControl {
    pub(crate) fn new(permission_mode: PermissionMode) -> Self {
        Self {
            permission_mode,
            approval_timeout: APPROVAL_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_approval_timeout(permission_mode: PermissionMode, approval_timeout: Duration) -> Self {
        Self {
            permission_mode,
            approval_timeout,
        }
    }

    pub(crate) fn requires_approval(&self, call: &AiToolCall) -> bool {
        authorize_tool_call(&call.name, &call.arguments, self.permission_mode, false)
            .requires_approval()
    }

    pub(crate) async fn authorize(
        &self,
        call: &AiToolCall,
        event_tx: &mpsc::UnboundedSender<LoopEvent>,
        input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    ) -> AuthorizationDecision {
        if !self.requires_approval(call) {
            return AuthorizationDecision::Execute;
        }

        let _ = event_tx.send(LoopEvent::ToolApprovalRequired {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        });

        match self.wait_for_approval(call, input_rx).await {
            ApprovalDecision::Approved => {
                let _ = event_tx.send(LoopEvent::ToolApproved {
                    id: call.id.clone(),
                });
                AuthorizationDecision::Execute
            }
            ApprovalDecision::Denied(denial) => {
                let _ = event_tx.send(LoopEvent::ToolDenied {
                    id: call.id.clone(),
                });
                AuthorizationDecision::Deny(denial)
            }
        }
    }

    pub(crate) fn retry_directive(
        &self,
        call: &AiToolCall,
        result: &ToolResult,
        retries_attempted: usize,
    ) -> RetryDirective {
        if retries_attempted >= READ_ONLY_TIMEOUT_RETRIES {
            return RetryDirective::Stop;
        }

        if !tool_policy(&call.name).retry_timeout_once {
            return RetryDirective::Stop;
        }

        if matches!(
            extract_error_code(&result.output).as_deref(),
            Some("timeout")
        ) {
            return RetryDirective::RetryOnce {
                reason: "read-only tool timed out",
            };
        }

        RetryDirective::Stop
    }

    pub(crate) fn publish_result(
        &self,
        call: &AiToolCall,
        result: &ToolResult,
        event_tx: &mpsc::UnboundedSender<LoopEvent>,
    ) -> Content {
        let output = truncate_output(&result.output);
        let _ = event_tx.send(LoopEvent::ToolResult {
            id: call.id.clone(),
            output,
            is_error: result.is_error,
        });

        Content::ToolResult {
            tool_use_id: call.id.clone(),
            output: build_history_tool_result(&call.name, &result.output, result.is_error),
            is_error: result.is_error.then_some(true),
        }
    }

    async fn wait_for_approval(
        &self,
        call: &AiToolCall,
        input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    ) -> ApprovalDecision {
        let deadline = tokio::time::Instant::now() + self.approval_timeout;

        loop {
            match tokio::time::timeout_at(deadline, input_rx.recv()).await {
                Ok(Some(LoopInput::ToolApproval {
                    tool_call_id,
                    approved,
                })) if tool_call_id == call.id => {
                    return if approved {
                        ApprovalDecision::Approved
                    } else {
                        ApprovalDecision::Denied(ApprovalDenial::UserRejected)
                    };
                }
                Ok(Some(LoopInput::Cancel)) => {
                    return ApprovalDecision::Denied(ApprovalDenial::Cancelled);
                }
                Ok(Some(_)) => continue,
                Ok(None) => return ApprovalDecision::Denied(ApprovalDenial::ChannelClosed),
                Err(_) => return ApprovalDecision::Denied(ApprovalDenial::TimedOut),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalDecision {
    Approved,
    Denied(ApprovalDenial),
}

pub(crate) fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_TOOL_OUTPUT_CHARS {
        return output.to_string();
    }

    let truncated_len = floor_char_boundary(output, MAX_TOOL_OUTPUT_CHARS);
    let truncated = &output[..truncated_len];
    let break_point = truncated.rfind('\n').unwrap_or(truncated_len);
    let clean = &output[..break_point];
    format!(
        "{}\n\n[... OUTPUT TRUNCATED: {} chars -> {} chars ...]",
        clean,
        output.len(),
        clean.len()
    )
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut boundary = index.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn extract_error_code(output: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(output).ok()?;
    parsed
        .get("error")
        .and_then(|value| value.get("code"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read_call() -> AiToolCall {
        AiToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: json!({"file_path":"src/lib.rs"}),
        }
    }

    fn edit_call() -> AiToolCall {
        AiToolCall {
            id: "call_2".to_string(),
            name: "edit".to_string(),
            arguments: json!({"file_path":"src/lib.rs"}),
        }
    }

    fn agent_call(agent_type: &str) -> AiToolCall {
        AiToolCall {
            id: format!("call_agent_{agent_type}"),
            name: "agent".to_string(),
            arguments: json!({"agent_type": agent_type}),
        }
    }

    #[test]
    fn approval_only_required_for_supervised_write_tools() {
        let supervised = ToolControl::new(PermissionMode::Supervised);
        let autonomous = ToolControl::new(PermissionMode::Autonomous);

        assert!(supervised.requires_approval(&edit_call()));
        assert!(!supervised.requires_approval(&read_call()));
        assert!(!autonomous.requires_approval(&edit_call()));
        assert!(supervised.requires_approval(&agent_call("build")));
        assert!(!supervised.requires_approval(&agent_call("explore")));
        assert!(!autonomous.requires_approval(&agent_call("build")));
    }

    #[tokio::test]
    async fn authorize_emits_approval_events_for_agent_build() {
        let control = ToolControl::new(PermissionMode::Supervised);
        let call = agent_call("build");
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (input_tx, mut input_rx) = mpsc::unbounded_channel();
        input_tx
            .send(LoopInput::ToolApproval {
                tool_call_id: call.id.clone(),
                approved: true,
            })
            .unwrap();

        let decision = control.authorize(&call, &event_tx, &mut input_rx).await;

        assert_eq!(decision, AuthorizationDecision::Execute);
        assert!(matches!(
            event_rx.recv().await,
            Some(LoopEvent::ToolApprovalRequired { name, .. }) if name == "agent"
        ));
        assert!(matches!(
            event_rx.recv().await,
            Some(LoopEvent::ToolApproved { .. })
        ));
    }

    #[tokio::test]
    async fn authorize_emits_approval_events_for_write_tools() {
        let control = ToolControl::new(PermissionMode::Supervised);
        let call = edit_call();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (input_tx, mut input_rx) = mpsc::unbounded_channel();
        input_tx
            .send(LoopInput::ToolApproval {
                tool_call_id: call.id.clone(),
                approved: true,
            })
            .unwrap();

        let decision = control.authorize(&call, &event_tx, &mut input_rx).await;

        assert_eq!(decision, AuthorizationDecision::Execute);
        assert!(matches!(
            event_rx.recv().await,
            Some(LoopEvent::ToolApprovalRequired { .. })
        ));
        assert!(matches!(
            event_rx.recv().await,
            Some(LoopEvent::ToolApproved { .. })
        ));
    }

    #[tokio::test]
    async fn authorize_denies_when_cancelled() {
        let control = ToolControl::new(PermissionMode::Supervised);
        let call = edit_call();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (input_tx, mut input_rx) = mpsc::unbounded_channel();
        input_tx.send(LoopInput::Cancel).unwrap();

        let decision = control.authorize(&call, &event_tx, &mut input_rx).await;

        assert_eq!(
            decision,
            AuthorizationDecision::Deny(ApprovalDenial::Cancelled)
        );
        assert!(matches!(
            event_rx.recv().await,
            Some(LoopEvent::ToolApprovalRequired { .. })
        ));
        assert!(matches!(
            event_rx.recv().await,
            Some(LoopEvent::ToolDenied { .. })
        ));
    }

    #[tokio::test]
    async fn authorize_times_out_cleanly() {
        let control = ToolControl::with_approval_timeout(
            PermissionMode::Supervised,
            Duration::from_millis(1),
        );
        let call = edit_call();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (_input_tx, mut input_rx) = mpsc::unbounded_channel();

        let decision = control.authorize(&call, &event_tx, &mut input_rx).await;

        assert_eq!(
            decision,
            AuthorizationDecision::Deny(ApprovalDenial::TimedOut)
        );
    }

    #[test]
    fn retries_only_read_only_timeouts_once() {
        let control = ToolControl::new(PermissionMode::Autonomous);
        let timeout = ToolResult::error_with_code("timeout", "timed out");
        let denied = ToolResult::error_with_code("permission_denied", "denied");

        assert_eq!(
            control.retry_directive(&read_call(), &timeout, 0),
            RetryDirective::RetryOnce {
                reason: "read-only tool timed out"
            }
        );
        assert_eq!(
            control.retry_directive(&read_call(), &timeout, 1),
            RetryDirective::Stop
        );
        assert_eq!(
            control.retry_directive(&edit_call(), &timeout, 0),
            RetryDirective::Stop
        );
        assert_eq!(
            control.retry_directive(&read_call(), &denied, 0),
            RetryDirective::Stop
        );
    }

    #[test]
    fn truncate_output_preserves_utf8_boundaries() {
        let source = "x".repeat(MAX_TOOL_OUTPUT_CHARS + 10);
        let truncated = truncate_output(&source);
        assert!(truncated.contains("OUTPUT TRUNCATED"));
    }
}
