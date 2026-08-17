use serde_json::Value;

use super::super::super::super::core::AiClient;
use crate::ai::retry::safe_provider_event_error;

impl AiClient {
    pub(crate) fn codex_api_event_is_retryable(event: &Value) -> bool {
        let code = event
            .pointer("/error/code")
            .or_else(|| event.pointer("/response/error/code"))
            .and_then(Value::as_str);
        let error_type = event
            .pointer("/error/type")
            .or_else(|| event.pointer("/response/error/type"))
            .and_then(Value::as_str);

        let metadata = [code, error_type]
            .into_iter()
            .flatten()
            .map(|value| value.trim().to_ascii_lowercase())
            .collect::<Vec<_>>();

        if metadata.iter().any(|value| {
            matches!(
                value.as_str(),
                "api_error"
                    | "capacity_exceeded"
                    | "gateway_timeout"
                    | "internal_server_error"
                    | "overloaded_error"
                    | "rate_limit_error"
                    | "rate_limit_exceeded"
                    | "server_error"
                    | "service_unavailable"
                    | "too_many_requests"
            )
        }) {
            return true;
        }

        if metadata.iter().any(|value| {
            matches!(
                value.as_str(),
                "authentication_error"
                    | "bad_request"
                    | "billing_hard_limit_reached"
                    | "conflict"
                    | "content_policy_violation"
                    | "context_length_exceeded"
                    | "forbidden"
                    | "insufficient_quota"
                    | "invalid_request_error"
                    | "model_not_found"
                    | "not_found"
                    | "not_found_error"
                    | "permission_error"
                    | "quota_exceeded"
                    | "request_too_large"
                    | "unauthorized"
                    | "unprocessable_entity"
                    | "usage_limit_reached"
            )
        }) {
            return false;
        }

        // ChatGPT's Responses WebSocket occasionally returns backend-specific
        // code/type vocabulary for transient failures. Unknown metadata stays
        // fingerprint-only at the client boundary, but receives the same
        // bounded pre-output retry as recognized transient codes. Events with
        // no structured metadata remain terminal rather than being guessed at.
        !metadata.is_empty()
    }

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

    #[test]
    fn websocket_server_errors_are_retryable_but_invalid_requests_are_not() {
        assert!(AiClient::codex_api_event_is_retryable(
            &serde_json::json!({"type":"error","error":{"code":"server_error"}})
        ));
        assert!(!AiClient::codex_api_event_is_retryable(
            &serde_json::json!({"type":"error","error":{"code":"invalid_request_error"}})
        ));
        assert!(AiClient::codex_api_event_is_retryable(&serde_json::json!({
            "type":"error",
            "error":{"code":"BACKEND_TRANSIENT_SENTINEL","type":"CUSTOM_FAILURE"}
        })));
        assert!(!AiClient::codex_api_event_is_retryable(
            &serde_json::json!({"type":"error","error":{"message":"unclassified"}})
        ));
    }
}
