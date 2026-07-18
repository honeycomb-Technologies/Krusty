//! Auditable, user-reviewable learning proposals produced after Mako turns.

mod model;
mod store;

#[cfg(test)]
mod tests;

pub use model::{
    LearningCandidate, LearningCandidateInput, LearningCandidateStatus, LearningKind,
    LearningSensitivity, LearningThroughState,
};
pub(crate) use store::load_candidate_owned_from_connection;
pub use store::LearningCandidateStore;
