//! Typed Codex hook catalog contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksListParams {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cwds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksListResponse {
    pub data: Vec<HooksListEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksListEntry {
    pub cwd: String,
    pub hooks: Vec<HookMetadata>,
    pub warnings: Vec<String>,
    pub errors: Vec<HookErrorInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookErrorInfo {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookMetadata {
    pub key: String,
    pub event_name: HookEventName,
    pub handler_type: HookHandlerType,
    pub matcher: Option<String>,
    pub command: Option<String>,
    pub timeout_sec: u64,
    pub status_message: Option<String>,
    pub additional_context_limit: Option<usize>,
    pub source_path: String,
    pub source: HookSource,
    pub plugin_id: Option<String>,
    pub display_order: i64,
    pub enabled: bool,
    pub is_managed: bool,
    pub current_hash: String,
    pub trust_status: HookTrustStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookEventName {
    PreToolUse,
    PermissionRequest,
    PostToolUse,
    PreCompact,
    PostCompact,
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    SubagentStart,
    SubagentStop,
    Stop,
}

impl HookEventName {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PermissionRequest => "PermissionRequest",
            Self::PostToolUse => "PostToolUse",
            Self::PreCompact => "PreCompact",
            Self::PostCompact => "PostCompact",
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::Stop => "Stop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookHandlerType {
    Command,
    Prompt,
    Agent,
}

impl HookHandlerType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Prompt => "prompt",
            Self::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookSource {
    System,
    User,
    Project,
    Mdm,
    SessionFlags,
    Plugin,
    CloudRequirements,
    CloudManagedConfig,
    LegacyManagedConfigFile,
    LegacyManagedConfigMdm,
    Unknown,
}

impl HookSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Project => "project",
            Self::Mdm => "MDM",
            Self::SessionFlags => "session flags",
            Self::Plugin => "plugin",
            Self::CloudRequirements => "cloud requirements",
            Self::CloudManagedConfig => "cloud managed config",
            Self::LegacyManagedConfigFile => "legacy managed file",
            Self::LegacyManagedConfigMdm => "legacy managed MDM",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookTrustStatus {
    Managed,
    Untrusted,
    Trusted,
    Modified,
}

impl HookTrustStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Untrusted => "untrusted",
            Self::Trusted => "trusted",
            Self::Modified => "modified",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_params_omit_empty_default_cwds() {
        assert_eq!(
            serde_json::to_value(HooksListParams::default()).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(HooksListParams {
                cwds: vec!["/workspace".to_owned()],
            })
            .unwrap(),
            serde_json::json!({"cwds": ["/workspace"]})
        );
    }

    #[test]
    fn hook_metadata_parses_current_generated_shape() {
        let hook: HookMetadata = serde_json::from_value(serde_json::json!({
            "key": "project:preToolUse:0",
            "eventName": "preToolUse",
            "handlerType": "command",
            "matcher": "exec_command",
            "command": "scripts/check.sh",
            "timeoutSec": 10,
            "statusMessage": null,
            "additionalContextLimit": null,
            "sourcePath": "/workspace/.codex/hooks.json",
            "source": "project",
            "pluginId": null,
            "displayOrder": 0,
            "enabled": true,
            "isManaged": false,
            "currentHash": "sha256:test",
            "trustStatus": "trusted"
        }))
        .unwrap();
        assert_eq!(hook.event_name, HookEventName::PreToolUse);
        assert_eq!(hook.handler_type, HookHandlerType::Command);
        assert_eq!(hook.trust_status, HookTrustStatus::Trusted);
    }
}
