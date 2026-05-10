use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use super::expansion::expand_env_var;
use super::types::{McpConfig, McpServerConfig, McpServerConfigRaw, RemoteMcpServer};

impl McpConfig {
    /// Load config from .mcp.json in project root.
    pub async fn load(working_dir: &Path) -> Result<Self> {
        let config_path = working_dir.join(".mcp.json");

        if !config_path.exists() {
            tracing::debug!("No .mcp.json found at {:?}", config_path);
            return Ok(Self::default());
        }

        let content = tokio::fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("Failed to read {:?}", config_path))?;

        let config: McpConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {:?}", config_path))?;

        tracing::info!(
            "Loaded MCP config with {} servers from {:?}",
            config.mcp_servers.len(),
            config_path
        );

        Ok(config)
    }

    /// Get resolved server configurations
    pub async fn servers(&self) -> HashMap<String, McpServerConfig> {
        let mut result = HashMap::new();
        for (name, raw) in &self.mcp_servers {
            let config = match raw {
                McpServerConfigRaw::Local { command, args, env } => {
                    let mut expanded_env = HashMap::new();
                    for (k, v) in env {
                        expanded_env.insert(k.clone(), expand_env_var(v).await);
                    }
                    McpServerConfig::Local {
                        command: command.clone(),
                        args: args.clone(),
                        env: expanded_env,
                    }
                }
                McpServerConfigRaw::Remote {
                    url,
                    transport,
                    authorization_token,
                    ..
                } => {
                    let token = match authorization_token {
                        Some(t) => Some(expand_env_var(t).await),
                        None => None,
                    };
                    McpServerConfig::Remote {
                        url: url.clone(),
                        transport: transport.clone(),
                        authorization_token: token,
                    }
                }
            };
            result.insert(name.clone(), config);
        }
        result
    }

    /// Get remote servers formatted for Anthropic API's MCP Connector
    pub async fn remote_servers_for_api(&self) -> Vec<RemoteMcpServer> {
        let mut result = Vec::new();
        for (name, raw) in &self.mcp_servers {
            if let McpServerConfigRaw::Remote {
                url,
                authorization_token,
                ..
            } = raw
            {
                let token = match authorization_token {
                    Some(t) => Some(expand_env_var(t).await),
                    None => None,
                };
                result.push(RemoteMcpServer {
                    server_type: "url".to_string(),
                    url: url.clone(),
                    name: name.clone(),
                    authorization_token: token,
                });
            }
        }
        result
    }
}
