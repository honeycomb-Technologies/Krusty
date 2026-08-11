//! Typed Codex app-server plugin skill and sharing contracts.

use serde::{Deserialize, Serialize};

use crate::PluginSummary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSkillReadParams {
    pub remote_marketplace_name: String,
    pub remote_plugin_id: String,
    pub skill_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSkillReadResponse {
    pub contents: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginShareDiscoverability {
    Listed,
    Unlisted,
    Private,
}

pub type PluginShareUpdateDiscoverability = PluginShareDiscoverability;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginSharePrincipalRole {
    Reader,
    Editor,
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginSharePrincipalType {
    User,
    Group,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginShareTargetRole {
    Reader,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSharePrincipal {
    pub principal_type: PluginSharePrincipalType,
    pub principal_id: String,
    pub role: PluginSharePrincipalRole,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareTarget {
    pub principal_type: PluginSharePrincipalType,
    pub principal_id: String,
    pub role: PluginShareTargetRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareContext {
    pub remote_plugin_id: String,
    pub remote_version: Option<String>,
    pub discoverability: Option<PluginShareDiscoverability>,
    pub share_url: Option<String>,
    pub creator_account_user_id: Option<String>,
    pub creator_name: Option<String>,
    pub share_principals: Option<Vec<PluginSharePrincipal>>,
    pub can_publish_to_workspace: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareListItem {
    pub plugin: PluginSummary,
    pub local_plugin_path: Option<String>,
}

impl PluginShareListItem {
    /// Decode the generated `PluginSummary.shareContext` field retained by the
    /// shared catalog model without forcing the catalog itself to depend on
    /// plugin-sharing policy types.
    pub fn share_context(&self) -> serde_json::Result<Option<PluginShareContext>> {
        match self.plugin.extra.get("shareContext") {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => serde_json::from_value(value.clone()).map(Some),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginShareListParams {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareListResponse {
    pub data: Vec<PluginShareListItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareDeleteParams {
    pub remote_plugin_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginShareDeleteResponse {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareSaveParams {
    pub plugin_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discoverability: Option<PluginShareDiscoverability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_targets: Option<Vec<PluginShareTarget>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareSaveResponse {
    pub remote_plugin_id: String,
    pub share_url: String,
    pub can_publish_to_workspace: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareUpdateTargetsParams {
    pub remote_plugin_id: String,
    pub discoverability: PluginShareUpdateDiscoverability,
    pub share_targets: Vec<PluginShareTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginShareUpdateTargetsResponse {
    pub principals: Vec<PluginSharePrincipal>,
    pub discoverability: PluginShareDiscoverability,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_contracts_use_generated_enum_and_optional_shapes() {
        let target = PluginShareTarget {
            principal_type: PluginSharePrincipalType::Workspace,
            principal_id: "workspace-1".to_owned(),
            role: PluginShareTargetRole::Editor,
        };
        let params = PluginShareSaveParams {
            plugin_path: "/tmp/plugin".to_owned(),
            remote_plugin_id: None,
            discoverability: Some(PluginShareDiscoverability::Unlisted),
            share_targets: Some(vec![target]),
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({
                "pluginPath": "/tmp/plugin",
                "discoverability": "UNLISTED",
                "shareTargets": [{
                    "principalType": "workspace",
                    "principalId": "workspace-1",
                    "role": "editor"
                }]
            })
        );
        assert_eq!(
            serde_json::to_value(PluginShareListParams::default()).unwrap(),
            serde_json::json!({})
        );
    }
}
