//! MCP Manager - manages MCP server connections
//!
//! Manages both local stdio servers and remote HTTP/SSE servers using
//! the rmcp SDK. Remote servers can also be passed to the Anthropic
//! API's MCP Connector.

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::client::McpClient;
use super::config::{McpConfig, McpServerConfig, RemoteMcpServer};

/// Server status
#[derive(Debug, Clone, PartialEq)]
pub enum McpServerStatus {
    Disconnected,
    Connected,
    Error(String),
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerStatus::Disconnected => write!(f, "disconnected"),
            McpServerStatus::Connected => write!(f, "connected"),
            McpServerStatus::Error(e) => write!(f, "error: {}", e),
        }
    }
}

/// Tool definition exposed to callers (bridge from rmcp::model::Tool)
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

impl From<rmcp::model::Tool> for McpToolDef {
    fn from(tool: rmcp::model::Tool) -> Self {
        Self {
            name: tool.name.to_string(),
            description: tool.description.as_deref().map(|s| s.to_string()),
            input_schema: serde_json::to_value(&*tool.input_schema).unwrap_or(Value::Object(
                serde_json::Map::new(),
            )),
        }
    }
}

/// Tool result exposed to callers (bridge from rmcp::model::CallToolResult)
#[derive(Debug, Clone)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

/// Content types returned by MCP tools
#[derive(Debug, Clone)]
pub enum McpContent {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { uri: String, text: Option<String> },
}

impl std::fmt::Display for McpContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpContent::Text { text } => write!(f, "{}", text),
            McpContent::Image { mime_type, .. } => write!(f, "[Image: {}]", mime_type),
            McpContent::Resource { uri, text } => {
                if let Some(t) = text {
                    write!(f, "{}\n{}", uri, t)
                } else {
                    write!(f, "{}", uri)
                }
            }
        }
    }
}

/// Format MCP tool result for display
pub fn format_mcp_result(result: &McpToolResult) -> String {
    let mut formatted = String::new();
    for (idx, content) in result.content.iter().enumerate() {
        if idx > 0 {
            formatted.push('\n');
        }
        formatted.push_str(&content.to_string());
    }
    formatted
}

impl From<rmcp::model::CallToolResult> for McpToolResult {
    fn from(result: rmcp::model::CallToolResult) -> Self {
        let content = result
            .content
            .into_iter()
            .filter_map(|c| {
                use rmcp::model::RawContent;
                match c.raw {
                    RawContent::Text(text_content) => Some(McpContent::Text {
                        text: text_content.text,
                    }),
                    RawContent::Image(image_content) => Some(McpContent::Image {
                        data: image_content.data,
                        mime_type: image_content.mime_type,
                    }),
                    RawContent::Resource(embedded) => {
                        use rmcp::model::ResourceContents;
                        match embedded.resource {
                            ResourceContents::TextResourceContents { uri, text, .. } => {
                                Some(McpContent::Resource {
                                    uri: uri.to_string(),
                                    text: Some(text),
                                })
                            }
                            ResourceContents::BlobResourceContents { uri, .. } => {
                                Some(McpContent::Resource {
                                    uri: uri.to_string(),
                                    text: None,
                                })
                            }
                        }
                    }
                    _ => None,
                }
            })
            .collect();

        Self {
            content,
            is_error: result.is_error.unwrap_or(false),
        }
    }
}

/// Server info for UI
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub server_type: String, // "stdio" or "remote"
    pub status: McpServerStatus,
    pub tool_count: usize,
    pub tools: Vec<McpToolDef>,
    pub error: Option<String>,
}

/// MCP Manager
pub struct McpManager {
    /// Connected clients (both local and remote)
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    /// Server configurations
    configs: RwLock<HashMap<String, McpServerConfig>>,
    /// Remote servers (for API passthrough)
    remote_servers: RwLock<Vec<RemoteMcpServer>>,
    /// Working directory
    working_dir: PathBuf,
}

