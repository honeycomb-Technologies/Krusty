//! MCP configuration parsing
//!
//! Parses .mcp.json files. Supports two server types:
//! - Local (stdio): Spawns a local process, we act as MCP client
//! - Remote (url): Passed to Anthropic API's MCP Connector

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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
        server_type: String, // Must be "url"
        url: String,
        #[serde(default)]
        transport: Option<String>, // "sse", "http", "streamable-http" — default "streamable-http"
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
        transport: Option<String>, // "sse", "http", "streamable-http" — default "streamable-http"
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

            // User configs override built-ins
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

/// Expand ${VAR} environment variables, with fallback to credentials store
async fn expand_env_var(s: &str) -> String {
    let mut result = s.to_string();

    while let Some(start) = result.find("${") {
        if let Some(end_offset) = result[start..].find('}') {
            let end = start + end_offset;
            let var_name = &result[start + 2..end];
            tracing::debug!("Expanding env var: {}", var_name);

            // Try environment variable first
            let value = match std::env::var(var_name) {
                Ok(v) => {
                    tracing::debug!("Found {} in environment", var_name);
                    v
                }
                Err(_) => {
                    // Fall back to credentials store
                    if let Some(cred_key) = credential_key_for_env(var_name) {
                        tracing::debug!(
                            "Looking up {} in credential store as '{}'",
                            var_name,
                            cred_key
                        );
                        match get_credential(cred_key).await {
                            Some(v) => {
                                tracing::debug!(
                                    "Found {} in credential store (len={})",
                                    var_name,
                                    v.len()
                                );
                                v
                            }
                            None => {
                                tracing::warn!("Credential '{}' not found in store", cred_key);
                                String::new()
                            }
                        }
                    } else {
                        tracing::warn!("No credential mapping for {}", var_name);
                        String::new()
                    }
                }
            };

            result.replace_range(start..end + 1, &value);
        } else {
            break;
        }
    }

    result
}

fn credential_key_for_env(env_name: &str) -> Option<&'static str> {
    match env_name {
        "ANTHROPIC_API_KEY" => Some("anthropic"),
        "MINIMAX_API_KEY" => Some("minimax"),
        "OPENROUTER_API_KEY" => Some("openrouter"),
        "OPENAI_API_KEY" => Some("openai"),
        _ => None,
    }
}

/// Get a credential from the credentials store (async to avoid blocking)
async fn get_credential(provider: &str) -> Option<String> {
    let path = crate::paths::config_dir()
        .join("tokens")
        .join("credentials.json");
    tracing::debug!("Looking for credentials at {:?}", path);
    if !path.exists() {
        tracing::warn!("Credentials file not found at {:?}", path);
        return None;
    }
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read credentials: {}", e);
            return None;
        }
    };
    let creds: HashMap<String, String> = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to parse credentials: {}", e);
            return None;
        }
    };
    let result = creds.get(provider).cloned();
    if result.is_some() {
        tracing::debug!("Found credential for '{}'", provider);
    } else {
        let mut available = String::new();
        for key in creds.keys() {
            if !available.is_empty() {
                available.push_str(", ");
            }
            available.push_str(key);
        }
        tracing::warn!(
            "No credential found for '{}' (available: [{}])",
            provider,
            available
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_local_server() {
        let json = r#"{
            "mcpServers": {
                "minimax": {
                    "command": "uvx",
                    "args": ["minimax-coding-plan-mcp", "-y"],
                    "env": {"MINIMAX_API_KEY": "test"}
                }
            }
        }"#;

        let config: McpConfig = serde_json::from_str(json).unwrap();
        let servers = config.servers().await;
        assert!(matches!(
            servers.get("minimax"),
            Some(McpServerConfig::Local { .. })
        ));
    }

    #[tokio::test]
    async fn test_parse_remote_server() {
        let json = r#"{
            "mcpServers": {
                "remote": {
                    "type": "url",
                    "url": "https://mcp.example.com/sse",
                    "authorization_token": "token123"
                }
            }
        }"#;

        let config: McpConfig = serde_json::from_str(json).unwrap();
        let servers = config.servers().await;
        assert!(matches!(
            servers.get("remote"),
            Some(McpServerConfig::Remote { .. })
        ));
    }

    #[tokio::test]
    async fn test_expand_env_var() {
        // Test that direct values pass through
        assert_eq!(
            expand_env_var("https://api.example.com").await,
            "https://api.example.com"
        );

        // Test env var expansion with fallback to credentials
        // This would need a real credential file to fully test
    }
}
