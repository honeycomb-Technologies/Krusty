//! Notification Bridge for ACP
//!
//! Provides a channel-based bridge between the Agent and the Connection,
//! allowing the Agent to send session notifications without direct access
//! to the connection.

use agent_client_protocol::{
    Client, Error as AcpError, PermissionOptionId, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, Result as AcpResult,
    SelectedPermissionOutcome, SessionNotification,
};
use tokio::sync::mpsc;
use tracing::warn;

/// Bridge that implements Client trait using channels
///
/// This allows the PromptProcessor to send session notifications
/// through a channel, which are then forwarded to the real connection
/// by the server.
pub struct NotificationBridge {
    tx: mpsc::UnboundedSender<SessionNotification>,
}

impl NotificationBridge {
    /// Create a new notification bridge
    pub fn new(tx: mpsc::UnboundedSender<SessionNotification>) -> Self {
        Self { tx }
    }
}

/// Async trait implementation for Client
///
/// The Client trait requires:
/// - request_permission (required)
/// - session_notification (required)
/// - Other methods have default implementations
#[async_trait::async_trait(?Send)]
impl Client for NotificationBridge {
    async fn request_permission(
        &self,
        request: RequestPermissionRequest,
    ) -> AcpResult<RequestPermissionResponse> {
        // Auto-approve permissions by selecting the first available option
        // In a full implementation, this would delegate to the editor
        warn!("Permission request auto-approved (editor delegation not implemented)");

        // Get the first option from the request, or use a default "allow" option
        let option_id = request
            .options
            .first()
            .map(|opt| opt.option_id.clone())
            .unwrap_or_else(|| PermissionOptionId::from("allow"));

        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id));
        Ok(RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(&self, notification: SessionNotification) -> AcpResult<()> {
        self.tx
            .send(notification)
            .map_err(|e| AcpError::new(-32603, format!("Channel send error: {}", e)))
    }
}

/// Create a notification channel and bridge
///
/// Returns (bridge, receiver) tuple:
/// - bridge: implements Client, used by PromptProcessor
/// - receiver: receives notifications to forward to real connection
pub fn create_notification_channel() -> (
    NotificationBridge,
    mpsc::UnboundedReceiver<SessionNotification>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    (NotificationBridge::new(tx), rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::{
        ContentBlock, ContentChunk, SessionId, SessionUpdate, TextContent,
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
}
