//! MCP (Model Context Protocol) client implementation using the official rmcp SDK
//!
//! Supports two types of MCP servers:
//! - Local (stdio): We spawn the process and communicate via stdin/stdout
//! - Remote (HTTP/SSE): We connect via Streamable HTTP transport
//!
//! Local and remote servers are both managed here. Connector-ready remote
//! descriptors are retained for future provider integrations, but current MCP
//! calls are executed through Mitsuro's own client manager.

mod client;
mod config;
mod manager;
mod oauth;
mod stdio_transport;
pub mod tool;
mod transport;

pub use config::{
    McpConfig, McpConfigSource, McpConnectionAuthority, McpOAuthConfig, McpPackageConfig,
    McpServerConfig, McpServerOptions, McpToolApproval, McpToolRules, RemoteMcpServer,
};
pub use manager::{
    format_mcp_result, McpContent, McpManager, McpServerInfo, McpServerStatus, McpToolDef,
    McpToolResult,
};
pub use oauth::{McpOAuthStart, McpOAuthState, McpOAuthStatus};
pub use tool::McpTool;
