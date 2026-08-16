use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Room messages stay bounded like daemon chat messages.
pub const MAX_HIVE_GROUP_MESSAGE_BYTES: usize = 64 * 1024;

/// How one user message fans out across the group's Workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HiveGroupExecutionMode {
    /// Every target works in parallel, capped by the group's parallelism.
    #[default]
    Workbench,
    /// Targets speak one at a time in rotating rounds, capped by max_rounds.
    Roundtable,
    /// One assigned Worker handles the turn.
    Direct,
}

impl HiveGroupExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workbench => "workbench",
            Self::Roundtable => "roundtable",
            Self::Direct => "direct",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "workbench" => Some(Self::Workbench),
            "roundtable" => Some(Self::Roundtable),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }
}

impl std::fmt::Display for HiveGroupExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Archiving hides a group without destroying its timeline or Workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HiveGroupStatus {
    #[default]
    Active,
    Archived,
}

impl HiveGroupStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

impl std::fmt::Display for HiveGroupStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A group room referencing Workers by id. Groups cannot nest by
/// construction: there is no parent column and members are Workers only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveGroup {
    pub id: String,
    /// Exact owner; NULL means the local single-tenant profile.
    pub user_id: Option<String>,
    pub title: String,
    pub execution_mode: HiveGroupExecutionMode,
    pub max_rounds: u32,
    pub max_member_messages_per_turn: u32,
    pub parallelism: u32,
    pub context_window_messages: u32,
    pub status: HiveGroupStatus,
    /// Worker that handles turns in `direct` mode when no mention selects one.
    pub default_assignee_worker_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating a group. Members are ordered; caps fall back to the
/// schema defaults when absent.
#[derive(Debug, Clone, Default)]
pub struct NewHiveGroup {
    pub user_id: Option<String>,
    pub title: String,
    pub execution_mode: HiveGroupExecutionMode,
    pub max_rounds: Option<u32>,
    pub max_member_messages_per_turn: Option<u32>,
    pub parallelism: Option<u32>,
    pub context_window_messages: Option<u32>,
    pub default_assignee_worker_id: Option<String>,
    /// Ordered member Worker ids.
    pub member_worker_ids: Vec<String>,
}

/// Full overwrite of the editable policy surface of a group. Membership is
/// updated through `set_members`; status through `set_status`.
#[derive(Debug, Clone)]
pub struct HiveGroupUpdate {
    pub title: String,
    pub execution_mode: HiveGroupExecutionMode,
    pub max_rounds: u32,
    pub max_member_messages_per_turn: u32,
    pub parallelism: u32,
    pub context_window_messages: u32,
    pub default_assignee_worker_id: Option<String>,
}

impl From<&HiveGroup> for HiveGroupUpdate {
    fn from(group: &HiveGroup) -> Self {
        Self {
            title: group.title.clone(),
            execution_mode: group.execution_mode,
            max_rounds: group.max_rounds,
            max_member_messages_per_turn: group.max_member_messages_per_turn,
            parallelism: group.parallelism,
            context_window_messages: group.context_window_messages,
            default_assignee_worker_id: group.default_assignee_worker_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveGroupMember {
    pub group_id: String,
    pub worker_id: String,
    pub position: u32,
    pub added_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveGroupSenderKind {
    User,
    Worker,
    System,
}

impl HiveGroupSenderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Worker => "worker",
            Self::System => "system",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "worker" => Some(Self::Worker),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

impl std::fmt::Display for HiveGroupSenderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One append-only room message with a per-group monotonic sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveGroupMessage {
    pub id: String,
    pub group_id: String,
    pub seq: i64,
    pub sender_kind: HiveGroupSenderKind,
    pub sender_worker_id: Option<String>,
    /// Member run that posted this message (worker senders only). Enables the
    /// per-run posting cap without scanning run transcripts.
    pub sender_run_id: Option<String>,
    pub content: String,
    pub reply_to_message_id: Option<String>,
    /// Group turn this message belongs to (the trigger and every member post
    /// of one turn share it).
    pub turn_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NewHiveGroupMessage {
    pub group_id: String,
    pub sender_kind: HiveGroupSenderKind,
    pub sender_worker_id: Option<String>,
    pub sender_run_id: Option<String>,
    pub content: String,
    pub reply_to_message_id: Option<String>,
    pub turn_id: Option<String>,
    pub idempotency_key: Option<String>,
}

impl NewHiveGroupMessage {
    pub fn user(group_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            group_id: group_id.into(),
            sender_kind: HiveGroupSenderKind::User,
            sender_worker_id: None,
            sender_run_id: None,
            content: content.into(),
            reply_to_message_id: None,
            turn_id: None,
            idempotency_key: None,
        }
    }

    pub fn worker(
        group_id: impl Into<String>,
        worker_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            sender_kind: HiveGroupSenderKind::Worker,
            sender_worker_id: Some(worker_id.into()),
            ..Self::user(group_id, content)
        }
    }

    pub fn system(group_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            sender_kind: HiveGroupSenderKind::System,
            ..Self::user(group_id, content)
        }
    }
}

/// Terminal aggregate states of one group turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveGroupTurnStatus {
    Running,
    Completed,
    /// Some members succeeded and at least one failed; the room survives.
    Partial,
    Failed,
    Cancelled,
}

impl HiveGroupTurnStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "partial" => Some(Self::Partial),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

impl std::fmt::Display for HiveGroupTurnStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The group policy frozen into a turn when it starts, so mid-turn group
/// edits never change in-flight behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveGroupTurnPolicy {
    pub max_rounds: u32,
    pub max_member_messages_per_turn: u32,
    pub parallelism: u32,
    pub context_window_messages: u32,
}

impl From<&HiveGroup> for HiveGroupTurnPolicy {
    fn from(group: &HiveGroup) -> Self {
        Self {
            max_rounds: group.max_rounds,
            max_member_messages_per_turn: group.max_member_messages_per_turn,
            parallelism: group.parallelism,
            context_window_messages: group.context_window_messages,
        }
    }
}

/// Durable record of one group turn: the trigger, the frozen policy, the
/// ordered speaker plan, and aggregated per-member outcomes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiveGroupTurn {
    pub id: String,
    pub group_id: String,
    pub trigger_message_id: String,
    pub execution_mode: HiveGroupExecutionMode,
    pub policy: HiveGroupTurnPolicy,
    /// Ordered Worker ids still to be dispatched. Workbench/direct dispatch
    /// the whole plan up front; roundtable advances `next_speaker_index`.
    pub speaker_plan: Vec<String>,
    pub next_speaker_index: u32,
    pub status: HiveGroupTurnStatus,
    /// Per-member outcome summaries (`worker_id` keyed), populated as member
    /// runs reach terminal states.
    pub member_outcomes: Option<Value>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveMemberCursor {
    pub group_id: String,
    pub worker_id: String,
    pub last_seen_seq: i64,
    pub last_spoke_seq: i64,
    pub updated_at: String,
}

/// Group linkage carried by one member run into context building and tool
/// execution. Frozen from the turn's policy snapshot at dispatch time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveGroupRunContext {
    pub group_id: String,
    pub group_turn_id: String,
    /// The member run this context belongs to; the post_to_group cap is
    /// enforced per run so each roundtable round gets a fresh budget.
    pub run_id: String,
    pub worker_id: String,
    pub max_member_messages_per_turn: u32,
    pub context_window_messages: u32,
}
