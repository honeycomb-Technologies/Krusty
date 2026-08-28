//! Canonical persistence boundary for neutral Hive Worker responses.
//!
//! Provider acknowledgement is not conversation durability. A neutral Worker
//! run therefore hands its text to this exact fenced capability before the
//! provider-call ledger may be terminalized. Implementations must commit or
//! adopt one deterministic assistant message (and its group projection, when
//! applicable) atomically under the supplied run lease.

use crate::storage::WorkerConversationLane;

pub use crate::storage::{
    SqliteWorkerConversationResponseStore as SqliteWorkerConversationResponseCommitter,
    WorkerConversationResponseCommit, WorkerConversationResponseCommitDisposition,
    WorkerConversationResponseCommitError,
};

/// Exact authority and content for one canonical Worker response commit.
///
/// Implementations must compare every fence to current durable state, use a
/// deterministic run-scoped idempotency key, and return `Ok` only when the
/// canonical row is durably present with identical content. A stale fence or
/// conflicting existing row is an error, never an adoption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConversationResponseCommitInput {
    pub worker_id: String,
    pub worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub lane: WorkerConversationLane,
    pub run_id: String,
    pub run_lease_token: String,
    pub run_lease_epoch: u64,
    pub provider_call_id: String,
    /// Canonical user-visible assistant text. Thinking and tool payloads are
    /// intentionally excluded from this contract.
    pub response_text: String,
}

/// Trusted persistence capability supplied by the claimed Hive runtime.
///
/// This is synchronous because the canonical SQLite transaction is bounded.
/// Callers run it once after the remote stream completes and before marking
/// the append-only provider call Completed.
pub trait WorkerConversationResponseCommitter: Send + Sync {
    fn commit_response(
        &self,
        input: &WorkerConversationResponseCommitInput,
    ) -> Result<WorkerConversationResponseCommit, WorkerConversationResponseCommitError>;
}

impl WorkerConversationResponseCommitter for crate::storage::SqliteWorkerConversationResponseStore {
    fn commit_response(
        &self,
        input: &WorkerConversationResponseCommitInput,
    ) -> Result<WorkerConversationResponseCommit, WorkerConversationResponseCommitError> {
        crate::storage::SqliteWorkerConversationResponseStore::commit_response(
            self,
            &crate::storage::CommitWorkerConversationResponse {
                worker_id: input.worker_id.clone(),
                worker_revision: input.worker_revision,
                owner_user_id: input.owner_user_id.clone(),
                session_id: input.session_id.clone(),
                lane: input.lane.clone(),
                run_id: input.run_id.clone(),
                run_lease_token: input.run_lease_token.clone(),
                run_lease_epoch: input.run_lease_epoch,
                provider_call_id: input.provider_call_id.clone(),
                response_text: input.response_text.clone(),
                committed_at: chrono::Utc::now(),
            },
        )
    }
}
