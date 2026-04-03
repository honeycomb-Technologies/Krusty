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
pub mod credentials;
mod database;
#[cfg(test)]
mod database_tests;
mod delegated_runs;
mod file_activity;
mod memories;
mod messages;
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
pub use apns_devices::{ApnsDevice, ApnsDeviceStore};
pub use autonomous_tasks::{AutonomousTask, AutonomousTaskStore, TaskStatus};
pub use block_ui::BlockUiState;
pub use credentials::CredentialStore;
pub use database::{Database, SharedDatabase};
pub use delegated_runs::{
    normalize_scope_key, DelegatedRunAgentSnapshot, DelegatedRunRecord, DelegatedRunRole,
    DelegatedRunScope, DelegatedRunSnapshot, DelegatedRunStartInput, DelegatedRunStore,
};
pub use file_activity::{FileActivityTracker, RankedFile};
pub use memories::{AgentMemory, MemoryStore, MemoryType};
pub use messages::MessageStore;
pub use plans::{PlanStore, PlanSummary};
pub use preferences::Preferences;
pub use project_settings::ProjectSettings;
pub use push_delivery_attempts::{
    PushDeliveryAttempt, PushDeliveryAttemptInput, PushDeliveryAttemptStore, PushDeliverySummary,
};
pub use push_subscriptions::{PushSubscription, PushSubscriptionStore};
pub use recovery::{
    PartialAssistantState, RecoveryDecision, RecoveryNonResumableReason, RecoveryStatus,
    RecoveryToolCall, SessionRecoveryState,
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
