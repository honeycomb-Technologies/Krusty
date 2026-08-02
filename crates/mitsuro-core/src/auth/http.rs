use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_AUTH_ERROR_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_AUTH_SUCCESS_RESPONSE_BYTES: usize = 256 * 1024;
const AUTH_RESPONSE_BODY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_AUTH_METADATA_BYTES: usize = 512;

const KNOWN_OAUTH_ERROR_CODES: &[&str] = &[
    "access_denied",
    "authorization_pending",
    "callback_timeout",
    "expired_token",
    "invalid_client",
    "invalid_grant",
    "invalid_request",
    "invalid_scope",
    "invalid_token",
    "missing_code",
    "server_error",
    "slow_down",
    "state_mismatch",
    "temporarily_unavailable",
    "unauthorized_client",
    "unsupported_grant_type",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OAuthControlCode {
    AuthorizationPending,
    SlowDown,
    ExpiredToken,
    AccessDenied,
}

pub(super) struct BoundedAuthResponse {
    status: StatusCode,
    body: Vec<u8>,
    response_fingerprint: String,
}

impl BoundedAuthResponse {
    pub(super) fn status(&self) -> StatusCode {
        self.status
    }

    pub(super) fn oauth_control_code(&self) -> Option<OAuthControlCode> {
        match oauth_error_code(&self.body)? {
            "authorization_pending" => Some(OAuthControlCode::AuthorizationPending),
            "slow_down" => Some(OAuthControlCode::SlowDown),
            "expired_token" => Some(OAuthControlCode::ExpiredToken),
            "access_denied" => Some(OAuthControlCode::AccessDenied),
            _ => None,
        }
    }

    pub(super) fn safe_error(&self, label: &str) -> anyhow::Error {
        let parsed = serde_json::from_slice::<Value>(&self.body).ok();
        let raw_code = parsed.as_ref().and_then(extract_oauth_error_code);
        let raw_message = parsed.as_ref().and_then(extract_oauth_error_message);
        let mut fields = Vec::with_capacity(4);

        if let Some(code) = raw_code.filter(|value| !value.is_empty()) {
            if let Some(code) = known_oauth_error_code(code) {
                fields.push(format!("code={code}"));
            } else {
                fields.push(format!(
                    "code_fingerprint={}",
                    metadata_fingerprint(b"oauth-code", code.as_bytes())
                ));
            }
        }
        if let Some(message) = raw_message.filter(|value| !value.is_empty()) {
            fields.push(format!(
                "message_fingerprint={}",
                metadata_fingerprint(b"oauth-message", message.as_bytes())
            ));
        }
        fields.push(format!(
            "response_fingerprint={}",
            self.response_fingerprint
        ));
        fields.push(format!("response_bytes={}", self.body.len()));

        anyhow!(
            "{} ({}) [{}]",
            sanitize_label(label),
            self.status,
            fields.join(", ")
        )
    }

    pub(super) fn parse_json<T: DeserializeOwned>(&self, label: &str) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|error| {
            let category = match error.classify() {
                serde_json::error::Category::Io => "io",
                serde_json::error::Category::Syntax => "syntax",
                serde_json::error::Category::Data => "data",
                serde_json::error::Category::Eof => "eof",
            };
            anyhow!(
                "{} was invalid JSON [category={}, line={}, column={}, response_fingerprint={}]",
                sanitize_label(label),
                category,
                error.line(),
                error.column(),
                self.response_fingerprint
            )
        })
    }
}

pub(super) async fn read_auth_response(response: Response) -> Result<BoundedAuthResponse> {
    let max_bytes = if response.status().is_success() {
        MAX_AUTH_SUCCESS_RESPONSE_BYTES
    } else {
        MAX_AUTH_ERROR_RESPONSE_BYTES
    };
    read_auth_response_with_limits(response, max_bytes, AUTH_RESPONSE_BODY_TIMEOUT).await
}

pub(super) fn safe_oauth_callback_error(
    label: &str,
    code: Option<&str>,
    message: Option<&str>,
) -> anyhow::Error {
    let mut fields = Vec::with_capacity(2);
    if let Some(code) = code.filter(|value| !value.is_empty()) {
        if let Some(code) = known_oauth_error_code(code) {
            fields.push(format!("code={code}"));
        } else {
            fields.push(format!(
                "code_fingerprint={}",
                metadata_fingerprint(b"oauth-callback-code", code.as_bytes())
            ));
        }
    }
    if let Some(message) = message.filter(|value| !value.is_empty()) {
        fields.push(format!(
            "message_fingerprint={}",
            metadata_fingerprint(b"oauth-callback-message", message.as_bytes())
        ));
    }
    if fields.is_empty() {
        fields.push("metadata=unavailable".to_string());
    }
    anyhow!("{} [{}]", sanitize_label(label), fields.join(", "))
}

