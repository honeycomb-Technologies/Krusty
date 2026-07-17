//! Centralized tool-control policy for the agent loop.
//!
//! Keeps approval, retry, and result-shaping behavior in one place so the
//! executor stays focused on dispatch.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::types::{AiToolCall, Content};
use crate::tools::registry::{
    authorize_tool_call, effective_tool_call, tool_policy_for_call, PermissionMode, ToolResult,
};

use super::history_policy::build_history_tool_result;
#[cfg(test)]
use super::loop_events::LoopInput;
use super::loop_events::{LoopEvent, LoopInputInbox, ToolApprovalInput};

const MAX_TOOL_OUTPUT_CHARS: usize = 30_000;
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
const READ_ONLY_TIMEOUT_RETRIES: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorizationDecision {
    Execute,
    Deny(ApprovalDenial),
    Cancel,
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
        input_inbox: &mut LoopInputInbox,
    ) -> AuthorizationDecision {
        if !self.requires_approval(call) {
            return AuthorizationDecision::Execute;
        }

        let (effective_name, effective_arguments) =
            effective_tool_call(&call.name, &call.arguments);
        let _ = event_tx.send(LoopEvent::ToolApprovalRequired {
            id: call.id.clone(),
            name: effective_name.to_string(),
            arguments: effective_arguments.clone(),
        });

        match self.wait_for_approval(call, input_inbox).await {
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
                if denial == ApprovalDenial::Cancelled {
                    AuthorizationDecision::Cancel
                } else {
                    AuthorizationDecision::Deny(denial)
                }
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

        if !tool_policy_for_call(&call.name, &call.arguments).retry_timeout_once {
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
        let (effective_name, _) = effective_tool_call(&call.name, &call.arguments);
        let _ = event_tx.send(LoopEvent::ToolResult {
            id: call.id.clone(),
            output,
            is_error: result.is_error,
        });

        Content::ToolResult {
            tool_use_id: call.id.clone(),
            output: build_history_tool_result(effective_name, &result.output, result.is_error),
            is_error: result.is_error.then_some(true),
        }
    }

    async fn wait_for_approval(
        &self,
        call: &AiToolCall,
        input_inbox: &mut LoopInputInbox,
    ) -> ApprovalDecision {
        let deadline = tokio::time::Instant::now() + self.approval_timeout;

        match tokio::time::timeout_at(deadline, input_inbox.recv_tool_approval(&call.id)).await {
            Ok(ToolApprovalInput::Decision(true)) => ApprovalDecision::Approved,
            Ok(ToolApprovalInput::Decision(false)) => {
                ApprovalDecision::Denied(ApprovalDenial::UserRejected)
            }
            Ok(ToolApprovalInput::Cancelled) => ApprovalDecision::Denied(ApprovalDenial::Cancelled),
            Ok(ToolApprovalInput::Closed) => {
                ApprovalDecision::Denied(ApprovalDenial::ChannelClosed)
            }
            Err(_) => ApprovalDecision::Denied(ApprovalDenial::TimedOut),
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

    fn deferred_call(tool: &str, arguments: serde_json::Value) -> AiToolCall {
        AiToolCall {
            id: format!("call_deferred_{tool}"),
            name: "tool_search".to_string(),
            arguments: json!({
                "action": "execute",
                "tool": tool,
                "arguments": arguments,
            }),
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
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let mut input_inbox = LoopInputInbox::new(input_rx);
        input_tx
            .send(LoopInput::ToolApproval {
                tool_call_id: call.id.clone(),
                approved: true,
            })
            .unwrap();

        let decision = control.authorize(&call, &event_tx, &mut input_inbox).await;

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
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let mut input_inbox = LoopInputInbox::new(input_rx);
        input_tx
            .send(LoopInput::ToolApproval {
                tool_call_id: call.id.clone(),
                approved: true,
            })
            .unwrap();

        let decision = control.authorize(&call, &event_tx, &mut input_inbox).await;

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
    async fn deferred_write_approval_names_the_effective_target() {
        let control = ToolControl::new(PermissionMode::Supervised);
        let call = deferred_call("edit", json!({"file_path": "src/lib.rs"}));
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let mut input_inbox = LoopInputInbox::new(input_rx);
        input_tx
            .send(LoopInput::ToolApproval {
                tool_call_id: call.id.clone(),
                approved: true,
            })
            .unwrap();

        assert_eq!(
            control.authorize(&call, &event_tx, &mut input_inbox).await,
            AuthorizationDecision::Execute
        );
        assert!(matches!(
            event_rx.recv().await,
            Some(LoopEvent::ToolApprovalRequired { name, arguments, .. })
                if name == "edit" && arguments["file_path"] == "src/lib.rs"
        ));
    }

    #[test]
    fn deferred_results_use_target_specific_history_retention() {
        let control = ToolControl::new(PermissionMode::Autonomous);
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let grep = deferred_call("grep", json!({"pattern": "needle"}));
        let bash = deferred_call("bash", json!({"command": "cargo test"}));
        let result = ToolResult::success_data(json!({"output": "ok"}));

        let Content::ToolResult { output, .. } = control.publish_result(&grep, &result, &event_tx)
        else {
            panic!("expected grep result");
        };
        assert_eq!(output["tool"], "grep");
        assert_eq!(output["retention"], "summarize_after_turn");

        let Content::ToolResult { output, .. } = control.publish_result(&bash, &result, &event_tx)
        else {
            panic!("expected bash result");
        };
        assert_eq!(output["tool"], "bash");
        assert_eq!(output["retention"], "drop_after_compaction");
    }

    #[tokio::test]
    async fn authorize_stops_the_batch_when_cancelled() {
        let control = ToolControl::new(PermissionMode::Supervised);
        let call = edit_call();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let mut input_inbox = LoopInputInbox::new(input_rx);
        input_tx.send(LoopInput::Cancel).unwrap();

        let decision = control.authorize(&call, &event_tx, &mut input_inbox).await;

        assert_eq!(decision, AuthorizationDecision::Cancel);
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
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let mut input_inbox = LoopInputInbox::new(input_rx);

        let decision = control.authorize(&call, &event_tx, &mut input_inbox).await;

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
