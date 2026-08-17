//! Typed Codex experimental-feature catalog and runtime enablement contracts.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExperimentalFeatureStage {
    Beta,
    UnderDevelopment,
    Stable,
    Deprecated,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalFeature {
    pub name: String,
    pub stage: ExperimentalFeatureStage,
    pub enabled: bool,
    pub default_enabled: bool,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub announcement: Option<String>,
}

impl ExperimentalFeature {
    pub fn is_user_facing_beta(&self) -> bool {
        self.stage == ExperimentalFeatureStage::Beta
            && self
                .display_name
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && self
                .description
                .as_deref()
                .is_some_and(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalFeatureListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalFeatureListResponse {
    pub data: Vec<ExperimentalFeature>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalFeatureEnablementSetParams {
    pub enablement: HashMap<String, bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalFeatureEnablementSetResponse {
    pub enablement: HashMap<String, bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_catalog_shape_is_user_facing_only_with_copy() {
        let feature: ExperimentalFeature = serde_json::from_value(serde_json::json!({
            "name": "network_proxy",
            "stage": "beta",
            "displayName": "Network proxy",
            "description": "Apply proxy restrictions.",
            "announcement": "New",
            "enabled": false,
            "defaultEnabled": false
        }))
        .unwrap();
        assert!(feature.is_user_facing_beta());
        assert_eq!(
            serde_json::to_value(ExperimentalFeatureListParams::default()).unwrap(),
            serde_json::json!({})
        );
    }
}
