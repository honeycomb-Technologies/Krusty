//! Typed Codex permission-profile and managed-requirements contracts.
//!
//! The Codex desktop derives its composer permission menu from these read-only
//! app-server methods. Mitsuro's supervised/autonomous modes are a separate
//! product contract and intentionally do not use these types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const READ_ONLY_PROFILE_ID: &str = ":read-only";
pub const WORKSPACE_PROFILE_ID: &str = ":workspace";
pub const FULL_ACCESS_PROFILE_ID: &str = ":danger-full-access";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfileListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfileSummary {
    pub id: String,
    pub description: Option<String>,
    pub allowed: bool,
}

impl PermissionProfileSummary {
    pub fn is_builtin(&self) -> bool {
        matches!(
            self.id.as_str(),
            READ_ONLY_PROFILE_ID | WORKSPACE_PROFILE_ID | FULL_ACCESS_PROFILE_ID
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfileListResponse {
    pub data: Vec<PermissionProfileSummary>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequirementsReadResponse {
    #[serde(default)]
    pub requirements: Option<ConfigRequirements>,
}

/// Managed requirements that can narrow the permission menu.
///
/// Complex policy values stay as JSON because the UI only needs to preserve and
/// compare them, while the profile, sandbox, reviewer, and feature gates are
/// schema-exact typed fields.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequirements {
    #[serde(default)]
    pub default_permissions: Option<String>,
    #[serde(default)]
    pub allowed_permission_profiles: Option<HashMap<String, bool>>,
    #[serde(default)]
    pub allowed_sandbox_modes: Option<Vec<SandboxMode>>,
    #[serde(default)]
    pub allowed_approval_policies: Option<Vec<Value>>,
    #[serde(default)]
    pub allowed_approvals_reviewers: Option<Vec<ApprovalsReviewer>>,
    #[serde(default)]
    pub allow_remote_control: Option<bool>,
    #[serde(default)]
    pub feature_requirements: Option<HashMap<String, bool>>,
    #[serde(default)]
    pub feedback: Option<crate::FeedbackRequirements>,
}

impl ConfigRequirements {
    pub fn allows_profile(&self, id: &str) -> bool {
        self.allowed_permission_profiles
            .as_ref()
            .and_then(|profiles| profiles.get(id))
            .copied()
            .unwrap_or(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalsReviewer {
    User,
    AutoReview,
    GuardianSubagent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProviderCapabilitiesReadParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProviderCapabilitiesReadResponse {
    pub image_generation: bool,
    pub namespace_tools: bool,
    pub web_search: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_permission_contracts_round_trip() {
        let profiles: PermissionProfileListResponse = serde_json::from_value(serde_json::json!({
            "data": [
                {"id": ":read-only", "description": null, "allowed": true},
                {"id": ":workspace", "description": "Workspace", "allowed": true},
                {"id": ":danger-full-access", "description": null, "allowed": false}
            ],
            "nextCursor": null
        }))
        .unwrap();
        assert!(profiles.data[0].is_builtin());
        assert!(!profiles.data[2].allowed);

        let requirements: ConfigRequirementsReadResponse =
            serde_json::from_value(serde_json::json!({
                "requirements": {
                    "defaultPermissions": ":workspace",
                    "allowedPermissionProfiles": {
                        ":workspace": true,
                        ":danger-full-access": false
                    },
                    "allowedSandboxModes": ["read-only", "workspace-write"],
                    "allowedApprovalsReviewers": ["user", "auto_review"],
                    "feedback": {"enabled": false}
                }
            }))
            .unwrap();
        let requirements = requirements.requirements.unwrap();
        assert!(requirements.allows_profile(WORKSPACE_PROFILE_ID));
        assert!(!requirements.allows_profile(FULL_ACCESS_PROFILE_ID));
        assert_eq!(requirements.feedback.unwrap().enabled, Some(false));

        assert_eq!(
            serde_json::to_value(PermissionProfileListParams::default()).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(ModelProviderCapabilitiesReadParams::default()).unwrap(),
            serde_json::json!({})
        );
    }
}
