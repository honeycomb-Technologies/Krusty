//! Typed Codex memory contracts used by Personalization and per-thread controls.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadMemoryMode {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMemoryModeSetParams {
    pub thread_id: String,
    pub mode: ThreadMemoryMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMemoryModeSetResponse {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryResetResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_memory_contracts_use_exact_wire_shapes() {
        assert_eq!(
            serde_json::to_value(ThreadMemoryModeSetParams {
                thread_id: "thread-1".to_owned(),
                mode: ThreadMemoryMode::Disabled,
            })
            .unwrap(),
            serde_json::json!({ "threadId": "thread-1", "mode": "disabled" })
        );
        assert_eq!(
            serde_json::to_value(MemoryResetResponse::default()).unwrap(),
            serde_json::json!({})
        );
    }
}
