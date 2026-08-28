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
    WorkerSendMessage(MessageCommand),
    GroupMessage(GroupMessageCommand),
    GroupStop(GroupStopCommand),
    GroupArchive(GroupArchiveCommand),
    CreateWorkerIntroduction(CreateWorkerIntroductionCommand),
    RetryWorkerIntroduction(WorkerIntroductionCommand),
    SkipWorkerIntroduction(WorkerIntroductionCommand),
    ConfirmWorkerIntroduction(ConfirmWorkerIntroductionCommand),
    ReturnWorkerIntroductionToContext(ReturnWorkerIntroductionToContextCommand),
    UpdateWorker(UpdateWorkerCommand),
    SetWorkerStatus(SetWorkerStatusCommand),
    GrantWorkerGovernorRecovery(GrantWorkerGovernorRecoveryCommand),
    ActivateOrResumeWorkerWorkflow(ActivateOrResumeWorkerWorkflowCommand),
    PauseWorkerWorkflow(WorkerWorkflowLifecycleCommand),
    CancelWorkerWorkflow(WorkerWorkflowLifecycleCommand),
    SetWorkerWorkspace(SetWorkerWorkspaceCommand),
    Steer(SteerCommand),
    WorkerSteer(SteerCommand),
    ToolApproval(ToolApprovalCommand),
    UserResponse(UserResponseCommand),
    WorkerUserResponse(UserResponseCommand),
    StopWorkerConversation(SessionCommand),
    ResolveWorkerGoalAcceptance(ResolveWorkerGoalAcceptanceCommand),
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
            Self::WorkerSendMessage(_) => "worker_send_message",
            Self::GroupMessage(_) => "group_message",
            Self::GroupStop(_) => "group_stop",
            Self::GroupArchive(_) => "group_archive",
            Self::CreateWorkerIntroduction(_) => "create_worker_introduction",
            Self::RetryWorkerIntroduction(_) => "retry_worker_introduction",
            Self::SkipWorkerIntroduction(_) => "skip_worker_introduction",
            Self::ConfirmWorkerIntroduction(_) => "confirm_worker_introduction",
            Self::ReturnWorkerIntroductionToContext(_) => "return_worker_introduction_to_context",
            Self::UpdateWorker(_) => "update_worker",
            Self::SetWorkerStatus(_) => "set_worker_status",
            Self::GrantWorkerGovernorRecovery(_) => "grant_worker_governor_recovery",
            Self::ActivateOrResumeWorkerWorkflow(_) => "activate_or_resume_worker_workflow",
            Self::PauseWorkerWorkflow(_) => "pause_worker_workflow",
            Self::CancelWorkerWorkflow(_) => "cancel_worker_workflow",
            Self::SetWorkerWorkspace(_) => "set_worker_workspace",
            Self::Steer(_) => "steer",
            Self::WorkerSteer(_) => "worker_steer",
            Self::ToolApproval(_) => "tool_approval",
            Self::UserResponse(_) => "user_response",
            Self::WorkerUserResponse(_) => "worker_user_response",
            Self::StopWorkerConversation(_) => "stop_worker_conversation",
            Self::ResolveWorkerGoalAcceptance(_) => "resolve_worker_goal_acceptance",
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
        if matches!(self, Self::GroupArchive(_)) {
            return crate::GROUP_ARCHIVE_PROTOCOL_MINOR;
        }
        if matches!(self, Self::GroupMessage(_) | Self::GroupStop(_)) {
            return crate::GROUP_MESSAGING_PROTOCOL_MINOR;
        }
        if matches!(
            self,
            Self::ConfirmWorkerIntroduction(_) | Self::ReturnWorkerIntroductionToContext(_)
        ) {
            return crate::WORKER_INTRODUCTION_REVIEW_PROTOCOL_MINOR;
        }
        if matches!(self, Self::UpdateWorker(_) | Self::SetWorkerStatus(_)) {
            return crate::WORKER_LIFECYCLE_PROTOCOL_MINOR;
        }
        if matches!(self, Self::GrantWorkerGovernorRecovery(_)) {
            return crate::WORKER_GOVERNOR_RECOVERY_PROTOCOL_MINOR;
        }
        if matches!(
            self,
            Self::ActivateOrResumeWorkerWorkflow(_)
                | Self::PauseWorkerWorkflow(_)
                | Self::CancelWorkerWorkflow(_)
                | Self::SetWorkerWorkspace(_)
        ) {
            return crate::WORKER_WORKFLOW_PROTOCOL_MINOR;
        }
        if matches!(
            self,
            Self::WorkerSendMessage(_) | Self::WorkerSteer(_) | Self::WorkerUserResponse(_)
        ) {
            return crate::WORKER_CONVERSATION_PROTOCOL_MINOR;
        }
        if matches!(self, Self::ResolveWorkerGoalAcceptance(_)) {
            return crate::WORKER_GOAL_ACCEPTANCE_PROTOCOL_MINOR;
        }
        if matches!(self, Self::StopWorkerConversation(_)) {
            return crate::WORKER_CONVERSATION_STOP_PROTOCOL_MINOR;
        }
        if matches!(
            self,
            Self::CreateWorkerIntroduction(_)
                | Self::RetryWorkerIntroduction(_)
                | Self::SkipWorkerIntroduction(_)
        ) {
            return crate::WORKER_INTRODUCTION_PROTOCOL_MINOR;
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

/// Atomically cancel the active group turn and archive the group. The
/// timeline and member Workers remain durable and readable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupArchiveCommand {
    pub group_id: String,
}