impl McpManager {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
            remote_servers: RwLock::new(Vec::new()),
            working_dir,
        }
    }

    /// Load configuration from .mcp.json
    pub async fn load_config(&self) -> Result<()> {
        let config = McpConfig::load(&self.working_dir).await?;

        let mut configs = self.configs.write().await;
        *configs = config.servers().await;

        // Store remote servers for API passthrough
        *self.remote_servers.write().await = config.remote_servers_for_api().await;

        let local_count = configs.values().filter(|c| c.is_local()).count();
        let remote_count = configs.values().filter(|c| c.is_remote()).count();

        info!(
            "Loaded MCP config: {} local, {} remote servers",
            local_count, remote_count
        );

        Ok(())
    }

    /// Connect to all configured servers in parallel
    pub async fn connect_all(&self) -> Result<()> {
        let configs: Vec<_> = {
            let configs = self.configs.read().await;
            configs
                .iter()
                .map(|(n, c)| (n.clone(), c.clone()))
                .collect()
        };

        if configs.is_empty() {
            return Ok(());
        }

        info!(
            "Connecting to {} MCP servers in parallel",
            configs.len()
        );

        // Connect to all servers in parallel
        let connect_futures: Vec<_> = configs
            .iter()
            .map(|(name, _)| {
                let name = name.clone();
                async move {
                    info!("Attempting to connect to MCP server: {}", name);
                    (name.clone(), self.connect(&name).await)
                }
            })
            .collect();

        let results = futures::future::join_all(connect_futures).await;

        for (name, result) in results {
            if let Err(e) = result {
                warn!("Failed to connect to MCP server {}: {:?}", name, e);
            }
        }

        Ok(())
    }

    /// Connect to a specific server
    pub async fn connect(&self, name: &str) -> Result<()> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(name).cloned()
        };

        let Some(config) = config else {
            return Err(anyhow::anyhow!("Unknown server: {}", name));
        };

        // Disconnect first if already connected
        self.disconnect(name).await;

        // Connect based on server type
        let client = match &config {
            McpServerConfig::Local { command, args, env } => {
                McpClient::connect_local(name, command, args, env, &self.working_dir)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?
            }
            McpServerConfig::Remote {
                url,
                authorization_token,
                ..
            } => McpClient::connect_remote(name, url, authorization_token.as_deref())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?,
        };

        // List tools to populate cache
        if let Err(e) = client.list_tools().await {
            error!("Failed to list tools from MCP server {}: {}", name, e);
        }

        let client = Arc::new(client);
        self.clients.write().await.insert(name.to_string(), client);

        info!("Connected to MCP server: {}", name);
        Ok(())
    }

    /// Disconnect from a server
    pub async fn disconnect(&self, name: &str) {
        if self.clients.write().await.remove(name).is_some() {
            info!("Disconnected from MCP server: {}", name);
        }
    }

    /// Get all tools from connected servers
    pub async fn get_all_tools(&self) -> Vec<(String, McpToolDef)> {
        let clients = self.clients.read().await;
        let mut tools = Vec::new();

        for (name, client) in clients.iter() {
            for tool in client.get_cached_tools().await {
                tools.push((name.clone(), McpToolDef::from(tool)));
            }
        }

        tools
    }

    /// Call a tool on a specific server
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<McpToolResult> {
        let clients = self.clients.read().await;
        let client = clients
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("Server not connected: {}", server))?;

        let result = client
            .call_tool(tool, arguments)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        Ok(McpToolResult::from(result))
    }

    /// List resources from a specific server
    pub async fn list_resources(&self, server: &str) -> Result<Vec<rmcp::model::Resource>> {
        let clients = self.clients.read().await;
        let client = clients
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("Server not connected: {}", server))?;
        client
            .list_resources()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Read a resource from a specific server
    pub async fn read_resource(
        &self,
        server: &str,
        uri: &str,
    ) -> Result<rmcp::model::ReadResourceResult> {
        let clients = self.clients.read().await;
        let client = clients
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("Server not connected: {}", server))?;
        client
            .read_resource(uri)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// List prompts from a specific server
    pub async fn list_prompts(&self, server: &str) -> Result<Vec<rmcp::model::Prompt>> {
        let clients = self.clients.read().await;
        let client = clients
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("Server not connected: {}", server))?;
        client
            .list_prompts()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Get a prompt from a specific server
    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<rmcp::model::GetPromptResult> {
        let clients = self.clients.read().await;
        let client = clients
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("Server not connected: {}", server))?;
        client
            .get_prompt(name, arguments)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Get server info for UI
    pub async fn list_servers(&self) -> Vec<McpServerInfo> {
        let configs = self.configs.read().await;
        let clients = self.clients.read().await;

        let mut servers = Vec::new();

        for (name, config) in configs.iter() {
            let (status, tool_count, tools, error) = if let Some(client) = clients.get(name) {
                let cached = client.get_cached_tools().await;
                let tool_defs: Vec<McpToolDef> =
                    cached.into_iter().map(McpToolDef::from).collect();
                if client.is_alive().await {
                    let count = tool_defs.len();
                    (McpServerStatus::Connected, count, tool_defs, None)
                } else {
                    (
                        McpServerStatus::Error("Process died".to_string()),
                        0,
                        Vec::new(),
                        Some("Process died".to_string()),
                    )
                }
            } else {
                (McpServerStatus::Disconnected, 0, Vec::new(), None)
            };

            servers.push(McpServerInfo {
                name: name.clone(),
                server_type: config.transport_type().to_string(),
                status,
                tool_count,
                tools,
                error,
            });
        }

        servers.sort_by(|a, b| a.name.cmp(&b.name));
        servers
    }

    /// Get remote servers for Anthropic API passthrough
    pub async fn get_remote_servers(&self) -> Vec<RemoteMcpServer> {
        self.remote_servers.read().await.clone()
    }

    /// Check if any servers are configured
    pub async fn has_servers(&self) -> bool {
        !self.configs.read().await.is_empty()
    }

    /// Get a connected client
    pub async fn get_client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }
}
