use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

const DEFAULT_DURATION_SECS: u64 = 60;
const MAX_DURATION_SECS: u64 = 300;

pub struct SleepTool;

#[derive(Deserialize)]
struct Params {
    #[serde(default = "default_duration")]
    duration_secs: u64,
    #[serde(default)]
    reason: Option<String>,
}

fn default_duration() -> u64 {
    DEFAULT_DURATION_SECS
}

#[async_trait]
impl Tool for SleepTool {
    fn name(&self) -> &str {
        "sleep"
    }

    fn description(&self) -> &str {
        "Idle when there is nothing to do. Returns a signal that pauses the autonomous tick engine for the specified duration."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use this when you have no pending work and are waiting for an external condition (build completing, user returning, scheduled event, etc.).

This does NOT actually sleep — it returns a signal that the tick engine reads to pause ticking. The agent loop will resume after the specified duration or when an external event wakes it.

Duration considerations:
- Default: 60 seconds. Maximum: 300 seconds (5 minutes).
- Anthropic prompt caching has a ~5-minute TTL. Sleeping longer than 300s would expire the cache, so the maximum is capped there.
- For short waits (polling a build), use 30-60s.
- For longer idle periods (nothing to do), use the full 300s.

Always provide a reason so the user understands why the agent is idle."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "duration_secs": {
                    "type": "integer",
                    "description": "How long to idle in seconds (default: 60, max: 300)",
                    "minimum": 1,
                    "maximum": 300
                },
                "reason": {
                    "type": "string",
                    "description": "Why the agent is idling (shown to user)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.duration_secs == 0 {
            return ToolResult::invalid_parameters("duration_secs must be at least 1");
        }

        let clamped = params.duration_secs.min(MAX_DURATION_SECS);
        let reason = params
            .reason
            .unwrap_or_else(|| "no work pending".to_string());

        ToolResult::success_data(json!({
            "slept_seconds": clamped,
            "signal": "sleep_idle",
            "reason": reason,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_ctx() -> ToolContext {
        ToolContext::default()
    }

    #[tokio::test]
    async fn default_duration() {
        let result = SleepTool.execute(json!({}), &default_ctx()).await;
        assert!(!result.is_error, "{}", result.output);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["slept_seconds"], 60);
        assert_eq!(parsed["data"]["signal"], "sleep_idle");
    }

    #[tokio::test]
    async fn custom_duration_and_reason() {
        let result = SleepTool
            .execute(
                json!({ "duration_secs": 120, "reason": "waiting for CI" }),
                &default_ctx(),
            )
            .await;
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["slept_seconds"], 120);
        assert_eq!(parsed["data"]["reason"], "waiting for CI");
    }

    #[tokio::test]
    async fn duration_clamped_to_max() {
        let result = SleepTool
            .execute(json!({ "duration_secs": 600 }), &default_ctx())
            .await;
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["slept_seconds"], 300);
    }

    #[tokio::test]
    async fn zero_duration_rejected() {
        let result = SleepTool
            .execute(json!({ "duration_secs": 0 }), &default_ctx())
            .await;
        assert!(result.is_error);
    }
}
