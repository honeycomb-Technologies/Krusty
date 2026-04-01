//! MCP (Model Context Protocol) client implementation using the official rmcp SDK
//!
//! Supports two types of MCP servers:
//! - Local (stdio): We spawn the process and communicate via stdin/stdout
//! - Remote (HTTP/SSE): We connect via Streamable HTTP transport
//!
//! Local and remote servers are both managed here. Remote servers can
//! additionally be passed to the Anthropic API's MCP Connector.

mod client;
mod config;
mod manager;
pub mod tool;
mod transport;

pub use config::{McpConfig, McpServerConfig, RemoteMcpServer};
pub use manager::{
    format_mcp_result, McpContent, McpManager, McpServerInfo, McpServerStatus, McpToolDef,
    McpToolResult,
};
pub use tool::McpTool;
