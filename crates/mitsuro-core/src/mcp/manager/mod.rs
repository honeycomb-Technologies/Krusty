//! MCP Manager - manages MCP server connections.
//!
//! Manages both local stdio servers and remote HTTP/SSE servers using the rmcp
//! SDK. Connector-shaped remote descriptors remain available to callers, but
//! no provider request path consumes them today.

mod runtime;
mod types;

pub use runtime::McpManager;
pub use types::{
    format_mcp_result, McpContent, McpServerInfo, McpServerStatus, McpToolDef, McpToolResult,
};
