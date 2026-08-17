//! Typed Codex configuration writes used by the desktop MCP server form.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeStrategy {
    Replace,
    Upsert,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigValueWriteParams {
    pub key_path: String,
    pub value: Value,
    pub merge_strategy: MergeStrategy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEdit {
    pub key_path: String,
    pub value: Value,
    pub merge_strategy: MergeStrategy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigBatchWriteParams {
    pub edits: Vec<ConfigEdit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<String>,
    #[serde(default)]
    pub reload_user_config: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigWriteStatus {
    Ok,
    OkOverridden,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigWriteResponse {
    pub status: ConfigWriteStatus,
    pub version: String,
    pub file_path: String,
    pub overridden_metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigMcpServerReloadResponse {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerTransportConfig {
    StreamableHttp { url: String },
    Stdio { command: String, args: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfigAddParams {
    pub name: String,
    pub transport: McpServerTransportConfig,
}

impl McpServerConfigAddParams {
    pub fn config_write_params(&self) -> ConfigValueWriteParams {
        let value = match &self.transport {
            McpServerTransportConfig::StreamableHttp { url } => json!({ "url": url }),
            McpServerTransportConfig::Stdio { command, args } => {
                json!({ "command": command, "args": args })
            }
        };
        ConfigValueWriteParams {
            key_path: format!("mcp_servers.{}", self.name),
            value,
            merge_strategy: MergeStrategy::Upsert,
            file_path: None,
            expected_version: None,
        }
    }
}

pub fn valid_mcp_server_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_server_serializes_to_exact_config_write_shape() {
        let params = McpServerConfigAddParams {
            name: "github-mcp".to_owned(),
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://mcp.example.test".to_owned(),
            },
        }
        .config_write_params();
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            json!({
                "keyPath": "mcp_servers.github-mcp",
                "value": {"url": "https://mcp.example.test"},
                "mergeStrategy": "upsert"
            })
        );
    }

    #[test]
    fn stdio_server_preserves_argument_boundaries() {
        let params = McpServerConfigAddParams {
            name: "local_tools".to_owned(),
            transport: McpServerTransportConfig::Stdio {
                command: "npx".to_owned(),
                args: vec!["-y".to_owned(), "@example/mcp server".to_owned()],
            },
        }
        .config_write_params();
        assert_eq!(
            params.value,
            json!({"command": "npx", "args": ["-y", "@example/mcp server"]})
        );
    }

    #[test]
    fn server_names_match_codex_cli_validation() {
        assert!(valid_mcp_server_name("github-mcp_2"));
        assert!(!valid_mcp_server_name(""));
        assert!(!valid_mcp_server_name("github.mcp"));
        assert!(!valid_mcp_server_name("github mcp"));
    }

    #[test]
    fn batch_write_matches_generated_atomic_edit_shape() {
        let params = ConfigBatchWriteParams {
            edits: vec![ConfigEdit {
                key_path: "features.network_proxy".to_owned(),
                value: Value::Bool(true),
                merge_strategy: MergeStrategy::Upsert,
            }],
            file_path: None,
            expected_version: None,
            reload_user_config: true,
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            json!({
                "edits": [{
                    "keyPath": "features.network_proxy",
                    "value": true,
                    "mergeStrategy": "upsert"
                }],
                "reloadUserConfig": true
            })
        );
    }
}
