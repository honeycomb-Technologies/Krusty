//! Typed Codex per-thread settings and metadata contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ApprovalsReviewer, CollaborationMode, SandboxPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicyMode {
    Untrusted,
    OnRequest,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GranularApprovalPolicy {
    pub sandbox_approval: bool,
    pub rules: bool,
    pub skill_approval: bool,
    pub request_permissions: bool,
    pub mcp_elicitations: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AskForApproval {
    Mode(ApprovalPolicyMode),
    Granular { granular: GranularApprovalPolicy },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadPersonality {
    None,
    Friendly,
    Pragmatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadReasoningSummary {
    Auto,
    Concise,
    Detailed,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadMultiAgentModeName {
    ExplicitRequestOnly,
    Proactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThreadMultiAgentMode {
    Named(ThreadMultiAgentModeName),
    Custom { custom: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePermissionProfile {
    pub id: String,
    pub extends: Option<String>,
}

/// Patch for `thread/settings/update`.
///
/// Every optional field is double-wrapped so `None` omits the field while
/// `Some(None)` emits JSON `null` and clears the persisted override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSettingsUpdateParams {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<Option<AskForApproval>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approvals_reviewer: Option<Option<ApprovalsReviewer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<Option<SandboxPolicy>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Option<ThreadReasoningSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_mode: Option<Option<CollaborationMode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_agent_mode: Option<Option<ThreadMultiAgentMode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality: Option<Option<ThreadPersonality>>,
}

impl ThreadSettingsUpdateParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSettingsUpdateResponse {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSettings {
    pub cwd: String,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub sandbox_policy: SandboxPolicy,
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub model: String,
    pub model_provider: String,
    pub service_tier: Option<String>,
    pub effort: Option<String>,
    pub summary: Option<ThreadReasoningSummary>,
    pub collaboration_mode: CollaborationMode,
    pub multi_agent_mode: ThreadMultiAgentMode,
    pub personality: Option<ThreadPersonality>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSettingsUpdatedNotification {
    pub thread_id: String,
    pub thread_settings: ThreadSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMetadataGitInfoUpdateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_url: Option<Option<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMetadataUpdateParams {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_info: Option<Option<ThreadMetadataGitInfoUpdateParams>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMetadataUpdateResponse {
    pub thread: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_setting_patch_distinguishes_omitted_values_from_explicit_null() {
        let mut params = ThreadSettingsUpdateParams::new("thread-1");
        params.model = Some(Some("gpt-5.6-sol".to_owned()));
        params.service_tier = Some(None);
        params.personality = Some(Some(ThreadPersonality::Pragmatic));
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({
                "threadId": "thread-1",
                "model": "gpt-5.6-sol",
                "serviceTier": null,
                "personality": "pragmatic"
            })
        );
    }

    #[test]
    fn metadata_patch_preserves_clear_replace_and_omit_semantics() {
        let params = ThreadMetadataUpdateParams {
            thread_id: "thread-1".to_owned(),
            git_info: Some(Some(ThreadMetadataGitInfoUpdateParams {
                sha: None,
                branch: Some(Some("main".to_owned())),
                origin_url: Some(None),
            })),
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({
                "threadId": "thread-1",
                "gitInfo": {"branch": "main", "originUrl": null}
            })
        );
    }

    #[test]
    fn settings_notification_preserves_authoritative_composer_state() {
        let notification: ThreadSettingsUpdatedNotification =
            serde_json::from_value(serde_json::json!({
                "threadId": "thread-1",
                "threadSettings": {
                    "cwd": "/workspace",
                    "approvalPolicy": "on-request",
                    "approvalsReviewer": "user",
                    "sandboxPolicy": {
                        "type": "workspaceWrite",
                        "writableRoots": ["/workspace"],
                        "networkAccess": false,
                        "excludeSlashTmp": false,
                        "excludeTmpdirEnvVar": false
                    },
                    "activePermissionProfile": {"id": ":workspace", "extends": null},
                    "model": "gpt-5.6-sol",
                    "modelProvider": "openai",
                    "serviceTier": "priority",
                    "effort": "high",
                    "summary": "concise",
                    "collaborationMode": {
                        "mode": "plan",
                        "settings": {
                            "model": "gpt-5.6-sol",
                            "reasoning_effort": "high",
                            "developer_instructions": null
                        }
                    },
                    "multiAgentMode": "explicitRequestOnly",
                    "personality": "pragmatic"
                }
            }))
            .unwrap();
        assert_eq!(notification.thread_id, "thread-1");
        assert_eq!(notification.thread_settings.model, "gpt-5.6-sol");
        assert_eq!(
            notification.thread_settings.collaboration_mode.mode,
            crate::ModeKind::Plan
        );
        assert_eq!(
            notification
                .thread_settings
                .active_permission_profile
                .unwrap()
                .id,
            ":workspace"
        );
    }
}
