use async_trait::async_trait;
use krusty_mako_protocol::{
    Actor, Command, EventEnvelope, PeerIdentity, ProtocolErrorPayload, ResponsePayload,
    SubscriptionAccepted,
};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct CommandContext {
    pub request_id: String,
    pub idempotency_key: String,
    pub actor: Actor,
    pub deadline_unix_ms: i64,
    pub peer: PeerIdentity,
    pub daemon_instance_id: String,
}

pub enum HandlerReply {
    Response(ResponsePayload),
    Subscription {
        accepted: SubscriptionAccepted,
        events: mpsc::Receiver<EventEnvelope>,
    },
}

impl std::fmt::Debug for HandlerReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Response(response) => formatter.debug_tuple("Response").field(response).finish(),
            Self::Subscription { accepted, .. } => formatter
                .debug_struct("Subscription")
                .field("accepted", accepted)
                .field("events", &"<receiver>")
                .finish(),
        }
    }
}

pub type HandlerResult = Result<HandlerReply, ProtocolErrorPayload>;

#[async_trait]
pub trait CommandHandler: Send + Sync + 'static {
    /// Handle every non-foundation command. Ping, stats, and shutdown remain
    /// daemon-owned so they continue working while runtime recovery is degraded.
    async fn handle(&self, context: CommandContext, command: Command) -> HandlerResult;

    /// Runtime-specific stats merged into the daemon's stable transport stats.
    async fn runtime_stats(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

#[derive(Debug, Default)]
pub struct UnavailableCommandHandler;

#[async_trait]
impl CommandHandler for UnavailableCommandHandler {
    async fn handle(&self, _context: CommandContext, command: Command) -> HandlerResult {
        Err(ProtocolErrorPayload::new(
            "runtime_unavailable",
            format!(
                "Mako runtime handler is not installed for {}",
                command.name()
            ),
            true,
        ))
    }
}
