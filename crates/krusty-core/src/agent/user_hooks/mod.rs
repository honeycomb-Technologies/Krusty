//! User-configurable hooks system.
//!
//! Allows users to define custom hooks that execute shell commands
//! before/after tool execution. Hooks can block, warn, or silently proceed
//! based on exit codes.

mod adapters;
mod executor;
mod manager;
mod model;

pub use self::adapters::{UserPostToolHook, UserPreToolHook};
pub use self::executor::UserHookExecutor;
pub use self::manager::UserHookManager;
pub use self::model::{UserHook, UserHookResult, UserHookType};