/// Atomically create one durable Worker identity, its private DM, and the
/// tool-free run that lets the Worker initiate the relationship. The command
/// deliberately carries an exact model identity: an Introduction must never
/// drift onto a different provider or silently inherit a mutable default.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateWorkerIntroductionCommand {
    pub slug: String,
    pub display_name: String,
    pub avatar_color: Option<String>,
    pub model: String,
    pub model_key: ModelKey,
    #[serde(default)]
    pub model_catalog_revision: Option<String>,
    pub permission_mode: String,
    pub autonomy: String,
    pub heartbeat_interval_secs: Option<u32>,
    pub identity: Option<String>,
    pub soul: Option<String>,
}

/// Select one owned Worker for an explicit Introduction lifecycle action.
/// Retry and skip remain separate command variants so their idempotency
/// receipts and audit events cannot be confused.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerIntroductionCommand {
    pub worker_id: String,
}

/// One exact proposal fact selected by the user. The statement is repeated so
/// core can prove the client did not edit provider-reviewed text while a
/// confirmation request was in flight.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionSelectedFact {
    pub fact_id: String,
    pub final_statement: String,
}

/// Confirm an exact revision of the currently review-ready proposal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfirmWorkerIntroductionCommand {
    pub worker_id: String,
    pub proposal_id: String,
    pub proposal_revision: u32,
    pub selected_facts: Vec<WorkerIntroductionSelectedFact>,
}

/// Deliberately limited return decisions. Confirmation has a separate command
/// so a transport decode can never reinterpret selected facts as rejection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionReturnDecision {
    KeepTalking,
    Rejected,
}

/// Clear an exact proposal and resume assistant-first context gathering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReturnWorkerIntroductionToContextCommand {
    pub worker_id: String,
    pub proposal_id: String,
    pub proposal_revision: u32,
    pub decision: WorkerIntroductionReturnDecision,
}

/// Complete replacement of the mutable Worker profile. HTTP clients may send
/// a partial patch, but the server resolves it against one durable snapshot
/// and the daemon applies this full value only when `expected_revision` still
/// matches. This keeps documents and the DM execution identity in one commit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateWorkerCommand {
    pub worker_id: String,
    pub expected_revision: u64,
    pub display_name: String,
    pub avatar_color: Option<String>,
    pub model: Option<String>,
    pub model_key: Option<ModelKey>,
    pub model_catalog_revision: Option<String>,
    pub permission_mode: String,
    pub autonomy: String,
    pub heartbeat_interval_secs: Option<u32>,
    pub identity: Option<String>,
    pub soul: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerTargetStatus {
    Paused,
    Active,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetWorkerStatusCommand {
    pub worker_id: String,
    /// CAS for Worker profile and execution provenance. A status-only
    /// transition does not advance this revision; a future lifecycle/status
    /// revision must be distinct so provenance fences are never repurposed.
    pub expected_revision: u64,
    pub status: WorkerTargetStatus,
}

/// Request one daemon-authored recovery grant for an exact owned Worker. The
/// command intentionally carries no bypass flags, expiry, run, or lane: those
/// are fixed and validated by the daemon so a client cannot widen authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GrantWorkerGovernorRecoveryCommand {
    pub worker_id: String,
}

