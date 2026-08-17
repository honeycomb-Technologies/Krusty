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
    CreateSchedule(CreateScheduleCommand),
    ReplaceSchedule(ReplaceScheduleCommand),
    SetScheduleStatus(SetScheduleStatusCommand),
    StartSession(SessionCommand),
    ScheduleSession(ScheduleCommand),
    PauseSession(SessionCommand),
    ResumeSession(SessionCommand),
    CancelSession(SessionCommand),
    DeleteSession(SessionCommand),
    SendMessage(MessageCommand),
    GroupMessage(GroupMessageCommand),
    GroupStop(GroupStopCommand),
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
            Self::CreateSchedule(_) => "create_schedule",
            Self::ReplaceSchedule(_) => "replace_schedule",
            Self::SetScheduleStatus(_) => "set_schedule_status",
            Self::StartSession(_) => "start_session",
            Self::ScheduleSession(_) => "schedule_session",
            Self::PauseSession(_) => "pause_session",
            Self::ResumeSession(_) => "resume_session",
            Self::CancelSession(_) => "cancel_session",
            Self::DeleteSession(_) => "delete_session",
            Self::SendMessage(_) => "send_message",
            Self::GroupMessage(_) => "group_message",
            Self::GroupStop(_) => "group_stop",
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

    /// Minimum negotiated minor needed to transmit this command without
    /// silently losing fields unknown to an older peer.
    pub fn minimum_protocol_minor(&self) -> u16 {
        // Group-room commands did not exist before v1.3; an older daemon
        // would reject the unknown variant with an opaque decode error, so
        // fail closed with a clear version message instead.
        if matches!(self, Self::GroupMessage(_) | Self::GroupStop(_)) {
            return crate::GROUP_MESSAGING_PROTOCOL_MINOR;
        }
        let carries_exact_model_identity = match self {
            Self::Dispatch(command) => {
                command.model_key.is_some() || command.model_catalog_revision.is_some()
            }
            Self::CreateSchedule(command) => {
                command.definition.model_key.is_some()
                    || command.definition.model_catalog_revision.is_some()
            }
            Self::ReplaceSchedule(command) => {
                command.definition.model_key.is_some()
                    || command.definition.model_catalog_revision.is_some()
            }
            _ => false,
        };
        if carries_exact_model_identity {
            crate::MODEL_IDENTITY_PROTOCOL_MINOR
        } else {
            0
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

/// Provider-aware executable model identity. String-valued transport fields
/// keep the daemon protocol independent from `mitsuro-core` while preserving
/// the exact serialized shape of the core `ModelKey`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ModelKey {
    pub provider: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_scope: Option<String>,
    pub api_format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DispatchCommand {
    pub task: String,
    pub working_dir: String,
    pub project_dir: Option<String>,
    /// Legacy compatibility mirror of `model_key.model_id`.
    pub model: Option<String>,
    #[serde(default)]
    pub model_key: Option<ModelKey>,
    #[serde(default)]
    pub model_catalog_revision: Option<String>,
    pub start_at_unix_ms: Option<i64>,
    pub priority: Option<String>,
    pub crew_slug: Option<String>,
}

/// Complete, versioned schedule definition accepted by the daemon. Complex
/// policy fields remain JSON at the transport boundary and are deserialized
/// into the core's strongly typed recurrence/policy models before commit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleDefinition {
    pub title: String,
    pub summary: String,
    pub objective: String,
    pub recurrence: serde_json::Value,
    pub timezone: String,
    pub dst_policy: serde_json::Value,
    pub priority: i32,
    pub project_dir: Option<String>,
    /// Legacy compatibility mirror of `model_key.model_id`.
    pub model: Option<String>,
    #[serde(default)]
    pub model_key: Option<ModelKey>,
    #[serde(default)]
    pub model_catalog_revision: Option<String>,
    pub crew_slug: Option<String>,
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    pub misfire: serde_json::Value,
    pub overlap_policy: String,
    pub retry: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateScheduleCommand {
    pub session_id: String,
    pub definition: ScheduleDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplaceScheduleCommand {
    pub session_id: String,
    pub schedule_id: String,
    pub expected_revision: u64,
    pub definition: ScheduleDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetScheduleStatusCommand {
    pub session_id: String,
    pub schedule_id: String,
    pub expected_revision: u64,
    pub status: String,
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

/// One user message into a group room. The daemon appends it durably,
/// resolves targets (explicit override or server-side mention parsing), and
/// fans out member runs according to the group's execution mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupMessageCommand {
    pub group_id: String,
    pub message: String,
    /// Explicit target Worker slugs; `None` derives targets from mentions.
    #[serde(default)]
    pub mentions_override: Option<Vec<String>>,
}

/// Cancel the in-flight member runs of a group's active turn and mark the
/// turn cancelled. Idle groups acknowledge without effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupStopCommand {
    pub group_id: String,
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
    pub run_id: String,
    pub tool_call_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserResponseCommand {
    pub session_id: String,
    pub run_id: String,
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
    Schedule(ScheduleResponse),
    Session(SessionResponse),
    GroupTurn(GroupTurnResponse),
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
    #[serde(default, deserialize_with = "deserialize_daemon_runtime_stats")]
    pub runtime: DaemonRuntimeStats,
}

/// Stable scheduler counters and readiness signals exported by the daemon.
///
/// This deliberately remains a concrete protocol type instead of an open JSON
/// object so independently shipped clients cannot silently drift onto keys the
/// daemon never emits. `serde(default)` keeps additive evolution and older
/// partial v1 payloads readable while unknown future fields remain harmless.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DaemonRuntimeStats {
    pub active_controllers: usize,
    pub active_runs: usize,
    pub queued_runs: usize,
    pub recovery_required: usize,
    pub pump_alive: bool,
    pub scheduler_ready: bool,
}

fn deserialize_daemon_runtime_stats<'de, D>(deserializer: D) -> Result<DaemonRuntimeStats, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<DaemonRuntimeStats>::deserialize(deserializer).map(Option::unwrap_or_default)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleResponse {
    pub schedule_id: String,
    pub revision: u64,
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

/// Durable acceptance of one group turn: the appended trigger message plus
/// the dispatch plan the daemon committed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupTurnResponse {
    pub group_id: String,
    pub turn_id: String,
    pub message_id: String,
    pub message_seq: i64,
    pub status: String,
    /// Worker ids selected as this turn's targets, in dispatch order.
    pub target_worker_ids: Vec<String>,
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
    pub event: HiveEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum HiveEvent {
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
    Request(Box<RequestEnvelope>),
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
    fn older_minor_remains_compatible_for_foundation_commands() {
        assert_eq!(
            ProtocolVersion {
                major: PROTOCOL_MAJOR,
                minor: 0
            }
            .negotiate()
            .unwrap(),
            ProtocolVersion {
                major: PROTOCOL_MAJOR,
                minor: 0
            }
        );
    }

    #[test]
    fn schedule_commands_round_trip_on_current_minor() {
        let command = Command::CreateSchedule(CreateScheduleCommand {
            session_id: "session-1".into(),
            definition: ScheduleDefinition {
                title: "Daily check".into(),
                summary: "Check health".into(),
                objective: "Run health checks".into(),
                recurrence: serde_json::json!({
                    "kind": "daily",
                    "start_date": "2026-07-17",
                    "time": "09:30:00"
                }),
                timezone: "America/Los_Angeles".into(),
                dst_policy: serde_json::json!({
                    "gap": "shift_forward",
                    "fold": "first"
                }),
                priority: 0,
                project_dir: Some("/work/repo".into()),
                model: None,
                model_key: None,
                model_catalog_revision: None,
                crew_slug: None,
                worker_id: None,
                group_id: None,
                misfire: serde_json::json!({
                    "policy": "fire_once",
                    "grace_secs": 300,
                    "catch_up_limit": 3
                }),
                overlap_policy: "queue_one".into(),
                retry: serde_json::json!({
                    "max_attempts": 5,
                    "base_delay_secs": 15,
                    "max_delay_secs": 900,
                    "jitter": "full"
                }),
            },
        });
        let encoded = serde_json::to_vec(&command).unwrap();
        let decoded: Command = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, command);
        assert!(encoded.len() < crate::MAX_FRAME_BYTES);
    }

    #[test]
    fn model_identity_fields_are_additive_and_legacy_compatible() {
        let legacy: DispatchCommand = serde_json::from_value(serde_json::json!({
            "task": "inspect",
            "working_dir": "/work",
            "project_dir": null,
            "model": "grok-code-fast-1",
            "start_at_unix_ms": null,
            "priority": null,
            "crew_slug": null
        }))
        .unwrap();
        assert!(legacy.model_key.is_none());
        assert!(legacy.model_catalog_revision.is_none());

        let exact = DispatchCommand {
            task: "inspect".into(),
            working_dir: "/work".into(),
            project_dir: None,
            model: Some("grok-code-fast-1".into()),
            model_key: Some(ModelKey {
                provider: "grok".into(),
                model_id: "grok-code-fast-1".into(),
                auth_scope: Some("oauth".into()),
                api_format: "open_ai_responses".into(),
            }),
            model_catalog_revision: Some("catalog-42".into()),
            start_at_unix_ms: None,
            priority: None,
            crew_slug: None,
        };
        let round_trip =
            serde_json::from_value::<DispatchCommand>(serde_json::to_value(&exact).unwrap())
                .unwrap();
        assert_eq!(round_trip, exact);
        assert_eq!(Command::Dispatch(exact).minimum_protocol_minor(), 2);
    }

    #[test]
    fn group_commands_require_the_group_messaging_minor() {
        let message = Command::GroupMessage(GroupMessageCommand {
            group_id: "group-1".into(),
            message: "@researcher take a look".into(),
            mentions_override: None,
        });
        let stop = Command::GroupStop(GroupStopCommand {
            group_id: "group-1".into(),
        });
        assert_eq!(
            message.minimum_protocol_minor(),
            crate::GROUP_MESSAGING_PROTOCOL_MINOR
        );
        assert_eq!(
            stop.minimum_protocol_minor(),
            crate::GROUP_MESSAGING_PROTOCOL_MINOR
        );
        // Pre-group commands stay transmittable to older daemons.
        assert_eq!(Command::Ping.minimum_protocol_minor(), 0);

        let encoded = serde_json::to_vec(&message).unwrap();
        let decoded: Command = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, message);

        // mentions_override stays additive for readers of the v1.3 shape.
        let legacy_shape: GroupMessageCommand = serde_json::from_value(serde_json::json!({
            "group_id": "group-1",
            "message": "hello"
        }))
        .unwrap();
        assert!(legacy_shape.mentions_override.is_none());
    }

    #[test]
    fn group_turn_response_round_trips() {
        let payload = ResponsePayload::GroupTurn(GroupTurnResponse {
            group_id: "group-1".into(),
            turn_id: "turn-1".into(),
            message_id: "message-1".into(),
            message_seq: 7,
            status: "running".into(),
            target_worker_ids: vec!["worker-a".into(), "worker-b".into()],
        });
        let encoded = serde_json::to_value(&payload).unwrap();
        assert_eq!(encoded["response"], "group_turn");
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            payload
        );
    }

    #[test]
    fn daemon_runtime_stats_round_trip_without_key_drift() {
        let stats = DaemonRuntimeStats {
            active_controllers: 7,
            active_runs: 5,
            queued_runs: 3,
            recovery_required: 2,
            pump_alive: true,
            scheduler_ready: true,
        };

        let encoded = serde_json::to_value(stats).unwrap();
        assert_eq!(encoded["active_controllers"], 7);
        assert_eq!(encoded["active_runs"], 5);
        assert_eq!(encoded["queued_runs"], 3);
        assert_eq!(encoded["recovery_required"], 2);
        assert_eq!(
            serde_json::from_value::<DaemonRuntimeStats>(encoded).unwrap(),
            stats
        );
    }

    #[test]
    fn daemon_runtime_stats_accept_older_partial_payloads() {
        let stats: DaemonRuntimeStats = serde_json::from_value(serde_json::json!({
            "pump_alive": true,
            "scheduler_ready": true,
            "future_additive_field": 42
        }))
        .unwrap();

        assert!(stats.pump_alive);
        assert!(stats.scheduler_ready);
        assert_eq!(stats.active_controllers, 0);
        assert_eq!(stats.active_runs, 0);
        assert_eq!(stats.queued_runs, 0);
        assert_eq!(stats.recovery_required, 0);
    }

    #[test]
    fn daemon_stats_accept_legacy_null_runtime() {
        let stats: DaemonStats = serde_json::from_value(serde_json::json!({
            "instance_id": "legacy-daemon",
            "daemon_version": "0.1.0",
            "protocol": { "major": 1, "minor": 0 },
            "uptime_secs": 1,
            "active_connections": 1,
            "handled_requests": 1,
            "runtime": null
        }))
        .unwrap();

        assert_eq!(stats.runtime, DaemonRuntimeStats::default());
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
