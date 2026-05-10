//! Notification Bridge for ACP
//!
//! Provides a channel-based bridge between the Agent and the Connection,
//! allowing the Agent to send session notifications without direct access
//! to the connection.

use std::time::Duration;

use agent_client_protocol::{
    Client, Error as AcpError, PermissionOptionKind, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, Result as AcpResult,
    SelectedPermissionOutcome, SessionNotification,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Bridge that implements Client trait using channels
///
/// This allows the PromptProcessor to send session notifications
/// through a channel, which are then forwarded to the real connection
/// by the server.
pub struct NotificationBridge {
    tx: mpsc::Sender<SessionNotification>,
}

impl NotificationBridge {
    /// Create a new notification bridge
    pub fn new(tx: mpsc::Sender<SessionNotification>) -> Self {
        Self { tx }
    }
}

/// Async trait implementation for Client
///
/// The Client trait requires:
/// - request_permission (required)
/// - session_notification (required)
/// - Other methods have default implementations
///
/// # Security Note
///
/// In headless bridge mode there is no UI that can safely confirm sensitive tool
/// requests. The bridge therefore rejects permission requests by default instead
/// of silently granting write or shell access. Interactive ACP clients can still
/// approve by implementing `request_permission` on their real connection.
#[async_trait::async_trait(?Send)]
impl Client for NotificationBridge {
    async fn request_permission(
        &self,
        request: RequestPermissionRequest,
    ) -> AcpResult<RequestPermissionResponse> {
        // No UI is available on the notification bridge, so choose the safest
        // provided option. Prefer an explicit reject choice; if the caller did
        // not provide one, return Cancelled instead of granting access.
        let Some(option_id) = request
            .options
            .iter()
            .find(|opt| {
                matches!(
                    opt.kind,
                    PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                )
            })
            .map(|opt| opt.option_id.clone())
        else {
            warn!("Permission request cancelled in headless mode: no reject option provided");
            return Ok(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ));
        };

        let tool_desc = request
            .tool_call
            .fields
            .title
            .as_deref()
            .unwrap_or("unknown operation");
        info!(
            "Permission rejected for '{}' (headless mode, option: {})",
            tool_desc, option_id
        );

        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id));
        Ok(RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(&self, notification: SessionNotification) -> AcpResult<()> {
        // Try non-blocking send first to avoid stalling the processor
        match self.tx.try_send(notification) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(notification)) => {
                // Channel full - wait with timeout rather than blocking forever.
                // On slow clients (phones), the forwarder may not drain fast enough.
                match tokio::time::timeout(Duration::from_secs(10), self.tx.send(notification))
                    .await
                {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(AcpError::new(-32603, format!("Channel closed: {}", e))),
                    Err(_) => {
                        warn!("Notification channel full for 10s, dropping notification");
                        Ok(())
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(AcpError::new(-32603, "Notification channel closed"))
            }
        }
    }
}

/// Create a bounded notification channel and bridge
///
/// Uses bounded channels (capacity 1000) to prevent unbounded memory growth
/// from slow notification consumers.
///
/// Returns (bridge, receiver) tuple:
/// - bridge: implements Client, used by PromptProcessor
/// - receiver: receives notifications to forward to real connection
pub fn create_notification_channel() -> (NotificationBridge, mpsc::Receiver<SessionNotification>) {
    const CHANNEL_CAPACITY: usize = 1000;
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    (NotificationBridge::new(tx), rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::{
        ContentBlock, ContentChunk, PermissionOption, PermissionOptionId, RequestPermissionOutcome,
        SessionId, SessionUpdate, TextContent, ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
    };

    #[tokio::test]
    async fn test_bridge_sends_notifications() {
        let (bridge, mut rx) = create_notification_channel();

        let session_id = SessionId::from("test-session");
        let chunk = ContentChunk::new(ContentBlock::Text(TextContent::new("Hello")));
        let notification =
            SessionNotification::new(session_id, SessionUpdate::AgentMessageChunk(chunk));

        bridge.session_notification(notification).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert!(matches!(
            received.update,
            SessionUpdate::AgentMessageChunk(_)
        ));
    }
    #[tokio::test]
    async fn test_bridge_rejects_permission_requests_in_headless_mode() {
        let (bridge, _rx) = create_notification_channel();

        let response = bridge
            .request_permission(RequestPermissionRequest::new(
                SessionId::from("test-session"),
                ToolCallUpdate::new(
                    ToolCallId::from("write-001"),
                    ToolCallUpdateFields::new().title("Run write"),
                ),
                vec![
                    PermissionOption::new(
                        PermissionOptionId::new("allow-once"),
                        "Allow once",
                        PermissionOptionKind::AllowOnce,
                    ),
                    PermissionOption::new(
                        PermissionOptionId::new("reject-once"),
                        "Reject",
                        PermissionOptionKind::RejectOnce,
                    ),
                ],
            ))
            .await
            .unwrap();

        match response.outcome {
            RequestPermissionOutcome::Selected(selected) => {
                assert_eq!(selected.option_id.0.as_ref(), "reject-once");
            }
            RequestPermissionOutcome::Cancelled => panic!("expected explicit rejection"),
            _ => panic!("unexpected permission outcome"),
        }
    }

    #[tokio::test]
    async fn test_bridge_cancels_permission_without_reject_option() {
        let (bridge, _rx) = create_notification_channel();

        let response = bridge
            .request_permission(RequestPermissionRequest::new(
                SessionId::from("test-session"),
                ToolCallUpdate::new(
                    ToolCallId::from("write-001"),
                    ToolCallUpdateFields::new().title("Run write"),
                ),
                vec![PermissionOption::new(
                    PermissionOptionId::new("allow-once"),
                    "Allow once",
                    PermissionOptionKind::AllowOnce,
                )],
            ))
            .await
            .unwrap();

        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Cancelled
        ));
    }
}