/// Start or resume one exact user-approved Goal for one owned Worker. The
/// envelope idempotency key is the durable operation identity; actor ownership
/// is injected by the daemon and is intentionally absent here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivateOrResumeWorkerWorkflowCommand {
    pub worker_id: String,
    pub expected_worker_revision: u64,
    pub goal_id: String,
    pub expected_goal_revision: u64,
}

/// Pause or cancel one exact Worker-owned Goal. Separate command variants keep
/// lifecycle intent stable while sharing this revision-fenced payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerWorkflowLifecycleCommand {
    pub worker_id: String,
    pub expected_worker_revision: u64,
    pub goal_id: String,
    pub expected_goal_revision: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerWorkspaceMode {
    Neutral,
    Selected,
    Created,
}

/// Explicitly attach or detach the private Worker conversation workspace.
/// Attached Workflow v1 workspaces require one exact canonical directory for
/// both working and project scope; no daemon cwd fallback is representable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetWorkerWorkspaceCommand {
    pub worker_id: String,
    pub expected_worker_revision: u64,
    pub workspace_mode: WorkerWorkspaceMode,
    pub working_dir: Option<String>,
    pub project_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalAcceptanceDecision {
    Accept,
    Reject,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalCriterionDecision {
    Passed,
    Failed,
    Waived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerGoalCriterionAcceptance {
    pub criterion_id: String,
    pub decision: WorkerGoalCriterionDecision,
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// User-supplied values for one storage-authored acceptance run. Worker,
/// Workflow, attempt, plan, and step identities are deliberately absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolveWorkerGoalAcceptanceCommand {
    pub acceptance_run_id: String,
    pub expected_goal_revision: u64,
    pub decision: WorkerGoalAcceptanceDecision,
    pub reason: String,
    #[serde(default)]
    pub criteria: Vec<WorkerGoalCriterionAcceptance>,
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
            outcome: ResponseOutcome::Success {
                payload: Box::new(payload),
            },
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
    Success { payload: Box<ResponsePayload> },
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
    WorkerIntroduction(WorkerIntroductionResponse),
    WorkerIntroductionAction(WorkerIntroductionActionResponse),
    WorkerMutation(WorkerMutationResponse),
    WorkerGovernorRecovery(WorkerGovernorRecoveryResponse),
    WorkerConversationInput(WorkerConversationInputResponse),
    WorkerWorkflow(WorkerWorkflowResponse),
    WorkerGoalAcceptance(WorkerGoalAcceptanceResponse),
    WorkerWorkspace(WorkerWorkspaceResponse),
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

/// Durable identities committed by a create-and-meet request. Replaying the
/// same IPC idempotency key returns these exact ids.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerIntroductionResponse {
    pub worker_id: String,
    pub session_id: String,
    pub run_id: String,
    pub status: String,
    /// Newly created Workers always begin at revision 1.
    #[serde(default = "initial_worker_revision")]
    pub revision: u64,
}

/// Exact durable disposition for content accepted into a Worker's private DM.
/// A queued input is already canonical and owns a new serialized response run;
/// a staged input is non-canonical until the active response commits and
/// adopts it into a replacement run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerConversationInputDisposition {
    Queued,
    Staged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerConversationInputResponse {
    pub worker_id: String,
    pub session_id: String,
    pub disposition: WorkerConversationInputDisposition,
    /// Newly queued response run or the exact unfinished run that fenced a
    /// staged input.
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_message_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_input_id: Option<String>,
}

const fn initial_worker_revision() -> u64 {
    1
}

/// Result of an explicit retry or skip. Legacy Workers can be explicitly
/// skipped without ever having had a run, hence the optional run id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerIntroductionActionResponse {
    pub worker_id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub status: String,
    pub autonomy_eligible: bool,
    /// The daemon durably cancelled an in-flight Introduction and callers
    /// should deliver cooperative cancellation after commit.
    #[serde(default)]
    pub cancellation_requested: bool,
}

/// One exact running claim that was durably fenced before cooperative
/// cancellation was delivered. The worker and run ids prevent a late signal
/// from cancelling a replacement run in the same conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerRunCancellation {
    pub worker_id: String,
    pub session_id: String,
    pub run_id: String,
    pub reason: String,
}

