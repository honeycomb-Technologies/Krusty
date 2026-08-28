mod execution_context;
mod model;
mod store;

pub use execution_context::{
    HiveRunExecutionContextV1, HiveRunExecutionModeV1, HIVE_RUN_EXECUTION_CONTEXT_VERSION,
};
pub use model::{
    ClaimRunRequest, ClaimedHiveRun, DaemonFence, HiveRun, HiveRunAttempt, HiveRunAttemptOutcome,
    HiveRunKind, LeaseReconciliation, ReconciledRun, RunCompletion,
    WORKER_CONVERSATION_STOP_REQUESTED_REASON,
};
pub(crate) use store::{
    finalize_worker_conversation_after_governor_recovery_in_transaction,
    reactivate_worker_conversation_controller_after_governor_recovery_in_transaction,
    update_derived_state_for_run_in_transaction,
};
#[doc(hidden)]
pub use store::{
    reconcile_worker_introduction_review_in_transaction, HiveRunStore,
    WorkerIntroductionReviewRecovery,
};

#[cfg(test)]
mod tests;
