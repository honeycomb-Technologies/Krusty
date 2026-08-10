//! Typed Codex app-server plugin mutation contracts.

use serde::{Deserialize, Serialize};

use crate::PluginAuthPolicy;

/// Parameters for `plugin/install`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallParams {
    pub plugin_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_marketplace_name: Option<String>,
}

impl PluginInstallParams {
    pub fn named(plugin_name: impl Into<String>) -> Self {
        Self {
            plugin_name: plugin_name.into(),
            marketplace_path: None,
            remote_marketplace_name: None,
        }
    }
}

/// App metadata returned when installing a plugin requires follow-up auth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginAppSummary {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallResponse {
    pub apps_needing_auth: Vec<PluginAppSummary>,
    pub auth_policy: PluginAuthPolicy,
}

/// Parameters for `plugin/uninstall`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginUninstallParams {
    pub plugin_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginUninstallResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_params_match_canonical_wire_shape() {
        assert_eq!(
            serde_json::to_value(PluginInstallParams::named("documents")).unwrap(),
            serde_json::json!({"pluginName": "documents"})
        );
    }

    #[test]
    fn install_response_parses_auth_follow_up() {
        let response: PluginInstallResponse = serde_json::from_value(serde_json::json!({
            "appsNeedingAuth": [{
                "id": "drive",
                "name": "Google Drive",
                "installUrl": "https://example.test/install"
            }],
            "authPolicy": "ON_INSTALL"
        }))
        .unwrap();
        assert_eq!(response.auth_policy, PluginAuthPolicy::OnInstall);
        assert_eq!(response.apps_needing_auth[0].id, "drive");
    }
}
