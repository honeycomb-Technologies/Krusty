mod acceptance;
mod acceptance_store;
mod model;
mod store;

pub use acceptance::{
    WorkerGoalAcceptanceAssessment, WorkerGoalAcceptanceAuthority,
    WorkerGoalAcceptanceCandidateRecord, WorkerGoalAcceptanceCandidateState,
    WorkerGoalAcceptanceCommitDisposition, WorkerGoalAcceptanceContractV1,
    WorkerGoalAcceptanceIntentV1, WorkerGoalAcceptanceReceipt, WorkerGoalAcceptanceReceiptKind,
    WorkerGoalAcceptanceResolution, WorkerGoalAcceptanceResultRecord,
    WorkerGoalAcceptanceSourceSummary, WorkerGoalCriterionAcceptanceSpecV1,
    MAX_WORKER_GOAL_ACCEPTANCE_RECEIPTS, MAX_WORKER_GOAL_ACCEPTANCE_RECEIPT_DURATION_MILLIS,
    MAX_WORKER_GOAL_ACCEPTANCE_RECEIPT_SUMMARY_BYTES, WORKER_GOAL_ACCEPTANCE_CONTRACT_VERSION,
    WORKER_GOAL_ACCEPTANCE_INTENT_VERSION, WORKER_GOAL_AUTOMATIC_ACCEPTANCE_ENABLED,
};
pub(crate) use acceptance_store::{
    pending_worker_goal_acceptance_exists_in_transaction,
    progressed_acceptance_is_staged_in_transaction, stage_user_review_acceptance_in_transaction,
    terminalize_pending_worker_goal_acceptances_in_transaction, WorkerGoalAcceptanceLifecycle,
    WorkerGoalAcceptanceStageError,
};
pub use acceptance_store::{SqliteWorkerGoalAcceptanceStore, WorkerGoalAcceptanceStoreError};
pub use model::WorkerGoalOutcomeRecord;
pub(crate) use store::{
    committed_worker_goal_outcome_in_transaction,
    pause_worker_workflow_after_uncertain_run_in_transaction,
    worker_goal_outcome_is_accounted_in_transaction,
};
pub use store::{
    reconcile_worker_workflow_provider_boundary_in_transaction, SqliteWorkerGoalOutcomeStore,
    WorkerWorkflowProviderRecovery,
};

#[cfg(test)]
mod tests;
