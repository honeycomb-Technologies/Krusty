//! Restricted post-turn learning for Mako.
//!
//! The reviewer may propose durable facts; it never receives tools and cannot
//! mutate identity, soul, files, skills, channels, or permissions.

mod policy;
mod reviewer;
mod transcript;
mod types;

pub use policy::{LearningDecision, LearningPolicy};
pub use reviewer::{
    review_latest_completed_mako_turn, LearningReviewOutcome, PostTurnLearningReviewRequest,
};
pub use types::{LearningProposal, LearningReviewerOutput, LearningScope};
