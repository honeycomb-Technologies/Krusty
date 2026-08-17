//! Typed Codex external-agent configuration discovery and import contracts.
//!
//! Detection is read-only. Import writes the selected migration items into the
//! active Codex installation and reports asynchronous progress/completion
//! notifications keyed by `importId`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CLAUDE_CODE_MIGRATION_SOURCE: &str = "claude-code";
pub const CURSOR_MIGRATION_SOURCE: &str = "cursor";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigDetectParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwds: Option<Vec<String>>,
    #[serde(default)]
    pub include_home: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_session_age_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_sessions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration_source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExternalAgentConfigMigrationItemType {
    AgentsMd,
    Config,
    Skills,
    Plugins,
    McpServerConfig,
    Subagents,
    Hooks,
    Commands,
    Memory,
    Sessions,
}

impl ExternalAgentConfigMigrationItemType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::AgentsMd => "Instructions",
            Self::Config => "Settings",
            Self::Skills => "Skills",
            Self::Plugins => "Plugins",
            Self::McpServerConfig => "MCP servers",
            Self::Subagents => "Subagents",
            Self::Hooks => "Hooks",
            Self::Commands => "Commands",
            Self::Memory => "Memory",
            Self::Sessions => "Recent chats",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigMigrationItem {
    pub item_type: ExternalAgentConfigMigrationItemType,
    pub description: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Kept losslessly because each item type has a different generated shape.
    #[serde(default)]
    pub details: Option<Value>,
}

impl ExternalAgentConfigMigrationItem {
    pub fn detail_count(&self) -> usize {
        let Some(details) = self.details.as_ref().and_then(Value::as_object) else {
            return 0;
        };
        details
            .values()
            .filter_map(Value::as_array)
            .map(Vec::len)
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalAgentDetectedConnectorSource {
    RemoteMcpServersConfig,
    SessionToolUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentDetectedConnectorCandidate {
    pub name: String,
    pub session_count: u32,
    pub source: ExternalAgentDetectedConnectorSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigDetectResponse {
    pub items: Vec<ExternalAgentConfigMigrationItem>,
    #[serde(default)]
    pub connectors: Vec<ExternalAgentDetectedConnectorCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportParams {
    pub migration_items: Vec<ExternalAgentConfigMigrationItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportResponse {
    pub import_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportItemTypeSuccess {
    pub item_type: ExternalAgentConfigMigrationItemType,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportItemTypeFailure {
    pub failure_stage: String,
    pub item_type: ExternalAgentConfigMigrationItemType,
    pub message: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub error_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub sub_error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportTypeResult {
    pub item_type: ExternalAgentConfigMigrationItemType,
    pub successes: Vec<ExternalAgentConfigImportItemTypeSuccess>,
    pub failures: Vec<ExternalAgentConfigImportItemTypeFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportStatusNotification {
    pub import_id: String,
    pub item_type_results: Vec<ExternalAgentConfigImportTypeResult>,
}

pub type ExternalAgentConfigImportProgressNotification =
    ExternalAgentConfigImportStatusNotification;
pub type ExternalAgentConfigImportCompletedNotification =
    ExternalAgentConfigImportStatusNotification;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportHistory {
    pub completed_at_ms: i64,
    pub failures: Vec<ExternalAgentConfigImportItemTypeFailure>,
    pub import_id: String,
    #[serde(default)]
    pub provider_id: Option<String>,
    pub successes: Vec<ExternalAgentConfigImportItemTypeSuccess>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentImportedConnectorCandidate {
    pub name: String,
    pub session_count: u32,
    pub source: ExternalAgentImportedConnectorSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalAgentImportedConnectorSource {
    RemoteMcpServersConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportHistoriesReadResponse {
    pub data: Vec<ExternalAgentConfigImportHistory>,
    pub connectors: Vec<ExternalAgentImportedConnectorCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentConfigImportHistoryRecordParams {
    pub item_type_results: Vec<ExternalAgentConfigImportTypeResult>,
    pub provider_id: String,
}

pub type ExternalAgentConfigImportHistoryRecordResponse = ExternalAgentConfigImportResponse;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_detection_and_notification_shapes_round_trip() {
        let response: ExternalAgentConfigDetectResponse =
            serde_json::from_value(serde_json::json!({
                "items": [{
                    "itemType": "SKILLS",
                    "description": "Migrate skills",
                    "cwd": null,
                    "details": {"skills": [{"name": "canvas"}]}
                }],
                "connectors": []
            }))
            .unwrap();
        assert_eq!(response.items[0].detail_count(), 1);
        assert_eq!(response.items[0].item_type.label(), "Skills");

        let completed: ExternalAgentConfigImportCompletedNotification =
            serde_json::from_value(serde_json::json!({
                "importId": "import-1",
                "itemTypeResults": [{
                    "itemType": "SKILLS",
                    "successes": [{"itemType": "SKILLS", "target": "/tmp/skills"}],
                    "failures": []
                }]
            }))
            .unwrap();
        assert_eq!(completed.import_id, "import-1");
        assert_eq!(completed.item_type_results[0].successes.len(), 1);
    }
}
