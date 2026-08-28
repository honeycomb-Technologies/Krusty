//! Typed persistence for the one-time Hive Worker Introduction lifecycle.

mod model;
mod store;

#[cfg(test)]
mod tests;

pub use model::{
    HiveWorkerIntroduction, HiveWorkerIntroductionStatus, WorkerIntroductionDecisionKind,
    WorkerIntroductionDecisionV1, WorkerIntroductionEvidenceAxis,
    WorkerIntroductionEvidenceCoverage, WorkerIntroductionFactKind,
    WorkerIntroductionProposalBasisV1, WorkerIntroductionProposalFactV1,
    WorkerIntroductionProposalV1, WorkerIntroductionReviewProjection,
    WorkerIntroductionReviewProjectionState, WorkerIntroductionReviewReadiness,
    WorkerIntroductionReviewRecord, WorkerIntroductionReviewStatus,
    WorkerIntroductionReviewerFactV1, WorkerIntroductionReviewerOutputV1,
    WorkerIntroductionSelectedFactV1, MAX_WORKER_INTRODUCTION_FACTS,
    WORKER_INTRODUCTION_PROPOSAL_VERSION,
};
#[cfg(test)]
pub(crate) use store::NewWorkerIntroductionReviewClaim;
pub use store::{save_worker_introduction_opening_once, HiveWorkerIntroductionStore};
pub(crate) use store::{
    ReviewProposalPersistence, WorkerIntroductionReviewStore, MAX_AUTOMATIC_REVIEW_ATTEMPTS,
};
