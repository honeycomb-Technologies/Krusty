//! MCP configuration parsing.
//!
//! Parses `.mcp.json` files. Supports two server types:
//! - Local (stdio): Spawns a local process, we act as MCP client.
//! - Remote (url): Passed to Anthropic API's MCP Connector.

mod expansion;
mod loader;
#[cfg(test)]
mod tests;
mod types;

pub use types::{
    McpConfig, McpConfigSource, McpConnectionAuthority, McpOAuthConfig, McpPackageConfig,
    McpServerConfig, McpServerOptions, McpToolApproval, McpToolRules, RemoteMcpServer,
};
