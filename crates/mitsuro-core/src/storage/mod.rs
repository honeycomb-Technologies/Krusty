//! Persistence layer
//!
//! SQLite-based storage for:
//! - Session storage and management
//! - Plan storage with session linkage
//! - User preferences
//! - File activity tracking for context
//! - API credentials

use std::time::{SystemTime, UNIX_EPOCH};

mod agent_state;
pub mod apns_devices;
pub mod autonomous_tasks;
mod block_ui;
mod compaction;
pub mod credentials;
mod database;
#[cfg(test)]
mod database_tests;
mod delegated_runs;
mod delegation;
mod episodes;
pub mod expo_push_devices;
mod file_activity;
mod hive_attention_state;
mod hive_controller_events;
mod hive_controllers;
mod hive_daemon_leases;
mod hive_deliveries;
pub mod hive_groups;
mod hive_home;
mod hive_idempotency;
mod hive_profiles;
mod hive_runs;
mod hive_runtime_state;
mod hive_schedules;
mod hive_workers;
mod knowledge;
mod learning_candidates;
pub mod live_activity_tokens;
mod memories;
mod messages;
pub mod mobile_diagnostics;
pub mod notification_intents;
mod plans;
mod preferences;
mod project_settings;
pub mod push_delivery_attempts;
pub mod push_subscriptions;
mod recovery;
pub mod reports;
mod runtime_traces;
mod sessions;

