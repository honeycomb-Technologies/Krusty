//! Restricted post-turn learning for Mako.
//!
//! The reviewer may propose durable facts; it never receives tools and cannot
//! mutate identity, soul, files, skills, channels, or permissions.

mod policy;
mod types;

pub use policy::{LearningDecision, LearningPolicy};
pub use types::{LearningProposal, LearningReviewerOutput};
