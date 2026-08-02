//! Agent state tracking storage
//!
//! Handles agent execution state for sessions (for background execution).

mod model;
mod store;
#[cfg(test)]
mod tests;

pub use model::AgentState;
pub use store::AgentStateStore;