/// A lane which remains paused because an uncertain run requires explicit
/// recovery. Resume never converts these runs back into executable work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerLaneAttention {
    pub session_id: String,
    pub controller_id: String,
    pub recovery_run_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerMutationResponse {
    pub worker_id: String,
    pub revision: u64,
    pub status: String,
    #[serde(default)]
    pub cancellation_requests: Vec<WorkerRunCancellation>,
    #[serde(default)]
    pub attention: Vec<WorkerLaneAttention>,
}

/// Daemon-authored result of an exact-owner Worker recovery action. Unresolved
/// accounting returns one short-lived grant. Acknowledged response loss alone
/// has no grant; if older unresolved accounting also remains, its settlement
/// returns the same narrow short-lived authority for the successor DM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerGovernorRecoveryResponse {
    pub worker_id: String,
    pub grant_id: Option<String>,
    pub expires_at: Option<String>,
    pub status: String,
    pub bypass_unresolved_provider_call: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerWorkflowDisposition {
    Created,
    Existing,
    Paused,
    Cancelled,
}

/// Authoritative projection for the one run/attempt created or adopted by an
/// activation. Lifecycle responses omit this projection and instead return
/// every affected durable identity below.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerWorkflowRunProjection {
    pub run_id: String,
    pub run_status: String,
    pub attempt_id: String,
    pub attempt_status: String,
}

/// One exact running Workflow attempt durably fenced before cooperative
/// cancellation. Replaying the command may safely re-emit this signal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerWorkflowRunCancellation {
    pub worker_id: String,
    pub session_id: String,
    pub goal_id: String,
    pub run_id: String,
    pub reason: String,
}

/// Typed daemon result for Worker Workflow activation, pause, and cancel.
/// Revisions and statuses are core-authored from the same transaction as the
/// command receipt; clients never infer them from optimistic local state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerWorkflowResponse {
    pub disposition: WorkerWorkflowDisposition,
    pub worker_id: String,
    pub worker_revision: u64,
    pub session_id: String,
    pub goal_id: String,
    pub goal_revision: u64,
    pub goal_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<WorkerWorkflowRunProjection>,
    #[serde(default)]
    pub affected_run_ids: Vec<String>,
    #[serde(default)]
    pub affected_attempt_ids: Vec<String>,
    #[serde(default)]
    pub cancellation_requests: Vec<WorkerWorkflowRunCancellation>,
}

