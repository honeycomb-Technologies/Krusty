//! Auditable, user-reviewable learning proposals produced after Mako turns.

mod model;
mod store;

#[cfg(test)]
mod tests;

pub use model::{
    LearningCandidate, LearningCandidateInput, LearningCandidateStatus, LearningKind,
    LearningSensitivity, LearningThroughState,
};
pub use store::LearningCandidateStore;
