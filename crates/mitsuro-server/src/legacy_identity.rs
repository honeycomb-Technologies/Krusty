//! Deprecated server input aliases retained for one mixed-version bridge.

use axum::http::HeaderMap;
use serde_json::Value;

pub(crate) const HIVE_API_PREFIX: &str = "/mako";
pub(crate) const HIVE_SESSION_TYPE: &str = "mako";
pub(crate) const HIVE_PUSH_EVENT: &str = "mako_update";
pub(crate) const HIVE_NOTIFICATION_FOCUS: &str = "mako";
pub(crate) const HIVE_APNS_CATEGORY: &str = "MAKO_SESSION";
pub(crate) const APNS_BUNDLE_ID: &str = "io.krusty.mobile";
pub(crate) const OAUTH_RESULT_STORAGE_KEY: &str = "krusty:oauth-result";
pub(crate) const OAUTH_RESULT_CHANNEL: &str = "krusty:oauth";
pub(crate) const OAUTH_COMPLETE_EVENT_TYPE: &str = "krusty-oauth-complete";
pub(crate) const SESSION_WIRE_VERSION_HEADER: &str = "x-mitsuro-wire-version";
pub(crate) const CANONICAL_SESSION_WIRE_VERSION: u64 = 2;

/// The generic `/sessions` surface defaults to the preceding release's Hive
/// discriminator until the caller explicitly opts into the canonical wire.
/// Canonical `/hive` and deprecated `/mako` routes have their own fixed DTOs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionWireFormat {
    Legacy,
    Canonical,
}

impl SessionWireFormat {
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let canonical = headers
            .get(SESSION_WIRE_VERSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|version| version >= CANONICAL_SESSION_WIRE_VERSION);
        if canonical {
            Self::Canonical
        } else {
            Self::Legacy
        }
    }
}

/// Rewrite only the delivery copy of a Hive notification for installed clients
/// from the preceding release. Durable intents and internal state stay canonical.
pub(crate) fn bridge_hive_notification(
    category: &mut Option<String>,
    data: &mut Option<Value>,
) -> bool {
    let is_hive = category.as_deref() == Some("HIVE_SESSION")
        || data.as_ref().is_some_and(|data| {
            data.get("type").and_then(Value::as_str) == Some("hive_update")
                || data.get("focus").and_then(Value::as_str) == Some("hive")
        });
    if !is_hive {
        return false;
    }
    if category.as_deref() == Some("HIVE_SESSION") {
        *category = Some(HIVE_APNS_CATEGORY.to_string());
    }
    if let Some(object) = data.as_mut().and_then(Value::as_object_mut) {
        if object.get("type").and_then(Value::as_str) == Some("hive_update") {
            object.insert("type".into(), Value::String(HIVE_PUSH_EVENT.to_string()));
        }
        if object.get("focus").and_then(Value::as_str) == Some("hive") {
            object.insert(
                "focus".into(),
                Value::String(HIVE_NOTIFICATION_FOCUS.to_string()),
            );
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hive_delivery_bridge_changes_only_legacy_wire_fields() {
        let mut category = Some("HIVE_SESSION".to_string());
        let mut data = Some(serde_json::json!({
            "type": "hive_update",
            "focus": "hive",
            "kind": "completion",
        }));
        assert!(bridge_hive_notification(&mut category, &mut data));
        assert_eq!(category.as_deref(), Some(HIVE_APNS_CATEGORY));
        let data = data.expect("data");
        assert_eq!(data["type"], HIVE_PUSH_EVENT);
        assert_eq!(data["focus"], HIVE_NOTIFICATION_FOCUS);
        assert_eq!(data["kind"], "completion");
    }

    #[test]
    fn generic_session_wire_requires_explicit_canonical_version() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            SessionWireFormat::from_headers(&headers),
            SessionWireFormat::Legacy
        );
        headers.insert(
            SESSION_WIRE_VERSION_HEADER,
            "1".parse().expect("header value"),
        );
        assert_eq!(
            SessionWireFormat::from_headers(&headers),
            SessionWireFormat::Legacy
        );
        headers.insert(
            SESSION_WIRE_VERSION_HEADER,
            "malformed".parse().expect("header value"),
        );
        assert_eq!(
            SessionWireFormat::from_headers(&headers),
            SessionWireFormat::Legacy
        );
        for version in ["2", "3", "999"] {
            headers.insert(
                SESSION_WIRE_VERSION_HEADER,
                version.parse().expect("header value"),
            );
            assert_eq!(
                SessionWireFormat::from_headers(&headers),
                SessionWireFormat::Canonical
            );
        }
        headers.insert(
            SESSION_WIRE_VERSION_HEADER,
            "2.0".parse().expect("header value"),
        );
        assert_eq!(
            SessionWireFormat::from_headers(&headers),
            SessionWireFormat::Legacy
        );
    }
}
