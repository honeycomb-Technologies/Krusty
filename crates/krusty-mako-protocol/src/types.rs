use serde::{Deserialize, Serialize};

use crate::error::ProtocolViolation;
use crate::{unix_time_millis, PROTOCOL_MAJOR, PROTOCOL_MINOR};

const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_ACTOR_ID_BYTES: usize = 256;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const CURRENT: Self = Self {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
    };

    // With v1.0 this is currently equivalent to selecting zero, but retaining
    // the real negotiation rule here keeps future minor-version bumps safe.
    #[allow(clippy::unnecessary_min_or_max)]
    pub fn negotiate(self) -> Result<Self, ProtocolViolation> {
        if self.major != PROTOCOL_MAJOR {
            return Err(ProtocolViolation::VersionMismatch {
                expected_major: PROTOCOL_MAJOR,
                expected_minor: PROTOCOL_MINOR,
                actual_major: self.major,
                actual_minor: self.minor,
            });
        }

        Ok(Self {
            major: PROTOCOL_MAJOR,
            minor: self.minor.min(PROTOCOL_MINOR),
        })
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    pub version: ProtocolVersion,
    pub client_id: String,
    /// Lowercase hexadecimal 32-byte random value.
    pub nonce: String,
    pub issued_at_unix_ms: i64,
    /// Lowercase hexadecimal HMAC-SHA256 authentication code.
    pub mac: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloAck {
    pub version: ProtocolVersion,
    pub instance_id: String,
    pub daemon_version: String,
    pub client_nonce: String,
    pub server_nonce: String,
    pub server_time_unix_ms: i64,
    /// HMAC-SHA256 over the negotiated acknowledgement fields.
    pub mac: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Actor {
    /// `None` is the single-tenant local identity. Multi-tenant callers must
    /// provide the authenticated user id and the daemon must re-check storage.
    pub user_id: Option<String>,
    pub client_kind: String,
}

impl Actor {
    pub fn local(client_kind: impl Into<String>) -> Self {
        Self {
            user_id: None,
            client_kind: client_kind.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestEnvelope {
    pub version: ProtocolVersion,
    pub request_id: String,
    pub actor: Actor,
    pub deadline_unix_ms: i64,
    pub idempotency_key: String,
    pub command: Command,
}

impl RequestEnvelope {
    pub fn new(actor: Actor, command: Command, timeout_ms: u64) -> Self {
        let request_id = uuid::Uuid::new_v4().to_string();
        Self {
            version: ProtocolVersion::CURRENT,
            idempotency_key: request_id.clone(),
            request_id,
            actor,
            deadline_unix_ms: unix_time_millis()
                .saturating_add(i64::try_from(timeout_ms).unwrap_or(i64::MAX)),
            command,
        }
    }

    pub fn validate(&self, now_unix_ms: i64) -> Result<(), ProtocolViolation> {
        self.version.negotiate()?;
        if self.request_id.is_empty() || self.request_id.len() > MAX_REQUEST_ID_BYTES {
            return Err(ProtocolViolation::InvalidRequestId);
        }
        if self.actor.client_kind.trim().is_empty()
            || self
                .actor
                .user_id
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > MAX_ACTOR_ID_BYTES)
            || self.actor.client_kind.len() > MAX_ACTOR_ID_BYTES
        {
            return Err(ProtocolViolation::InvalidActor);
        }
        if self.idempotency_key.is_empty() || self.idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        {
            return Err(ProtocolViolation::InvalidIdempotencyKey);
        }
        if self.deadline_unix_ms <= now_unix_ms {
            return Err(ProtocolViolation::DeadlineExpired);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum Command {
    Ping,
    Stats,
    Shutdown(ShutdownCommand),
    Dispatch(DispatchCommand),
    StartSession(SessionCommand),
    ScheduleSession(ScheduleCommand),
    PauseSession(SessionCommand),
    ResumeSession(SessionCommand),
    CancelSession(SessionCommand),
    DeleteSession(SessionCommand),
    SendMessage(MessageCommand),
    Steer(SteerCommand),
    ToolApproval(ToolApprovalCommand),
    UserResponse(UserResponseCommand),
    SetPriority(SetPriorityCommand),
    SetCrew(SetCrewCommand),
    Recover(RecoverCommand),
    Subscribe(SubscribeCommand),
    /// Forward-compatible escape hatch for independently shipped runtime work.
    /// Stable features should graduate to explicit enum variants.
    Extension(ExtensionCommand),
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Stats => "stats",
            Self::Shutdown(_) => "shutdown",
            Self::Dispatch(_) => "dispatch",
            Self::StartSession(_) => "start_session",
            Self::ScheduleSession(_) => "schedule_session",
            Self::PauseSession(_) => "pause_session",
            Self::ResumeSession(_) => "resume_session",
            Self::CancelSession(_) => "cancel_session",
            Self::DeleteSession(_) => "delete_session",
            Self::SendMessage(_) => "send_message",
            Self::Steer(_) => "steer",
            Self::ToolApproval(_) => "tool_approval",
            Self::UserResponse(_) => "user_response",
            Self::SetPriority(_) => "set_priority",
            Self::SetCrew(_) => "set_crew",
            Self::Recover(_) => "recover",
            Self::Subscribe(_) => "subscribe",
            Self::Extension(_) => "extension",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ShutdownCommand {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCommand {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DispatchCommand {
    pub task: String,
    pub working_dir: String,
    pub project_dir: Option<String>,
    pub model: Option<String>,
    pub start_at_unix_ms: Option<i64>,
    pub priority: Option<String>,
    pub crew_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleCommand {
    pub session_id: String,
    pub wake_at_unix_ms: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageCommand {
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SteerCommand {
    pub session_id: String,
    pub pending_id: Option<String>,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolApprovalCommand {
    pub session_id: String,
    pub tool_call_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserResponseCommand {
    pub session_id: String,
    pub tool_call_id: String,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetPriorityCommand {
    pub session_id: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetCrewCommand {
    pub session_id: String,
    pub crew_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RecoverCommand {
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscribeCommand {
    pub session_id: String,
    pub after_sequence: Option<i64>,
    pub replay_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionCommand {
    pub name: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponseEnvelope {
    pub version: ProtocolVersion,
    pub request_id: String,
    pub outcome: ResponseOutcome,
}

impl ResponseEnvelope {
    pub fn success(request_id: impl Into<String>, payload: ResponsePayload) -> Self {
        Self {
            version: ProtocolVersion::CURRENT,
            request_id: request_id.into(),
            outcome: ResponseOutcome::Success { payload },
        }
    }

    pub fn failure(request_id: impl Into<String>, error: ProtocolErrorPayload) -> Self {
        Self {
            version: ProtocolVersion::CURRENT,
            request_id: request_id.into(),
            outcome: ResponseOutcome::Failure { error },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseOutcome {
    Success { payload: ResponsePayload },
    Failure { error: ProtocolErrorPayload },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "response", content = "data", rename_all = "snake_case")]
pub enum ResponsePayload {
    Pong(PongResponse),
    Stats(DaemonStats),
    Ack(AckResponse),
    Dispatch(DispatchResponse),
    Session(SessionResponse),
    Recover(RecoverResponse),
    SubscriptionAccepted(SubscriptionAccepted),
    Extension(ExtensionResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PongResponse {
    pub instance_id: String,
    pub daemon_version: String,
    pub uptime_secs: u64,
    pub server_time_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DaemonStats {
    pub instance_id: String,
    pub daemon_version: String,
    pub protocol: ProtocolVersion,
    pub uptime_secs: u64,
    pub active_connections: usize,
    pub handled_requests: u64,
    pub runtime: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AckResponse {
    pub accepted: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DispatchResponse {
    pub session_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionResponse {
    pub session_id: String,
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoverResponse {
    pub recovered_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriptionAccepted {
    pub session_id: String,
    pub high_water_sequence: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionResponse {
    pub name: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolErrorPayload {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ProtocolErrorPayload {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    pub version: ProtocolVersion,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub sequence: Option<i64>,
    pub emitted_at_unix_ms: i64,
    pub event: MakoEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum MakoEvent {
    Runtime(RuntimeEvent),
    StateChanged(StateChangedEvent),
    ReplayGap(ReplayGapEvent),
    Lagged(LaggedEvent),
    DaemonShuttingDown { reason: Option<String> },
    Extension(ExtensionEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeEvent {
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateChangedEvent {
    pub previous: Option<String>,
    pub current: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayGapEvent {
    pub requested_after: i64,
    pub earliest_available: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaggedEvent {
    pub skipped: u64,
    pub resume_after_sequence: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionEvent {
    pub name: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "frame", content = "body", rename_all = "snake_case")]
pub enum ClientFrame {
    Hello(Hello),
    Request(RequestEnvelope),
}

impl ClientFrame {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::Request(_) => "request",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "frame", content = "body", rename_all = "snake_case")]
pub enum ServerFrame {
    HelloAck(HelloAck),
    Response(ResponseEnvelope),
    Event(EventEnvelope),
    Error(ProtocolErrorPayload),
}

impl ServerFrame {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::HelloAck(_) => "hello_ack",
            Self::Response(_) => "response",
            Self::Event(_) => "event",
            Self::Error(_) => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_version_mismatch_is_rejected() {
        let error = ProtocolVersion {
            major: PROTOCOL_MAJOR + 1,
            minor: 0,
        }
        .negotiate()
        .expect_err("major mismatch must fail");
        assert!(matches!(error, ProtocolViolation::VersionMismatch { .. }));
    }

    #[test]
    fn newer_minor_negotiates_to_supported_minor() {
        assert_eq!(
            ProtocolVersion {
                major: PROTOCOL_MAJOR,
                minor: PROTOCOL_MINOR + 5,
            }
            .negotiate()
            .unwrap(),
            ProtocolVersion::CURRENT
        );
    }

    #[test]
    fn expired_request_is_rejected() {
        let mut request = RequestEnvelope::new(Actor::local("test"), Command::Ping, 10);
        request.deadline_unix_ms = 5;
        assert_eq!(request.validate(5), Err(ProtocolViolation::DeadlineExpired));
    }

    #[test]
    fn empty_actor_kind_is_rejected() {
        let request = RequestEnvelope::new(Actor::local(""), Command::Ping, 10);
        assert_eq!(
            request.validate(unix_time_millis()),
            Err(ProtocolViolation::InvalidActor)
        );
    }
}
