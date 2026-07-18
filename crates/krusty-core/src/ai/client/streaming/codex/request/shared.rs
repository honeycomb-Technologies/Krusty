use serde_json::Value;

use super::super::super::super::core::AiClient;
use crate::ai::retry::safe_provider_event_error;

impl AiClient {
    pub(crate) fn codex_ws_create_payload(body: Value) -> Value {
        match body {
            Value::Object(mut object) => {
                // WebSocket `response.create` mirrors the Responses body, but
                // transport-specific HTTP fields are not valid on the socket.
                object.remove("stream");
                object.remove("background");
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
        let message = event
            .get("message")
            .and_then(|message| message.as_str())
            .or_else(|| {
                event
                    .pointer("/error/message")
                    .and_then(|message| message.as_str())
            })
            .or_else(|| {
                event
                    .pointer("/response/error/message")
                    .and_then(|message| message.as_str())
            })
            .or_else(|| {
                event
                    .pointer("/response/status_details/error/message")
                    .and_then(|message| message.as_str())
            })
            .or_else(|| event.get("error").and_then(|error| error.as_str()));
        let error_type = event
            .pointer("/error/type")
            .and_then(|value| value.as_str())
            .or_else(|| {
                event
                    .pointer("/response/error/type")
                    .and_then(|value| value.as_str())
            });
        let error_code = event
            .pointer("/error/code")
            .and_then(|value| value.as_str())
            .or_else(|| {
                event
                    .pointer("/response/error/code")
                    .and_then(|value| value.as_str())
            });

        if message.is_none() && error_type.is_none() && error_code.is_none() {
            return None;
        }
        Some(safe_provider_event_error(
            "Codex websocket API error",
            error_code,
            error_type,
            message,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::AiClient;

    #[test]
    fn websocket_error_metadata_never_reflects_provider_strings() {
        const MESSAGE_SENTINEL: &str = "WS_MESSAGE_SENTINEL_19b7";
        const CODE_SENTINEL: &str = "WS_CODE_SENTINEL_c42a";
        const TYPE_SENTINEL: &str = "WS_TYPE_SENTINEL_04de";
        let error = AiClient::codex_ws_error_message(&serde_json::json!({
            "type": "error",
            "error": {
                "message": MESSAGE_SENTINEL,
                "code": CODE_SENTINEL,
                "type": TYPE_SENTINEL
            }
        }))
        .expect("error metadata should be present");

        for sentinel in [MESSAGE_SENTINEL, CODE_SENTINEL, TYPE_SENTINEL] {
            assert!(!error.contains(sentinel));
        }
        assert!(error.contains("message_fingerprint=sha256:"));
        assert!(error.contains("code_fingerprint=sha256:"));
        assert!(error.contains("category_fingerprint=sha256:"));
    }

    #[test]
    fn websocket_payload_removes_http_transport_fields() {
        let payload = AiClient::codex_ws_create_payload(serde_json::json!({
            "model": "gpt-5.6",
            "stream": true,
            "background": false,
            "input": []
        }));

        assert_eq!(payload["type"], "response.create");
        assert!(payload.get("stream").is_none());
        assert!(payload.get("background").is_none());
    }
}
