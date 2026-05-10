use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::types::{McpServerInfo, McpServerStatus, McpToolDef, McpToolResult};
use crate::mcp::client::McpClient;
use crate::mcp::config::{McpConfig, McpServerConfig, RemoteMcpServer};

/// MCP Manager
pub struct McpManager {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    configs: RwLock<HashMap<String, McpServerConfig>>,
    remote_servers: RwLock<Vec<RemoteMcpServer>>,
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

    pub async fn load_config(&self) -> Result<()> {
        let config = McpConfig::load(&self.working_dir).await?;

        let mut configs = self.configs.write().await;
        *configs = config.servers().await;
        *self.remote_servers.write().await = config.remote_servers_for_api().await;

        let local_count = configs.values().filter(|c| c.is_local()).count();
        let remote_count = configs.values().filter(|c| c.is_remote()).count();

        info!(
            "Loaded MCP config: {} local, {} remote servers",
            local_count, remote_count
        );

        Ok(())
    }

    pub async fn connect_all(&self) -> Result<()> {
        let configs: Vec<_> = {
            let configs = self.configs.read().await;
            configs
                .iter()
                .filter(|(_, c)| c.should_auto_connect())
                .map(|(n, c)| (n.clone(), c.clone()))
                .collect()
        };

        if configs.is_empty() {
            return Ok(());
        }

        info!(
            "Auto-connecting to {} trusted MCP servers in parallel",
            configs.len()
        );

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

    pub async fn connect(&self, name: &str) -> Result<()> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(name).cloned()
        };

        let Some(config) = config else {
            return Err(anyhow::anyhow!("Unknown server: {}", name));
        };

        self.disconnect(name).await;

        let client = match &config {
            McpServerConfig::Local {
                command, args, env, ..
            } => McpClient::connect_local(name, command, args, env, &self.working_dir)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?,
            McpServerConfig::Remote {
                url,
                authorization_token,
                ..
            } => McpClient::connect_remote(name, url, authorization_token.as_deref())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?,
        };

        if let Err(e) = client.list_tools().await {
            error!("Failed to list tools from MCP server {}: {}", name, e);
        }

        let client = Arc::new(client);
        self.clients.write().await.insert(name.to_string(), client);

        info!("Connected to MCP server: {}", name);
        Ok(())
    }

    pub async fn disconnect(&self, name: &str) {
        if self.clients.write().await.remove(name).is_some() {
            info!("Disconnected from MCP server: {}", name);
        }
    }

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

    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
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

    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Option<serde_json::Value>,
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

    pub async fn list_servers(&self) -> Vec<McpServerInfo> {
        let configs = self.configs.read().await;
        let clients = self.clients.read().await;

        let mut servers = Vec::new();

        for (name, config) in configs.iter() {
            let (status, tool_count, tools, error) = if let Some(client) = clients.get(name) {
                let cached = client.get_cached_tools().await;
                let tool_defs: Vec<McpToolDef> = cached.into_iter().map(McpToolDef::from).collect();
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

    pub async fn get_remote_servers(&self) -> Vec<RemoteMcpServer> {
        self.remote_servers.read().await.clone()
    }

    pub async fn has_servers(&self) -> bool {
        !self.configs.read().await.is_empty()
    }

    pub async fn get_client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }
}
