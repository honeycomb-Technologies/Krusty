//! Durable session-level orchestration records.
//!
//! A delegation group is the parent-owned unit of work. Its tasks are logical
//! objectives and `delegated_runs` are the replaceable execution attempts.

mod model;
mod store;

#[cfg(test)]
mod tests;

pub use model::{
    DelegationCapacityClass, DelegationCapacityFeedback, DelegationCapacityPolicy,
    DelegationCapacityRequest, DelegationCompletionPolicy, DelegationEventRecord,
    DelegationEventType, DelegationExecutionMode, DelegationExecutorEnvelopeV1,
    DelegationExecutorKind, DelegationExecutorSessionType, DelegationFailurePolicy,
    DelegationGovernance, DelegationGroupContract, DelegationGroupRecord,
    DelegationGroupStartInput, DelegationGroupState, DelegationParentContinuationState,
    DelegationSynthesisLease, DelegationTaskLease, DelegationTaskRecord, DelegationTaskSpec,
    DelegationTaskState, DelegationWriterMode, DELEGATION_EXECUTOR_ENVELOPE_VERSION,
};
pub use store::{
    DelegationLeaseRenewalBatchResult, DelegationStore, DelegationSynthesisLeaseRenewal,
    DelegationTaskLeaseRenewal,
};
