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
use tracing::info;

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
/// In headless/ACP mode, permissions are auto-approved because there's no UI
/// to prompt the user. This is expected behavior for background agent execution.
/// The editor (Zed, etc.) is responsible for user consent before spawning the agent.
#[async_trait::async_trait(?Send)]
impl Client for NotificationBridge {
    async fn request_permission(
        &self,
        request: RequestPermissionRequest,
    ) -> AcpResult<RequestPermissionResponse> {
        // In headless mode, auto-approve permissions since there's no UI to prompt.
        // The editor is responsible for user consent before spawning the agent.
        let option_id = request
            .options
            .first()
            .map(|opt| opt.option_id.clone())
            .unwrap_or_else(|| PermissionOptionId::from("allow"));

        // Log what permission is being granted
        let tool_desc = request
            .tool_call
            .fields
            .title
            .as_deref()
            .unwrap_or("unknown operation");
        info!(
            "Permission auto-granted for '{}' (headless mode, option: {})",
            tool_desc, option_id
        );

        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id));
        Ok(RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(&self, notification: SessionNotification) -> AcpResult<()> {
        self.tx
            .send(notification)
            .await
            .map_err(|e| AcpError::new(-32603, format!("Channel send error: {}", e)))
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
