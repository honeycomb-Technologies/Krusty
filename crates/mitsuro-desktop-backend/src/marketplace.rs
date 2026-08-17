//! Typed Codex app-server marketplace-management contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceAddParams {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sparse_paths: Option<Vec<String>>,
}

impl MarketplaceAddParams {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            ref_name: None,
            sparse_paths: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceAddResponse {
    pub marketplace_name: String,
    pub installed_root: String,
    pub already_added: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceRemoveParams {
    pub marketplace_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceRemoveResponse {
    pub marketplace_name: String,
    pub installed_root: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceUpgradeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceUpgradeErrorInfo {
    pub marketplace_name: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceUpgradeResponse {
    pub selected_marketplaces: Vec<String>,
    pub upgraded_roots: Vec<String>,
    pub errors: Vec<MarketplaceUpgradeErrorInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_contracts_match_generated_wire_shape() {
        let mut params = MarketplaceAddParams::new("https://example.test/plugins.git");
        params.ref_name = Some("main".to_owned());
        params.sparse_paths = Some(vec!["plugins/docs".to_owned()]);
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({
                "source": "https://example.test/plugins.git",
                "refName": "main",
                "sparsePaths": ["plugins/docs"]
            })
        );
        assert_eq!(
            serde_json::to_value(MarketplaceUpgradeParams::default()).unwrap(),
            serde_json::json!({})
        );
    }
}