pub use agent_state::AgentState;
pub use apns_devices::{ApnsDevice, ApnsDeviceRegistration, ApnsDeviceStore};
pub use autonomous_tasks::{AutonomousTask, AutonomousTaskStore, TaskStatus};
pub use block_ui::BlockUiState;
pub use compaction::{CompactionSegmentRecord, CompactionStore};
pub use credentials::CredentialStore;
pub use database::{Database, SharedDatabase};
pub use delegated_runs::{
    normalize_scope_key, DelegatedRunAgentSnapshot, DelegatedRunCreateOutcome, DelegatedRunLease,
    DelegatedRunRecord, DelegatedRunRole, DelegatedRunScope, DelegatedRunSnapshot,
    DelegatedRunStartInput, DelegatedRunStore, DelegatedRunSummary,
};
pub use delegation::{
    DelegationCapacityClass, DelegationCapacityFeedback, DelegationCapacityPolicy,
    DelegationCapacityRequest, DelegationCompletionPolicy, DelegationEventRecord,
    DelegationEventType, DelegationExecutionMode, DelegationExecutorEnvelopeV1,
    DelegationExecutorKind, DelegationExecutorSessionType, DelegationFailurePolicy,
    DelegationGovernance, DelegationGroupContract, DelegationGroupRecord,
    DelegationGroupStartInput, DelegationGroupState, DelegationLeaseRenewalBatchResult,
    DelegationParentContinuationState, DelegationStore, DelegationSynthesisLease,
    DelegationSynthesisLeaseRenewal, DelegationTaskActivity, DelegationTaskLease,
    DelegationTaskLeaseRenewal, DelegationTaskRecord, DelegationTaskSpec, DelegationTaskState,
    DelegationWriterMode, DELEGATION_EXECUTOR_ENVELOPE_VERSION,
};
pub use episodes::{ConversationEpisode, EpisodeSearch, EpisodeStore};
pub use expo_push_devices::{ExpoPushDevice, ExpoPushDeviceRegistration, ExpoPushDeviceStore};
pub use file_activity::{FileActivityTracker, RankedFile};
pub use hive_attention_state::{HiveAttentionItemState, HiveAttentionStateStore};
pub use hive_controller_events::{
    HiveControllerEvent, HiveControllerEventStore, HiveControllerEventType, NewHiveControllerEvent,
};
pub use hive_controllers::{HiveController, HiveControllerStatus, HiveControllerStore};
pub use hive_daemon_leases::{DaemonLease, DaemonLeaseAcquire, HiveDaemonLeaseStore};
pub use hive_deliveries::{
    ack_for_terminal_runs_with_conn, claim_due_with_conn, enqueue_with_conn,
    fail_attempt_with_conn, hive_delivery_retry_backoff, load_delivery, mark_delivered_with_conn,
    revert_wait_with_conn, HiveDelivery, HiveDeliveryEnqueue, HiveDeliveryKind,
    HiveDeliveryPriority, HiveDeliveryStatus, HiveDeliveryStore, NewHiveDelivery,
    DEFAULT_HIVE_DELIVERY_MAX_ATTEMPTS, MAX_HIVE_DELIVERY_BODY_BYTES,
};
pub use hive_groups::{
    parse_group_mentions, GroupMentionTarget, HiveGroup, HiveGroupExecutionMode, HiveGroupMember,
    HiveGroupMessage, HiveGroupRunContext, HiveGroupSenderKind, HiveGroupStatus, HiveGroupStore,
    HiveGroupTurn, HiveGroupTurnPolicy, HiveGroupTurnStatus, HiveGroupUpdate, HiveMemberCursor,
    MentionResolution, NewHiveGroup, NewHiveGroupMessage, MAX_HIVE_GROUP_MESSAGE_BYTES,
};
pub use hive_home::{
    bootstrap_hive_home, is_valid_crew_slug, summarize_channel_bindings, summarize_crew_runtime,
    write_hive_crew_document, write_hive_home_document, HiveBootstrapResult, HiveChannelBinding,
    HiveChannelKind, HiveContextLayer, HiveCrewDocumentKind, HiveCrewProfile,
    HiveCrewRuntimeStatus, HiveCrewRuntimeSummary, HiveHomeDocument, HiveHomeDocumentKind,
    HiveHomeProfile,
};
pub use hive_idempotency::{
    hash_request_bytes, HiveIdempotencyStore, IdempotencyClaim, IdempotencyRecord,
};
pub use hive_profiles::{
    default_profile_seed, HiveCrewProfileDocumentKind, HiveCrewProfileSeed,
    HiveCrewProfileSnapshot, HiveLegacyImportResult, HiveProfileDocument, HiveProfileDocumentKind,
    HiveProfileMergeResult, HiveProfileOwner, HiveProfileOwnerError, HiveProfileSeed,
    HiveProfileSnapshot, HiveProfileStore, HiveProfileStoreError, MAX_HIVE_PROFILE_DOCUMENT_BYTES,
};
pub use hive_runs::{
    ClaimRunRequest, ClaimedHiveRun, DaemonFence, HiveRun, HiveRunAttempt, HiveRunAttemptOutcome,
    HiveRunKind, HiveRunStore, LeaseReconciliation, ReconciledRun, RunCompletion,
};
pub use hive_runtime_state::{
    HiveRunPriority, HiveRuntimeState, HiveRuntimeStateStatus, HiveRuntimeStateStore,
};
pub use hive_schedules::{
    HiveSchedule, HiveScheduleOccurrence, HiveScheduleOccurrenceStatus, HiveScheduleStatus,
    HiveScheduleStore, OverlapPolicy, OwnedHiveSchedule,
};
pub use hive_workers::{
    display_name_from_slug, load_worker_with_conn, HiveWorker, HiveWorkerAutonomy,
    HiveWorkerDocument, HiveWorkerDocumentKind, HiveWorkerProfileUpdate, HiveWorkerStatus,
    HiveWorkerStore, NewHiveWorker,
};
pub use knowledge::{
    get_current_snapshot, is_current_snapshot, is_current_snapshot_title, refresh_current_snapshot,
    KnowledgeSnapshot, CURRENT_SNAPSHOT_TITLE,
};
pub(crate) use learning_candidates::load_candidate_owned_from_connection;
pub use learning_candidates::{
    LearningCandidate, LearningCandidateInput, LearningCandidateStatus, LearningCandidateStore,
    LearningKind, LearningSensitivity, LearningThroughState,
};
pub use live_activity_tokens::{
    LiveActivityToken, LiveActivityTokenRegistration, LiveActivityTokenStore,
};
pub use memories::{
    is_compaction_flush_memory, AgentMemory, AgentMemoryRevision, CanonicalMemoryInput,
    HiveMemoryReader, MemoryAclScope, MemoryNamespace, MemoryRevisionEvent, MemorySensitivity,
    MemorySource, MemoryStatus, MemoryStore, MemoryType, COMPACTION_FLUSH_TITLE_PREFIX,
};
pub(crate) use memories::{
    load_canonical_for_provenance_from_connection, save_canonical_in_transaction,
};
pub use messages::{MessageStore, StoredMessageRecord};
pub use mobile_diagnostics::{
    MobileDiagnosticCategoryCount, MobileDiagnosticEvent, MobileDiagnosticEventInput,
    MobileDiagnosticNativePayload, MobileDiagnosticNativePayloadInput, MobileDiagnosticReport,
    MobileDiagnosticRun, MobileDiagnosticRunInput, MobileDiagnosticStore,
};
pub use notification_intents::{NotificationIntent, NotificationIntentStore};
pub use plans::{PlanStore, PlanSummary};
pub use preferences::Preferences;
pub use project_settings::{DelegationMode, ProjectAgentExtensionSettings, ProjectSettings};
pub use push_delivery_attempts::{
    PushDeliveryAttempt, PushDeliveryAttemptInput, PushDeliveryAttemptStore, PushDeliverySummary,
};
pub use push_subscriptions::{PushSubscription, PushSubscriptionStore};
pub use recovery::{
    ContinuationClaimSnapshot, PartialAssistantState, PendingInteractionSnapshot,
    PendingPlanTaskSnapshot, PendingQuestionOptionSnapshot, PendingQuestionSnapshot,
    RecoveryDecision, RecoveryNonResumableReason, RecoveryStatus, RecoveryToolArguments,
    RecoveryToolCall, SessionRecoveryState, REDACTED_ARGUMENT_VALUE,
};
pub use reports::{Report, ReportStore};
pub use runtime_traces::{
    ReplayExpectations, ReplayGateResult, RuntimeTraceEvent, RuntimeTraceStore,
    RuntimeTraceSummary, TraceEventCount, TraceFailureCategory,
};
pub use sessions::{SessionInfo, SessionManager, SessionType, WorkMode, WorkspaceMode};

/// Get current Unix timestamp in seconds
#[inline]
pub fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
