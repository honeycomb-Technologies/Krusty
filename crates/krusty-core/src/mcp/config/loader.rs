use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use super::expansion::expand_env_var;
use super::types::{McpConfig, McpServerConfig, McpServerConfigRaw, RemoteMcpServer};

impl McpConfig {
    /// Built-in MCP servers included by default.
    /// User configs in `.mcp.json` override these by name.
    fn builtin_servers() -> HashMap<String, McpServerConfigRaw> {
        HashMap::from([(
            "minimax".to_string(),
            McpServerConfigRaw::Local {
                command: "uvx".to_string(),
                args: vec!["minimax-coding-plan-mcp".to_string(), "-y".to_string()],
                env: HashMap::from([
                    (
                        "MINIMAX_API_KEY".to_string(),
                        "${MINIMAX_API_KEY}".to_string(),
                    ),
                    (
                        "MINIMAX_API_HOST".to_string(),
                        "https://api.minimax.io".to_string(),
                    ),
                ]),
            },
        )])
    }

    /// Load config from .mcp.json in project root, merged with built-in defaults.
    pub async fn load(working_dir: &Path) -> Result<Self> {
        let mut servers = Self::builtin_servers();

        let config_path = working_dir.join(".mcp.json");

        if config_path.exists() {
            let content = tokio::fs::read_to_string(&config_path)
                .await
                .with_context(|| format!("Failed to read {:?}", config_path))?;

            let user_config: McpConfig = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse {:?}", config_path))?;

            tracing::info!(
                "Loaded MCP config with {} servers from {:?}",
                user_config.mcp_servers.len(),
                config_path
            );

            servers.extend(user_config.mcp_servers);
        } else {
            tracing::debug!("No .mcp.json found at {:?}", config_path);
        }

        Ok(Self {
            mcp_servers: servers,
        })
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
