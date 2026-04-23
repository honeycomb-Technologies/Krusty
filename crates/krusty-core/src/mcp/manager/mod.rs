//! MCP Manager - manages MCP server connections.
//!
//! Manages both local stdio servers and remote HTTP/SSE servers using
//! the rmcp SDK. Remote servers can also be passed to the Anthropic
//! API's MCP Connector.

mod runtime;
mod types;

pub use runtime::McpManager;
pub use types::{
    format_mcp_result, McpContent, McpServerInfo, McpServerStatus, McpToolDef, McpToolResult,
};
