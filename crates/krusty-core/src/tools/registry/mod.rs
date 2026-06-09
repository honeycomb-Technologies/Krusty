//! Tool registry for managing available tools.
//!
//! Supports pre/post execution hooks for logging, validation, and safety.

mod context;
mod policy;
mod result;
mod runtime;

pub use context::{FileObservationTracker, FilesystemAccess, ToolContext, ToolOutputChunk};
pub use policy::{
    authorize_tool_call, tool_category, tool_policy, tool_policy_for_call, DelegationPolicy,
    DelegationSurface, PermissionMode, ToolAuthorization, ToolCategory, ToolPolicy,
};
pub use result::{parse_params, ToolResult};
pub use runtime::{Tool, ToolRegistry};

#[cfg(test)]
mod tests;
