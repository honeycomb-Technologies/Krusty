use serde_json::Value;

use super::super::super::super::core::AiClient;

impl AiClient {
    pub(crate) fn codex_ws_create_payload(body: Value) -> Value {
        match body {
            Value::Object(mut object) => {
                object.insert(
                    "type".to_string(),
                    Value::String("response.create".to_string()),
                );
                Value::Object(object)
            }
            other => serde_json::json!({
                "type": "response.create",
                "response": other
            }),
        }
    }

    pub(crate) fn codex_ws_error_message(event: &Value) -> Option<String> {
        if let Some(message) = event.get("message").and_then(|m| m.as_str()) {
            if !message.is_empty() {
                return Some(message.to_string());
            }
        }

        if let Some(message) = event
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .or_else(|| {
                event
                    .pointer("/response/error/message")
                    .and_then(|m| m.as_str())
            })
            .or_else(|| {
                event
                    .pointer("/response/status_details/error/message")
                    .and_then(|m| m.as_str())
            })
        {
            if !message.is_empty() {
                return Some(message.to_string());
            }
        }

        if let Some(error_text) = event.get("error").and_then(|e| e.as_str()) {
            if !error_text.is_empty() {
                return Some(error_text.to_string());
            }
        }

        let error_type = event
            .pointer("/error/type")
            .and_then(|t| t.as_str())
            .or_else(|| {
                event
                    .pointer("/response/error/type")
                    .and_then(|t| t.as_str())
            });
        let error_code = event
            .pointer("/error/code")
            .and_then(|t| t.as_str())
            .or_else(|| {
                event
                    .pointer("/response/error/code")
                    .and_then(|t| t.as_str())
            });
        match (error_type, error_code) {
            (Some(error_type), Some(error_code))
                if !error_type.is_empty() || !error_code.is_empty() =>
            {
                Some(format!("{} ({})", error_type, error_code))
            }
            (Some(error_type), None) if !error_type.is_empty() => Some(error_type.to_string()),
            (None, Some(error_code)) if !error_code.is_empty() => Some(error_code.to_string()),
            _ => None,
        }
    }
}
