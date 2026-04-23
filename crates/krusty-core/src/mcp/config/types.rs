use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// MCP configuration from .mcp.json
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfigRaw>,
}

/// Raw server configuration from JSON
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfigRaw {
    /// Local server (spawns process, stdio transport)
    Local {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// Remote server (passed to Anthropic MCP Connector API)
    Remote {
        #[serde(rename = "type")]
        server_type: String,
        url: String,
        #[serde(default)]
        transport: Option<String>,
        #[serde(default)]
        authorization_token: Option<String>,
    },
}

/// Resolved server configuration
#[derive(Debug, Clone)]
pub enum McpServerConfig {
    /// Local server - we spawn and manage the process
    Local {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    /// Remote server - we connect directly via HTTP/SSE, or pass to Anthropic API
    Remote {
        url: String,
        transport: Option<String>,
        authorization_token: Option<String>,
    },
}

/// Remote server config for Anthropic API
#[derive(Debug, Clone, Serialize)]
pub struct RemoteMcpServer {
    #[serde(rename = "type")]
    pub server_type: String,
    pub url: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_token: Option<String>,
}

impl McpServerConfig {
    pub fn is_local(&self) -> bool {
        matches!(self, McpServerConfig::Local { .. })
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, McpServerConfig::Remote { .. })
    }

    pub fn transport_type(&self) -> &'static str {
        match self {
            McpServerConfig::Local { .. } => "stdio",
            McpServerConfig::Remote { .. } => "remote",
        }
    }
}
