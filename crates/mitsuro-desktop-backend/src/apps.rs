//! Typed Codex app and connector catalog contracts.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsListParams {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force_refetch: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsInstalledParams {
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force_refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub id: String,
    pub runtime_name: Option<String>,
    pub enabled: bool,
    pub callable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsInstalledResponse {
    pub apps: Vec<InstalledApp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppBranding {
    pub category: Option<String>,
    pub developer: Option<String>,
    pub website: Option<String>,
    pub privacy_policy: Option<String>,
    pub terms_of_service: Option<String>,
    pub is_discoverable_app: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppReview {
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppScreenshot {
    pub url: Option<String>,
    #[serde(alias = "file_id")]
    pub file_id: Option<String>,
    #[serde(alias = "user_prompt")]
    pub user_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppMetadata {
    pub review: Option<AppReview>,
    pub categories: Option<Vec<String>>,
    pub sub_categories: Option<Vec<String>>,
    pub seo_description: Option<String>,
    pub screenshots: Option<Vec<AppScreenshot>>,
    pub developer: Option<String>,
    pub version: Option<String>,
    pub version_id: Option<String>,
    pub version_notes: Option<String>,
    pub first_party_type: Option<String>,
    pub first_party_requires_install: Option<bool>,
    pub show_in_composer_when_unlinked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub logo_url: Option<String>,
    pub logo_url_dark: Option<String>,
    pub icon_assets: Option<HashMap<String, String>>,
    pub icon_dark_assets: Option<HashMap<String, String>>,
    pub distribution_channel: Option<String>,
    pub branding: Option<AppBranding>,
    pub app_metadata: Option<AppMetadata>,
    pub labels: Option<HashMap<String, String>>,
    pub install_url: Option<String>,
    #[serde(default)]
    pub is_accessible: bool,
    #[serde(default = "default_enabled")]
    pub is_enabled: bool,
    #[serde(default)]
    pub plugin_display_names: Vec<String>,
}

impl AppInfo {
    pub fn category(&self) -> Option<String> {
        self.branding
            .as_ref()
            .and_then(|branding| non_empty(branding.category.as_deref()))
            .or_else(|| {
                self.app_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.categories.as_ref())
                    .and_then(|categories| {
                        categories
                            .iter()
                            .find_map(|category| non_empty(Some(category)))
                    })
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsListResponse {
    pub data: Vec<AppInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsReadParams {
    pub app_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_tools: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppToolSummary {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMetadata {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub icon_url_dark: Option<String>,
    pub distribution_channel: Option<String>,
    pub install_url: Option<String>,
    #[serde(default)]
    pub plugin_display_names: Vec<String>,
    pub tool_summaries: Option<Vec<AppToolSummary>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsReadResponse {
    pub apps: Vec<ConnectorMetadata>,
    pub missing_app_ids: Vec<String>,
}

const fn default_enabled() -> bool {
    true
}

fn non_empty(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_params_match_generated_default_shape() {
        assert_eq!(
            serde_json::to_value(AppsListParams::default()).unwrap(),
            serde_json::json!({
                "cursor": null,
                "limit": null,
                "threadId": null
            })
        );
    }

    #[test]
    fn app_info_uses_generated_defaults_and_category_fallback() {
        let app: AppInfo = serde_json::from_value(serde_json::json!({
            "id": "calendar",
            "name": "Calendar",
            "description": null,
            "logoUrl": null,
            "logoUrlDark": null,
            "iconAssets": null,
            "iconDarkAssets": null,
            "distributionChannel": null,
            "branding": null,
            "appMetadata": {
                "review": null,
                "categories": ["Productivity"],
                "subCategories": null,
                "seoDescription": null,
                "screenshots": null,
                "developer": null,
                "version": null,
                "versionId": null,
                "versionNotes": null,
                "firstPartyType": null,
                "firstPartyRequiresInstall": null,
                "showInComposerWhenUnlinked": null
            },
            "labels": null,
            "installUrl": null
        }))
        .unwrap();
        assert!(app.is_enabled);
        assert!(!app.is_accessible);
        assert_eq!(app.category().as_deref(), Some("Productivity"));
    }
}
