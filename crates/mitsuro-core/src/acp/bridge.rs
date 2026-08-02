//! Notification Bridge for ACP
//!
//! Provides a channel-based bridge between the Agent and the Connection,
//! allowing the Agent to send session notifications without direct access
//! to the connection.

use std::time::Duration;

use agent_client_protocol::{
    Client, Error as AcpError, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, Result as AcpResult, SessionNotification,
};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

/// Messages that must be delivered over the live ACP connection.
///
/// Notifications are fire-and-forget. Permission requests carry a one-shot
/// response channel so the agent loop can wait for the editor's decision
/// without owning the non-`Send` ACP connection itself.
pub enum AcpOutbound {
    Notification(SessionNotification),
    Permission {
        request: RequestPermissionRequest,
        response_tx: oneshot::Sender<Result<RequestPermissionResponse, String>>,
    },
}

/// Bridge that implements Client trait using channels
///
/// This allows the PromptProcessor to send session notifications
/// through a channel, which are then forwarded to the real connection
/// by the server.
pub struct NotificationBridge {
    tx: mpsc::Sender<AcpOutbound>,
}

impl NotificationBridge {
    /// Create a new notification bridge
    pub fn new(tx: mpsc::Sender<AcpOutbound>) -> Self {
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
#[async_trait::async_trait(?Send)]
impl Client for NotificationBridge {
    async fn request_permission(
        &self,
        request: RequestPermissionRequest,
    ) -> AcpResult<RequestPermissionResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(AcpOutbound::Permission {
                request,
                response_tx,
            })
            .await
            .map_err(|_| AcpError::new(-32603, "ACP connection closed"))?;

        match tokio::time::timeout(Duration::from_secs(300), response_rx).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(error))) => Err(AcpError::new(-32603, error)),
            Ok(Err(_)) => Err(AcpError::new(
                -32603,
                "ACP permission response channel closed",
            )),
            Err(_) => {
                warn!("ACP permission request timed out after 5 minutes");
                Ok(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ))
            }
        }
    }

    async fn session_notification(&self, notification: SessionNotification) -> AcpResult<()> {
        // Try non-blocking send first to avoid stalling the processor
        match self.tx.try_send(AcpOutbound::Notification(notification)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(outbound)) => {
                // Channel full - wait with timeout rather than blocking forever.
                // On slow clients (phones), the forwarder may not drain fast enough.
                match tokio::time::timeout(Duration::from_secs(10), self.tx.send(outbound)).await {
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
pub fn create_notification_channel() -> (NotificationBridge, mpsc::Receiver<AcpOutbound>) {
    const CHANNEL_CAPACITY: usize = 1000;
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    (NotificationBridge::new(tx), rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::{
        ContentBlock, ContentChunk, PermissionOption, PermissionOptionId, PermissionOptionKind,
        RequestPermissionOutcome, SelectedPermissionOutcome, SessionId, SessionUpdate, TextContent,
        ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
    };

    #[tokio::test]
    async fn test_bridge_sends_notifications() {
        let (bridge, mut rx) = create_notification_channel();

        let session_id = SessionId::from("test-session");
        let chunk = ContentChunk::new(ContentBlock::Text(TextContent::new("Hello")));
        let notification =
            SessionNotification::new(session_id, SessionUpdate::AgentMessageChunk(chunk));

        bridge.session_notification(notification).await.unwrap();

        let AcpOutbound::Notification(received) = rx.recv().await.unwrap() else {
            panic!("expected notification");
        };
        assert!(matches!(
            received.update,
            SessionUpdate::AgentMessageChunk(_)
        ));
    }
    #[tokio::test]
    async fn test_bridge_relays_permission_response() {
        let (bridge, mut rx) = create_notification_channel();
        let request = RequestPermissionRequest::new(
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
        );

        let responder = async {
            let AcpOutbound::Permission {
                request,
                response_tx,
            } = rx.recv().await.expect("permission request")
            else {
                panic!("expected permission request");
            };
            assert_eq!(request.session_id.to_string(), "test-session");
            let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                PermissionOptionId::new("allow-once"),
            ));
            response_tx
                .send(Ok(RequestPermissionResponse::new(outcome)))
                .expect("bridge should await response");
        };

        let (response, ()) = tokio::join!(bridge.request_permission(request), responder);
        let response = response.unwrap();

        match response.outcome {
            RequestPermissionOutcome::Selected(selected) => {
                assert_eq!(selected.option_id.0.as_ref(), "allow-once");
            }
            RequestPermissionOutcome::Cancelled => panic!("expected selected response"),
            _ => panic!("unexpected permission outcome"),
        }
    }

    #[tokio::test]
    async fn test_bridge_fails_permission_when_connection_is_closed() {
        let (bridge, rx) = create_notification_channel();
        drop(rx);

        let error = bridge
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
            .expect_err("closed connection must not grant permission");

        assert!(error.to_string().contains("closed"));
    }
}
