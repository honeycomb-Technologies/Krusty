use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use krusty_core::storage::Database;
use krusty_core::SessionManager;

use crate::apns::{ApnsEventType, ApnsPayload, ApnsService};
use crate::push::{PushEventType, PushPayload, PushService};

pub(crate) const APNS_CATEGORY_TOOL_APPROVAL: &str = "TOOL_APPROVAL";
pub(crate) const APNS_CATEGORY_CHAT_SESSION: &str = "CHAT_SESSION";
pub(crate) const APNS_CATEGORY_MAKO_SESSION: &str = "MAKO_SESSION";

pub(crate) fn session_title(db_path: &Path, session_id: &str) -> String {
    match Database::new(db_path) {
        Ok(db) => {
            let session_manager = SessionManager::new(db);
            match session_manager.get_session(session_id) {
                Ok(Some(session)) => session.title,
                Ok(None) => {
                    tracing::warn!(
                        session_id = %session_id,
                        "Session title requested for missing session"
                    );
                    "Session".to_string()
                }
                Err(err) => {
                    tracing::error!(
                        session_id = %session_id,
                        error = %err,
                        "Failed to load session title"
                    );
                    "Session".to_string()
                }
            }
        }
        Err(err) => {
            tracing::error!(
                "Failed to open database while loading session title: {}",
                err
            );
            "Session".to_string()
        }
    }
}

pub(crate) fn chat_session_notification_data(kind: &str, session_id: &str) -> Value {
    json!({
        "type": "chat_update",
        "kind": kind,
        "sessionId": session_id,
        "focus": "chat",
    })
}

pub(crate) fn mako_session_notification_data(
    kind: &str,
    session_id: &str,
    level: Option<&str>,
    title: Option<&str>,
) -> Value {
    let mut data = json!({
        "type": "mako_update",
        "kind": kind,
        "sessionId": session_id,
        "focus": "mako",
    });

    if let Some(level) = level.filter(|value| !value.trim().is_empty()) {
        data["level"] = Value::String(level.to_string());
    }

    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        data["title"] = Value::String(title.to_string());
    }

    data
}

pub(crate) fn tool_approval_notification_data(
    request_id: &str,
    session_id: &str,
    tool_name: &str,
    focus: &str,
) -> Value {
    json!({
        "type": "tool_approval",
        "requestId": request_id,
        "sessionId": session_id,
        "toolName": tool_name,
        "focus": focus,
    })
}

pub(crate) fn fire_push(
    push_service: &Option<Arc<PushService>>,
    user_id: Option<&str>,
    payload: PushPayload,
    event_type: PushEventType,
) {
    if let Some(svc) = push_service.clone() {
        let uid = user_id.map(String::from);
        tokio::spawn(async move {
            let stats = svc.notify_user(uid.as_deref(), payload, event_type).await;
            tracing::info!(
                event_type = event_type.as_str(),
                attempted = stats.attempted,
                sent = stats.sent,
                stale_removed = stats.stale_removed,
                failed = stats.failed,
                "Push event dispatched"
            );
        });
    }
}

pub(crate) fn fire_apns(
    apns_service: &Option<Arc<ApnsService>>,
    user_id: Option<&str>,
    payload: ApnsPayload,
    event_type: ApnsEventType,
) {
    if let Some(svc) = apns_service.clone() {
        let uid = user_id.map(String::from);
        tokio::spawn(async move {
            let stats = svc.notify_user(uid.as_deref(), payload, event_type).await;
            tracing::info!(
                event_type = event_type.as_str(),
                attempted = stats.attempted,
                sent = stats.sent,
                stale_removed = stats.stale_removed,
                failed = stats.failed,
                "APNs event dispatched"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chat_session_notification_data, mako_session_notification_data,
        tool_approval_notification_data,
    };

    #[test]
    fn chat_session_notification_data_carries_focus_and_kind() {
        let data = chat_session_notification_data("completion", "session-1");

        assert_eq!(data["type"], "chat_update");
        assert_eq!(data["kind"], "completion");
        assert_eq!(data["sessionId"], "session-1");
        assert_eq!(data["focus"], "chat");
    }

    #[test]
    fn mako_session_notification_data_carries_focus_kind_and_metadata() {
        let data =
            mako_session_notification_data("user_message", "session-2", Some("info"), Some("Crew"));

        assert_eq!(data["type"], "mako_update");
        assert_eq!(data["kind"], "user_message");
        assert_eq!(data["sessionId"], "session-2");
        assert_eq!(data["focus"], "mako");
        assert_eq!(data["level"], "info");
        assert_eq!(data["title"], "Crew");
    }

    #[test]
    fn tool_approval_notification_data_carries_focus() {
        let data = tool_approval_notification_data("req-1", "session-3", "bash", "mako");

        assert_eq!(data["type"], "tool_approval");
        assert_eq!(data["requestId"], "req-1");
        assert_eq!(data["sessionId"], "session-3");
        assert_eq!(data["toolName"], "bash");
        assert_eq!(data["focus"], "mako");
    }
}
