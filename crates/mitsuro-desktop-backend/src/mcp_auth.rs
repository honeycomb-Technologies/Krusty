//! Typed Codex app-server MCP OAuth contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerOauthLoginParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<i64>,
}

impl McpServerOauthLoginParams {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scopes: None,
            thread_id: None,
            timeout_secs: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerOauthLoginResponse {
    pub authorization_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerOauthLoginCompleted {
    pub name: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_params_match_generated_wire_shape() {
        assert_eq!(
            serde_json::to_value(McpServerOauthLoginParams::new("github")).unwrap(),
            serde_json::json!({"name": "github"})
        );
    }

    #[test]
    fn completion_notification_matches_generated_shape() {
        let completion: McpServerOauthLoginCompleted = serde_json::from_value(
            serde_json::json!({"name": "github", "success": false, "error": "denied"}),
        )
        .unwrap();
        assert_eq!(completion.name, "github");
        assert!(!completion.success);
        assert_eq!(completion.error.as_deref(), Some("denied"));
    }
}
