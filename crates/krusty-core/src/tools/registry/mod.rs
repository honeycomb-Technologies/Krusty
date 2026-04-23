//! Tool registry for managing available tools.
//!
//! Supports pre/post execution hooks for logging, validation, and safety.

mod context;
mod policy;
mod result;
mod runtime;

pub use context::{ToolContext, ToolOutputChunk};
pub use policy::{
    tool_category, tool_policy, DelegationPolicy, DelegationSurface, PermissionMode, ToolCategory,
    ToolPolicy,
};
pub use result::{parse_params, ToolResult};
pub use runtime::{Tool, ToolRegistry};

#[cfg(test)]
mod tests;
