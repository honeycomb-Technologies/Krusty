//! Persistent agent memory storage
//!
//! Cross-session knowledge retention for user preferences, feedback,
//! project context, and external references. SQLite-backed with
//! multi-tenant and project-scoped support.

mod model;
mod query;
mod store;
#[cfg(test)]
mod tests;

pub use model::{AgentMemory, MemoryType};
pub use store::MemoryStore;