async fn read_auth_response_with_limits(
    response: Response,
    max_bytes: usize,
    body_timeout: Duration,
) -> Result<BoundedAuthResponse> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(anyhow!(
            "Authentication response exceeded byte limit [status={}, limit_bytes={}]",
            status,
            max_bytes
        ));
    }

    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(max_bytes.min(8 * 1024));
    let deadline = tokio::time::Instant::now() + body_timeout;

    loop {
        let next = tokio::time::timeout_at(deadline, stream.next())
            .await
            .map_err(|_| {
                anyhow!(
                    "Authentication response body timed out [status={}, limit_bytes={}, captured_bytes={}, response_fingerprint={}]",
                    status,
                    max_bytes,
                    body.len(),
                    fingerprint(&body)
                )
            })?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|_| {
            anyhow!(
                "Authentication response body read failed [status={}, captured_bytes={}, response_fingerprint={}]",
                status,
                body.len(),
                fingerprint(&body)
            )
        })?;
        let remaining = max_bytes.saturating_sub(body.len());
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            return Err(anyhow!(
                "Authentication response exceeded byte limit [status={}, limit_bytes={}, response_fingerprint={}]",
                status,
                max_bytes,
                fingerprint(&body)
            ));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(BoundedAuthResponse {
        status,
        response_fingerprint: fingerprint(&body),
        body,
    })
}

fn oauth_error_code(body: &[u8]) -> Option<&str> {
    let parsed = serde_json::from_slice::<Value>(body).ok()?;
    // The borrowed value cannot escape this function, so use the fixed control
    // vocabulary directly rather than returning provider-owned text.
    match extract_oauth_error_code(&parsed)? {
        "authorization_pending" => Some("authorization_pending"),
        "slow_down" => Some("slow_down"),
        "expired_token" => Some("expired_token"),
        "access_denied" => Some("access_denied"),
        _ => None,
    }
}

fn extract_oauth_error_code(value: &Value) -> Option<&str> {
    value
        .pointer("/error/code")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.get("code").and_then(Value::as_str))
}

fn extract_oauth_error_message(value: &Value) -> Option<&str> {
    value
        .get("error_description")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
        .or_else(|| value.get("message").and_then(Value::as_str))
}

fn known_oauth_error_code(value: &str) -> Option<&'static str> {
    KNOWN_OAUTH_ERROR_CODES
        .iter()
        .find(|known| value.trim().eq_ignore_ascii_case(known))
        .copied()
}

