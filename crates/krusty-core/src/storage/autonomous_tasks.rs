//! Lightweight task list for Mako agent coordination
//!
//! Tracks tasks within an autonomous session, including dependency edges
//! (blocked_by) so the orchestrator can schedule work in the right order.

mod model;
mod store;
#[cfg(test)]
mod tests;

pub use model::{AutonomousTask, TaskStatus};
pub use store::AutonomousTaskStore;
