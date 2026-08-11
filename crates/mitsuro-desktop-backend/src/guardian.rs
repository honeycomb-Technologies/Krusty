//! Typed Codex app-server Guardian follow-up contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadApproveGuardianDeniedActionParams {
    pub thread_id: String,
    /// Serialized `GuardianAssessmentEvent` supplied by the server.
    pub event: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadApproveGuardianDeniedActionResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guardian_params_keep_the_opaque_assessment_event() {
        let params = ThreadApproveGuardianDeniedActionParams {
            thread_id: "thread-1".to_owned(),
            event: serde_json::json!({"action": "network", "reason": "policy"}),
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({
                "threadId": "thread-1",
                "event": {"action": "network", "reason": "policy"}
            })
        );
    }
}