/// Stable result for a durable acceptance decision. The response omits the
/// store's inserted/adopted implementation detail so an exact recovery replay
/// remains byte-equivalent to the first successful response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerGoalAcceptanceResponse {
    pub acceptance_run_id: String,
    pub source_run_id: String,
    pub workflow_goal_id: String,
    pub source_attempt_id: String,
    pub step_id: String,
    pub decision: WorkerGoalAcceptanceDecision,
    pub goal_revision: u64,
    pub goal_status: String,
    pub step_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerWorkspaceResponse {
    pub worker_id: String,
    pub revision: u64,
    pub session_id: String,
    pub workspace_mode: WorkerWorkspaceMode,
    pub working_dir: Option<String>,
    pub project_dir: Option<String>,
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
    fn boxed_success_payload_preserves_response_wire_shape() {
        let response = ResponseEnvelope::success(
            "request-1",
            ResponsePayload::Pong(PongResponse {
                instance_id: "daemon-1".into(),
                daemon_version: "1.0.0".into(),
                uptime_secs: 42,
                server_time_unix_ms: 1_234,
            }),
        );

        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({
                "version": {
                    "major": PROTOCOL_MAJOR,
                    "minor": PROTOCOL_MINOR,
                },
                "request_id": "request-1",
                "outcome": {
                    "status": "success",
                    "payload": {
                        "response": "pong",
                        "data": {
                            "instance_id": "daemon-1",
                            "daemon_version": "1.0.0",
                            "uptime_secs": 42,
                            "server_time_unix_ms": 1_234,
                        },
                    },
                },
            })
        );
        assert_eq!(
            serde_json::from_value::<ResponseEnvelope>(encoded).unwrap(),
            response
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
        let archive = Command::GroupArchive(GroupArchiveCommand {
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
        assert_eq!(
            archive.minimum_protocol_minor(),
            crate::GROUP_ARCHIVE_PROTOCOL_MINOR
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
    fn worker_introduction_requires_v1_4_and_round_trips_exact_identity() {
        let command = Command::CreateWorkerIntroduction(CreateWorkerIntroductionCommand {
            slug: "researcher".into(),
            display_name: "Researcher".into(),
            avatar_color: Some("#7743DB".into()),
            model: "grok-4.6".into(),
            model_key: ModelKey {
                provider: "grok".into(),
                model_id: "grok-4.6".into(),
                auth_scope: Some("oauth".into()),
                api_format: "open_ai_responses".into(),
            },
            model_catalog_revision: Some("catalog-1".into()),
            permission_mode: "supervised".into(),
            autonomy: "manual".into(),
            heartbeat_interval_secs: None,
            identity: None,
            soul: None,
        });
        assert_eq!(
            command.minimum_protocol_minor(),
            crate::WORKER_INTRODUCTION_PROTOCOL_MINOR
        );
        let encoded = serde_json::to_vec(&command).unwrap();
        assert_eq!(
            serde_json::from_slice::<Command>(&encoded).unwrap(),
            command
        );

        let response = ResponsePayload::WorkerIntroduction(WorkerIntroductionResponse {
            worker_id: "worker-1".into(),
            session_id: "session-1".into(),
            run_id: "run-1".into(),
            status: "queued".into(),
            revision: 1,
        });
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["response"], "worker_introduction");
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            response
        );

        for action in [
            Command::RetryWorkerIntroduction(WorkerIntroductionCommand {
                worker_id: "worker-1".into(),
            }),
            Command::SkipWorkerIntroduction(WorkerIntroductionCommand {
                worker_id: "worker-1".into(),
            }),
        ] {
            assert_eq!(
                action.minimum_protocol_minor(),
                crate::WORKER_INTRODUCTION_PROTOCOL_MINOR
            );
            let encoded = serde_json::to_vec(&action).unwrap();
            assert_eq!(serde_json::from_slice::<Command>(&encoded).unwrap(), action);
        }
        let action_response =
            ResponsePayload::WorkerIntroductionAction(WorkerIntroductionActionResponse {
                worker_id: "worker-1".into(),
                session_id: "session-1".into(),
                run_id: Some("run-2".into()),
                status: "queued".into(),
                autonomy_eligible: false,
                cancellation_requested: false,
            });
        let encoded = serde_json::to_value(&action_response).unwrap();
        assert_eq!(encoded["response"], "worker_introduction_action");
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            action_response
        );
    }

    #[test]
    fn worker_introduction_review_decisions_require_v1_5_and_round_trip() {
        let confirm = Command::ConfirmWorkerIntroduction(ConfirmWorkerIntroductionCommand {
            worker_id: "worker-1".into(),
            proposal_id: "proposal-1".into(),
            proposal_revision: 2,
            selected_facts: vec![WorkerIntroductionSelectedFact {
                fact_id: "fact-1".into(),
                final_statement: "Help investigate runtime reliability.".into(),
            }],
        });
        let keep_talking =
            Command::ReturnWorkerIntroductionToContext(ReturnWorkerIntroductionToContextCommand {
                worker_id: "worker-1".into(),
                proposal_id: "proposal-1".into(),
                proposal_revision: 2,
                decision: WorkerIntroductionReturnDecision::KeepTalking,
            });
        let rejected =
            Command::ReturnWorkerIntroductionToContext(ReturnWorkerIntroductionToContextCommand {
                worker_id: "worker-1".into(),
                proposal_id: "proposal-1".into(),
                proposal_revision: 2,
                decision: WorkerIntroductionReturnDecision::Rejected,
            });

        for command in [confirm, keep_talking, rejected] {
            assert_eq!(
                command.minimum_protocol_minor(),
                crate::WORKER_INTRODUCTION_REVIEW_PROTOCOL_MINOR
            );
            let encoded = serde_json::to_vec(&command).unwrap();
            assert_eq!(
                serde_json::from_slice::<Command>(&encoded).unwrap(),
                command
            );
        }
        assert_eq!(
            Command::ConfirmWorkerIntroduction(ConfirmWorkerIntroductionCommand {
                worker_id: "worker-1".into(),
                proposal_id: "proposal-1".into(),
                proposal_revision: 1,
                selected_facts: vec![],
            })
            .name(),
            "confirm_worker_introduction"
        );
        assert_eq!(
            Command::ReturnWorkerIntroductionToContext(ReturnWorkerIntroductionToContextCommand {
                worker_id: "worker-1".into(),
                proposal_id: "proposal-1".into(),
                proposal_revision: 1,
                decision: WorkerIntroductionReturnDecision::KeepTalking,
            })
            .name(),
            "return_worker_introduction_to_context"
        );

        // Existing create/retry/skip commands remain available to v1.4 peers.
        assert_eq!(
            Command::RetryWorkerIntroduction(WorkerIntroductionCommand {
                worker_id: "worker-1".into(),
            })
            .minimum_protocol_minor(),
            crate::WORKER_INTRODUCTION_PROTOCOL_MINOR
        );
    }

    #[test]
    fn worker_lifecycle_mutations_require_v1_6_and_round_trip() {
        let update = Command::UpdateWorker(UpdateWorkerCommand {
            worker_id: "worker-1".into(),
            expected_revision: 7,
            display_name: "Researcher".into(),
            avatar_color: Some("#7743DB".into()),
            model: Some("grok-4.6".into()),
            model_key: Some(ModelKey {
                provider: "grok".into(),
                model_id: "grok-4.6".into(),
                auth_scope: Some("oauth".into()),
                api_format: "open_ai_responses".into(),
            }),
            model_catalog_revision: Some("catalog-7".into()),
            permission_mode: "supervised".into(),
            autonomy: "always_on".into(),
            heartbeat_interval_secs: Some(900),
            identity: Some("Investigate runtime reliability.".into()),
            soul: Some("Be curious and exact.".into()),
        });
        let pause = Command::SetWorkerStatus(SetWorkerStatusCommand {
            worker_id: "worker-1".into(),
            expected_revision: 8,
            status: WorkerTargetStatus::Paused,
        });
        for command in [update, pause] {
            assert_eq!(
                command.minimum_protocol_minor(),
                crate::WORKER_LIFECYCLE_PROTOCOL_MINOR
            );
            assert!(matches!(
                command.name(),
                "update_worker" | "set_worker_status"
            ));
            let encoded = serde_json::to_vec(&command).unwrap();
            assert_eq!(
                serde_json::from_slice::<Command>(&encoded).unwrap(),
                command
            );
        }

        let response = ResponsePayload::WorkerMutation(WorkerMutationResponse {
            worker_id: "worker-1".into(),
            revision: 9,
            status: "paused".into(),
            cancellation_requests: vec![WorkerRunCancellation {
                worker_id: "worker-1".into(),
                session_id: "session-1".into(),
                run_id: "run-1".into(),
                reason: "Worker paused during execution".into(),
            }],
            attention: vec![WorkerLaneAttention {
                session_id: "session-1".into(),
                controller_id: "controller-1".into(),
                recovery_run_ids: vec!["run-1".into()],
                reason: "explicit recovery required".into(),
            }],
        });
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["response"], "worker_mutation");
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            response
        );
    }

    #[test]
    fn worker_conversation_acceptance_requires_v1_7_and_round_trips() {
        for command in [
            Command::WorkerSendMessage(MessageCommand {
                session_id: "worker-dm".into(),
                message: "hello".into(),
            }),
            Command::WorkerSteer(SteerCommand {
                session_id: "worker-dm".into(),
                pending_id: Some("input-1".into()),
                content: serde_json::json!([{"type": "text", "text": "hello"}]),
            }),
            Command::WorkerUserResponse(UserResponseCommand {
                session_id: "worker-dm".into(),
                run_id: "run-1".into(),
                tool_call_id: "question-1".into(),
                response: "hello".into(),
            }),
        ] {
            assert_eq!(
                command.minimum_protocol_minor(),
                crate::WORKER_CONVERSATION_PROTOCOL_MINOR
            );
            let encoded = serde_json::to_vec(&command).unwrap();
            assert_eq!(
                serde_json::from_slice::<Command>(&encoded).unwrap(),
                command
            );
        }

        for legacy in [
            Command::SendMessage(MessageCommand {
                session_id: "primary-hive".into(),
                message: "hello".into(),
            }),
            Command::Steer(SteerCommand {
                session_id: "primary-hive".into(),
                pending_id: None,
                content: serde_json::json!([]),
            }),
            Command::UserResponse(UserResponseCommand {
                session_id: "primary-hive".into(),
                run_id: "run-1".into(),
                tool_call_id: "question-1".into(),
                response: "hello".into(),
            }),
        ] {
            assert_eq!(legacy.minimum_protocol_minor(), 0);
        }

        let stop = Command::StopWorkerConversation(SessionCommand {
            session_id: "worker-dm".into(),
        });
        assert_eq!(stop.name(), "stop_worker_conversation");
        assert_eq!(
            stop.minimum_protocol_minor(),
            crate::WORKER_CONVERSATION_STOP_PROTOCOL_MINOR
        );
        let encoded = serde_json::to_vec(&stop).unwrap();
        assert_eq!(serde_json::from_slice::<Command>(&encoded).unwrap(), stop);

        for response in [
            WorkerConversationInputResponse {
                worker_id: "worker-1".into(),
                session_id: "worker-dm".into(),
                disposition: WorkerConversationInputDisposition::Queued,
                run_id: "run-1".into(),
                canonical_message_id: Some(42),
                staged_input_id: None,
            },
            WorkerConversationInputResponse {
                worker_id: "worker-1".into(),
                session_id: "worker-dm".into(),
                disposition: WorkerConversationInputDisposition::Staged,
                run_id: "run-1".into(),
                canonical_message_id: None,
                staged_input_id: Some("input-1".into()),
            },
        ] {
            let payload = ResponsePayload::WorkerConversationInput(response);
            let encoded = serde_json::to_value(&payload).unwrap();
            assert_eq!(encoded["response"], "worker_conversation_input");
            assert_eq!(
                serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
                payload
            );
        }
    }

    #[test]
    fn worker_workflow_commands_require_v1_8_and_round_trip() {
        let activate =
            Command::ActivateOrResumeWorkerWorkflow(ActivateOrResumeWorkerWorkflowCommand {
                worker_id: "worker-1".into(),
                expected_worker_revision: 7,
                goal_id: "goal-1".into(),
                expected_goal_revision: 11,
            });
        let pause = Command::PauseWorkerWorkflow(WorkerWorkflowLifecycleCommand {
            worker_id: "worker-1".into(),
            expected_worker_revision: 7,
            goal_id: "goal-1".into(),
            expected_goal_revision: 11,
            reason: "Paused by the user".into(),
        });
        let cancel = Command::CancelWorkerWorkflow(WorkerWorkflowLifecycleCommand {
            worker_id: "worker-1".into(),
            expected_worker_revision: 7,
            goal_id: "goal-1".into(),
            expected_goal_revision: 11,
            reason: "Cancelled by the user".into(),
        });
        let workspace = Command::SetWorkerWorkspace(SetWorkerWorkspaceCommand {
            worker_id: "worker-1".into(),
            expected_worker_revision: 7,
            workspace_mode: WorkerWorkspaceMode::Selected,
            working_dir: Some("/work/project".into()),
            project_dir: Some("/work/project".into()),
        });

        for (command, stable_name) in [
            (activate, "activate_or_resume_worker_workflow"),
            (pause, "pause_worker_workflow"),
            (cancel, "cancel_worker_workflow"),
            (workspace, "set_worker_workspace"),
        ] {
            assert_eq!(command.name(), stable_name);
            assert_eq!(
                command.minimum_protocol_minor(),
                crate::WORKER_WORKFLOW_PROTOCOL_MINOR
            );
            let encoded = serde_json::to_vec(&command).unwrap();
            assert_eq!(
                serde_json::from_slice::<Command>(&encoded).unwrap(),
                command
            );
        }

        let payload = ResponsePayload::WorkerWorkflow(WorkerWorkflowResponse {
            disposition: WorkerWorkflowDisposition::Created,
            worker_id: "worker-1".into(),
            worker_revision: 7,
            session_id: "worker-dm".into(),
            goal_id: "goal-1".into(),
            goal_revision: 11,
            goal_status: "active".into(),
            active: Some(WorkerWorkflowRunProjection {
                run_id: "run-1".into(),
                run_status: "queued".into(),
                attempt_id: "attempt-1".into(),
                attempt_status: "running".into(),
            }),
            affected_run_ids: vec![],
            affected_attempt_ids: vec![],
            cancellation_requests: vec![],
        });
        let encoded = serde_json::to_value(&payload).unwrap();
        assert_eq!(encoded["response"], "worker_workflow");
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            payload
        );

        let workspace = ResponsePayload::WorkerWorkspace(WorkerWorkspaceResponse {
            worker_id: "worker-1".into(),
            revision: 8,
            session_id: "worker-dm".into(),
            workspace_mode: WorkerWorkspaceMode::Selected,
            working_dir: Some("/work/project".into()),
            project_dir: Some("/work/project".into()),
        });
        let encoded = serde_json::to_value(&workspace).unwrap();
        assert_eq!(encoded["response"], "worker_workspace");
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            workspace
        );
    }

    #[test]
    fn worker_goal_acceptance_requires_v1_9_and_round_trips() {
        let command = Command::ResolveWorkerGoalAcceptance(ResolveWorkerGoalAcceptanceCommand {
            acceptance_run_id: "acceptance-1".into(),
            expected_goal_revision: 12,
            decision: WorkerGoalAcceptanceDecision::Accept,
            reason: "Reviewed the bounded result".into(),
            criteria: vec![WorkerGoalCriterionAcceptance {
                criterion_id: "criterion-1".into(),
                decision: WorkerGoalCriterionDecision::Passed,
                evidence: vec!["Focused validation passed".into()],
            }],
        });
        assert_eq!(command.name(), "resolve_worker_goal_acceptance");
        assert_eq!(
            command.minimum_protocol_minor(),
            crate::WORKER_GOAL_ACCEPTANCE_PROTOCOL_MINOR
        );
        let encoded = serde_json::to_vec(&command).unwrap();
        assert_eq!(
            serde_json::from_slice::<Command>(&encoded).unwrap(),
            command
        );

        let payload = ResponsePayload::WorkerGoalAcceptance(WorkerGoalAcceptanceResponse {
            acceptance_run_id: "acceptance-1".into(),
            source_run_id: "run-1".into(),
            workflow_goal_id: "goal-1".into(),
            source_attempt_id: "attempt-1".into(),
            step_id: "step-1".into(),
            decision: WorkerGoalAcceptanceDecision::Accept,
            goal_revision: 13,
            goal_status: "active".into(),
            step_status: "completed".into(),
        });
        let encoded = serde_json::to_value(&payload).unwrap();
        assert_eq!(encoded["response"], "worker_goal_acceptance");
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            payload
        );
    }

    #[test]
    fn worker_governor_recovery_requires_v1_10_and_round_trips_narrow_authority() {
        let command = Command::GrantWorkerGovernorRecovery(GrantWorkerGovernorRecoveryCommand {
            worker_id: "worker-1".into(),
        });
        assert_eq!(command.name(), "grant_worker_governor_recovery");
        assert_eq!(
            command.minimum_protocol_minor(),
            crate::WORKER_GOVERNOR_RECOVERY_PROTOCOL_MINOR
        );
        let encoded = serde_json::to_vec(&command).unwrap();
        assert_eq!(
            serde_json::from_slice::<Command>(&encoded).unwrap(),
            command
        );

        let payload = ResponsePayload::WorkerGovernorRecovery(WorkerGovernorRecoveryResponse {
            worker_id: "worker-1".into(),
            grant_id: Some("grant-1".into()),
            expires_at: Some("2026-08-25T12:05:00.000000Z".into()),
            status: "granted".into(),
            bypass_unresolved_provider_call: true,
        });
        let encoded = serde_json::to_value(&payload).unwrap();
        assert_eq!(encoded["response"], "worker_governor_recovery");
        assert_eq!(encoded["data"]["bypass_unresolved_provider_call"], true);
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            payload
        );

        let response_loss =
            ResponsePayload::WorkerGovernorRecovery(WorkerGovernorRecoveryResponse {
                worker_id: "worker-1".into(),
                grant_id: None,
                expires_at: None,
                status: "response_loss_acknowledged".into(),
                bypass_unresolved_provider_call: false,
            });
        let encoded = serde_json::to_value(&response_loss).unwrap();
        assert!(encoded["data"]["grant_id"].is_null());
        assert_eq!(
            serde_json::from_value::<ResponsePayload>(encoded).unwrap(),
            response_loss
        );
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
