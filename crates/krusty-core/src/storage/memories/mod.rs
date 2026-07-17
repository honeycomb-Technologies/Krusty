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

pub use model::{
    is_compaction_flush_memory, AgentMemory, AgentMemoryRevision, CanonicalMemoryInput,
    MemoryNamespace, MemoryRevisionEvent, MemorySensitivity, MemorySource, MemoryStatus,
    MemoryType, COMPACTION_FLUSH_TITLE_PREFIX,
};
pub use store::MemoryStore;