fn sanitize_label(label: &str) -> String {
    let mut sanitized = String::with_capacity(label.len().min(80));
    for character in label.trim().chars().take(80) {
        if character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '_' | '/' | '.') {
            sanitized.push(character);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.trim().is_empty() {
        "Authentication request failed".to_string()
    } else {
        sanitized
    }
}

fn fingerprint(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mitsuro-auth-response-v1\0");
    hasher.update(body.len().to_le_bytes());
    hasher.update(body);
    format!("sha256:{:x}", hasher.finalize())
}

fn metadata_fingerprint(domain: &[u8], value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mitsuro-auth-metadata-v1\0");
    hasher.update(domain.len().to_le_bytes());
    hasher.update(domain);
    hasher.update(value.len().to_le_bytes());
    hasher.update(&value[..value.len().min(MAX_AUTH_METADATA_BYTES)]);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    fn test_response(body: &[u8]) -> BoundedAuthResponse {
        BoundedAuthResponse {
            status: StatusCode::BAD_REQUEST,
            response_fingerprint: fingerprint(body),
            body: body.to_vec(),
        }
    }

    async fn serve_response(
        content_length: usize,
        body: &'static [u8],
        body_delay: Duration,
    ) -> (Response, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have an address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("test request should connect");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 512];
            while request.len() < 8 * 1024 {
                let count = socket.read(&mut chunk).await.unwrap_or_default();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let headers = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
            );
            let _ = socket.write_all(headers.as_bytes()).await;
            tokio::time::sleep(body_delay).await;
            let _ = socket.write_all(body).await;
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/oauth"))
            .send()
            .await
            .expect("test response should be received");
        (response, server)
    }

    #[test]
    fn oauth_error_never_reflects_unknown_code_message_or_token() {
        const CODE_SENTINEL: &str = "OAUTH_CODE_SENTINEL_a713";
        const MESSAGE_SENTINEL: &str = "OAUTH_MESSAGE_SENTINEL_0cb9";
        const TOKEN_SENTINEL: &str = "OAUTH_TOKEN_SENTINEL_b446";
        let body = format!(
            r#"{{"error":"{CODE_SENTINEL}","error_description":"{MESSAGE_SENTINEL}","access_token":"{TOKEN_SENTINEL}"}}"#
        );
        let error = test_response(body.as_bytes())
            .safe_error("Token exchange failed")
            .to_string();

        for sentinel in [CODE_SENTINEL, MESSAGE_SENTINEL, TOKEN_SENTINEL] {
            assert!(!error.contains(sentinel));
        }
        assert!(error.contains("code_fingerprint=sha256:"));
        assert!(error.contains("message_fingerprint=sha256:"));
        assert!(error.contains("response_fingerprint=sha256:"));
    }

    #[test]
    fn oauth_control_codes_are_fixed_protocol_vocabulary() {
        let response = test_response(br#"{"error":"authorization_pending"}"#);
        assert_eq!(
            response.oauth_control_code(),
            Some(OAuthControlCode::AuthorizationPending)
        );
        let error = response.safe_error("Authorization failed").to_string();
        assert!(error.contains("code=authorization_pending"));
    }

    #[test]
    fn auth_json_parse_error_does_not_reflect_body() {
        const SENTINEL: &str = "INVALID_JSON_SENTINEL_99a1";
        let response = test_response(format!(r#"{{"access_token":"{SENTINEL}""#).as_bytes());
        let error = response
            .parse_json::<Value>("OAuth token response")
            .expect_err("invalid JSON should fail")
            .to_string();
        assert!(!error.contains(SENTINEL));
        assert!(error.contains("response_fingerprint=sha256:"));
    }

    #[test]
    fn oauth_callback_error_never_reflects_query_values() {
        const CODE_SENTINEL: &str = "CALLBACK_CODE_SENTINEL_58b0";
        const MESSAGE_SENTINEL: &str = "CALLBACK_MESSAGE_SENTINEL_f310";
        let error = safe_oauth_callback_error(
            "OAuth callback failed",
            Some(CODE_SENTINEL),
            Some(MESSAGE_SENTINEL),
        )
        .to_string();
        assert!(!error.contains(CODE_SENTINEL));
        assert!(!error.contains(MESSAGE_SENTINEL));
        assert!(error.contains("code_fingerprint=sha256:"));
        assert!(error.contains("message_fingerprint=sha256:"));

        let fixed = safe_oauth_callback_error(
            "OAuth callback failed",
            Some("state_mismatch"),
            Some(MESSAGE_SENTINEL),
        )
        .to_string();
        assert!(fixed.contains("code=state_mismatch"));
        assert!(!fixed.contains(MESSAGE_SENTINEL));
    }

    #[tokio::test]
    async fn auth_response_read_is_byte_and_time_bounded() {
        let (oversized, oversized_server) = serve_response(32, b"", Duration::ZERO).await;
        let oversized_error =
            match read_auth_response_with_limits(oversized, 8, Duration::from_millis(10)).await {
                Ok(_) => panic!("oversized response should fail"),
                Err(error) => error.to_string(),
            };
        oversized_server.await.expect("test server should finish");
        assert!(oversized_error.contains("exceeded byte limit"));

        let (slow, slow_server) = serve_response(1, b"x", Duration::from_millis(50)).await;
        let timeout_error =
            match read_auth_response_with_limits(slow, 8, Duration::from_millis(10)).await {
                Ok(_) => panic!("slow response should time out"),
                Err(error) => error.to_string(),
            };
        slow_server.await.expect("test server should finish");
        assert!(timeout_error.contains("body timed out"));
    }

    #[test]
    fn auth_success_and_error_limits_are_finite() {
        const {
            assert!(MAX_AUTH_ERROR_RESPONSE_BYTES <= 16 * 1024);
            assert!(MAX_AUTH_SUCCESS_RESPONSE_BYTES <= 256 * 1024);
            assert!(AUTH_RESPONSE_BODY_TIMEOUT.as_secs() <= 5);
        }
    }
}
