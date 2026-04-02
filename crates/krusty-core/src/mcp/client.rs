//! MCP Client wrapper around the official rmcp SDK
//!
//! Provides a unified client for both local (stdio) and remote (HTTP/SSE)
//! MCP servers using the rmcp crate's transport layer.

use std::collections::HashMap;
use std::path::Path;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, GetPromptRequestParams,
    GetPromptResult, Implementation, InitializeRequestParams, ReadResourceRequestParams,
    ReadResourceResult,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransportConfig, StreamableHttpClientWorker,
};
use rmcp::transport::TokioChildProcess;
use tokio::sync::RwLock;
use tracing::{debug, info};

use super::transport::ReqwestStreamableHttpClient;

/// MCP client wrapping an rmcp RunningService
pub struct McpClient {
    name: String,
    /// The running rmcp service
    service: RunningService<RoleClient, InitializeRequestParams>,
    /// Cached tools from last list_tools call
    cached_tools: RwLock<Vec<rmcp::model::Tool>>,
    /// Current connection status
    status: RwLock<McpClientStatus>,
}

/// Connection status for an MCP client
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum McpClientStatus {
    Connected,
    Disconnected,
    Error,
}

/// Errors from MCP client operations
#[derive(Debug, thiserror::Error)]
pub enum McpClientError {
    #[error("Failed to spawn MCP server: {0}")]
    Spawn(String),
    #[error("Failed to initialize MCP connection: {0}")]
    Init(String),
    #[error("MCP request failed: {0}")]
    Request(String),
}

impl McpClient {
    /// Connect to a local stdio MCP server
    pub async fn connect_local(
        name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        working_dir: &Path,
    ) -> Result<Self, McpClientError> {
        info!(
            "Connecting to local MCP server: {} (cmd: {})",
            name, command
        );

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.current_dir(working_dir);

        let process =
            TokioChildProcess::new(cmd).map_err(|e| McpClientError::Spawn(e.to_string()))?;

        let client_info = client_info(name);

        let service = rmcp::serve_client(client_info, process)
            .await
            .map_err(|e| McpClientError::Init(e.to_string()))?;

        info!("Connected to local MCP server: {}", name);

        Ok(Self {
            name: name.to_string(),
            service,
            cached_tools: RwLock::new(Vec::new()),
            status: RwLock::new(McpClientStatus::Connected),
        })
    }

    /// Connect to a remote HTTP/SSE MCP server
    pub async fn connect_remote(
        name: &str,
        url: &str,
        auth_token: Option<&str>,
    ) -> Result<Self, McpClientError> {
        info!("Connecting to remote MCP server: {} (url: {})", name, url);

        let mut config = StreamableHttpClientTransportConfig::with_uri(url);
        if let Some(token) = auth_token {
            config.auth_header = Some(format!("Bearer {}", token));
        }
        // Allow stateless servers that don't return session IDs
        config.allow_stateless = true;

        let http_client = ReqwestStreamableHttpClient::new();
        let worker = StreamableHttpClientWorker::new(http_client, config);

        let client_info = client_info(name);

        let service = rmcp::serve_client(client_info, worker)
            .await
            .map_err(|e| McpClientError::Init(e.to_string()))?;

        info!("Connected to remote MCP server: {}", name);

        Ok(Self {
            name: name.to_string(),
            service,
            cached_tools: RwLock::new(Vec::new()),
            status: RwLock::new(McpClientStatus::Connected),
        })
    }

    /// Get the server name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// List all available tools from this server
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>, McpClientError> {
        let tools = self
            .service
            .peer()
            .list_all_tools()
            .await
            .map_err(|e| McpClientError::Request(e.to_string()))?;

        info!("MCP {} has {} tools", self.name, tools.len());
        for tool in &tools {
            debug!(
                "MCP {} tool '{}': {}",
                self.name,
                tool.name,
                tool.description.as_deref().unwrap_or("<no description>")
            );
        }

        *self.cached_tools.write().await = tools.clone();
        Ok(tools)
    }

    /// Call a tool on this server
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, McpClientError> {
        let mut params = CallToolRequestParams::new(name.to_string());
        if let Some(obj) = arguments.as_object() {
            params = params.with_arguments(obj.clone());
        }

        self.service
            .peer()
            .call_tool(params)
            .await
            .map_err(|e| McpClientError::Request(e.to_string()))
    }

    /// List all available resources from this server
    pub async fn list_resources(&self) -> Result<Vec<rmcp::model::Resource>, McpClientError> {
        self.service
            .peer()
            .list_all_resources()
            .await
            .map_err(|e| McpClientError::Request(e.to_string()))
    }

    /// Read a specific resource from this server
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, McpClientError> {
        self.service
            .peer()
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|e| McpClientError::Request(e.to_string()))
    }

    /// List all available prompts from this server
    pub async fn list_prompts(&self) -> Result<Vec<rmcp::model::Prompt>, McpClientError> {
        self.service
            .peer()
            .list_all_prompts()
            .await
            .map_err(|e| McpClientError::Request(e.to_string()))
    }

    /// Get a specific prompt from this server
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<GetPromptResult, McpClientError> {
        let mut params = GetPromptRequestParams::new(name);
        if let Some(args) = arguments.and_then(|v| v.as_object().cloned()) {
            params = params.with_arguments(args);
        }

        self.service
            .peer()
            .get_prompt(params)
            .await
            .map_err(|e| McpClientError::Request(e.to_string()))
    }

    /// Get cached tools from last list_tools call
    pub async fn get_cached_tools(&self) -> Vec<rmcp::model::Tool> {
        self.cached_tools.read().await.clone()
    }

    /// Get current connection status
    pub async fn status(&self) -> McpClientStatus {
        *self.status.read().await
    }

    /// Check if this client is connected by testing the actual connection.
    /// Sends a lightweight ping if supported, otherwise checks cached status.
    pub async fn is_alive(&self) -> bool {
        if *self.status.read().await != McpClientStatus::Connected {
            return false;
        }
        // Try a lightweight operation to verify the connection is still live.
        // list_tools is cached on the server side so it's cheap.
        match self.service.peer().list_tools(None).await {
            Ok(_) => true,
            Err(_) => {
                *self.status.write().await = McpClientStatus::Error;
                false
            }
        }
    }
}

/// Build the client info for rmcp initialization
fn client_info(name: &str) -> InitializeRequestParams {
    InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new(format!("krusty-mcp-{}", name), env!("CARGO_PKG_VERSION")),
    )
}
