//! Typed Codex app-server feedback contracts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Managed policy for whether the desktop may offer feedback submission.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackRequirements {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackUploadParams {
    pub classification: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_logs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_log_files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<BTreeMap<String, String>>,
}

impl FeedbackUploadParams {
    pub fn new(classification: impl Into<String>) -> Self {
        Self {
            classification: classification.into(),
            reason: None,
            thread_id: None,
            include_logs: None,
            extra_log_files: None,
            tags: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackUploadResponse {
    pub thread_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_upload_omits_unset_optional_fields() {
        assert_eq!(
            serde_json::to_value(FeedbackUploadParams::new("bug")).unwrap(),
            serde_json::json!({"classification": "bug"})
        );
    }
}
