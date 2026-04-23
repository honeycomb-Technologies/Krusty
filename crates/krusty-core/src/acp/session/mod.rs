//! ACP session management.
//!
//! Keeps per-session runtime state separate from the registry that owns all ACP
//! sessions, while preserving the existing public ACP session API.

mod manager;
mod state;
#[cfg(test)]
mod tests;

pub use manager::SessionManager;
pub use state::{SessionState, StorageHandle};
