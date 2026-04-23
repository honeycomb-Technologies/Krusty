//! Hook system for tool execution
//!
//! Allows intercepting tool calls before and after execution
//! for logging, validation, and safety.
//!
//! ## Built-in Hooks
//! - `SafetyHook` - Blocks dangerous bash commands (rm -rf, sudo, etc.)
//! - `LoggingHook` - Logs all tool executions with timing
//!
//! ## Custom Hooks
//! Implement `PreToolHook` or `PostToolHook` traits for custom behavior.

mod builtins;
mod shell_policy;

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::tools::registry::{ToolContext, ToolResult};

pub use builtins::{LoggingHook, PlanModeHook, SafetyHook};

/// Result of a hook execution
#[derive(Debug)]
pub enum HookResult {
    /// Continue with execution (no changes)
    Continue,
    /// Block execution with a reason
    Block { reason: String },
}

/// Hook called before tool execution
#[async_trait]
pub trait PreToolHook: Send + Sync {
    /// Called before a tool executes
    ///
    /// Returns:
    /// - `Continue` to proceed normally
    /// - `Block { reason }` to prevent execution
    async fn before_execute(&self, name: &str, params: &Value, ctx: &ToolContext) -> HookResult;
}

/// Hook called after tool execution
#[async_trait]
pub trait PostToolHook: Send + Sync {
    /// Called after a tool executes
    ///
    /// Can inspect the result and duration but typically just logs.
    /// Returns `HookResult` for potential future use (result modification).
    async fn after_execute(
        &self,
        name: &str,
        params: &Value,
        result: &ToolResult,
        duration: Duration,
    ) -> HookResult;
}
