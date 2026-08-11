//! Exhaustive Codex server-notification registry and normalized lifecycle event.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::TurnStreamEvent;

pub const SERVER_NOTIFICATIONS_TEXT: &str = include_str!("../fixtures/server-notifications.txt");

pub fn server_notification_methods() -> impl Iterator<Item = &'static str> {
    SERVER_NOTIFICATIONS_TEXT
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

pub fn is_known_server_notification(method: &str) -> bool {
    server_notification_methods().any(|known| known == method)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationFamily {
    Account,
    App,
    Configuration,
    Files,
    Search,
    Item,
    Mcp,
    Model,
    Process,
    RemoteControl,
    ServerRequest,
    Thread,
    Turn,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleNotification {
    pub method: String,
    pub family: NotificationFamily,
    pub severity: NotificationSeverity,
    pub title: String,
    pub detail: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub params: Option<Value>,
}

impl LifecycleNotification {
    pub fn from_known(method: &str, params: Option<&Value>) -> Option<Self> {
        if !is_known_server_notification(method) {
            return None;
        }
        let family = notification_family(method);
        let severity = if method == "error" || method.ends_with("/error") {
            NotificationSeverity::Error
        } else if method == "warning"
            || method.ends_with("Warning")
            || method == "configWarning"
            || method == "deprecationNotice"
            || method == "guardianWarning"
        {
            NotificationSeverity::Warning
        } else {
            NotificationSeverity::Info
        };
        let thread_id = text_field(params, "threadId");
        let turn_id = text_field(params, "turnId");
        let item_id = text_field(params, "itemId");
        Some(Self {
            method: method.to_owned(),
            family,
            severity,
            title: notification_title(method),
            detail: notification_detail(params),
            thread_id,
            turn_id,
            item_id,
            params: params.cloned(),
        })
    }

    /// Low-volume events that belong in the transcript activity narrative.
    pub fn is_transcript_activity(&self) -> bool {
        matches!(
            self.severity,
            NotificationSeverity::Warning | NotificationSeverity::Error
        ) || matches!(
            self.method.as_str(),
            "hook/started"
                | "hook/completed"
                | "item/autoApprovalReview/started"
                | "item/autoApprovalReview/completed"
                | "mcpServer/oauthLogin/completed"
                | "model/rerouted"
                | "model/verification"
                | "thread/compacted"
                | "thread/environment/connected"
                | "thread/environment/disconnected"
                | "thread/goal/updated"
                | "thread/goal/cleared"
        )
    }
}

fn notification_family(method: &str) -> NotificationFamily {
    if method.starts_with("account/") {
        NotificationFamily::Account
    } else if method.starts_with("app/") || method.starts_with("externalAgentConfig/") {
        NotificationFamily::App
    } else if method == "configWarning" || method == "deprecationNotice" {
        NotificationFamily::Configuration
    } else if method.starts_with("fs/") {
        NotificationFamily::Files
    } else if method.starts_with("fuzzyFileSearch/") {
        NotificationFamily::Search
    } else if method.starts_with("item/") || method.starts_with("hook/") {
        NotificationFamily::Item
    } else if method.starts_with("mcpServer/") || method == "skills/changed" {
        NotificationFamily::Mcp
    } else if method.starts_with("model/") {
        NotificationFamily::Model
    } else if method.starts_with("process/") || method.starts_with("command/exec/") {
        NotificationFamily::Process
    } else if method.starts_with("remoteControl/") {
        NotificationFamily::RemoteControl
    } else if method.starts_with("serverRequest/") {
        NotificationFamily::ServerRequest
    } else if method.starts_with("thread/") {
        NotificationFamily::Thread
    } else if method.starts_with("turn/") {
        NotificationFamily::Turn
    } else {
        NotificationFamily::System
    }
}

fn notification_title(method: &str) -> String {
    match method {
        "configWarning" => "Configuration warning".to_owned(),
        "deprecationNotice" => "Deprecation notice".to_owned(),
        "guardianWarning" => "Guardian warning".to_owned(),
        "windows/worldWritableWarning" => "World-writable directory warning".to_owned(),
        "error" => "Codex error".to_owned(),
        "warning" => "Codex warning".to_owned(),
        _ => method
            .split('/')
            .map(|part| {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" · "),
    }
}

fn notification_detail(params: Option<&Value>) -> String {
    let Some(params) = params else {
        return String::new();
    };
    for key in [
        "message",
        "summary",
        "details",
        "failureReason",
        "status",
        "query",
        "name",
    ] {
        if let Some(text) = params.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return text.to_owned();
            }
        }
    }
    if let Some(error) = params.get("error") {
        if let Some(message) = error.get("message").and_then(Value::as_str) {
            return message.to_owned();
        }
        if let Some(text) = error.as_str() {
            return text.to_owned();
        }
    }
    String::new()
}

fn text_field(params: Option<&Value>, key: &str) -> Option<String> {
    params?.get(key)?.as_str().map(str::to_owned)
}

pub fn known_notification_event(method: &str, params: Option<&Value>) -> Option<TurnStreamEvent> {
    LifecycleNotification::from_known(method, params).map(TurnStreamEvent::Lifecycle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_inventory_is_nonempty_and_categorized() {
        let methods: Vec<_> = server_notification_methods().collect();
        assert_eq!(methods.len(), 70);
        for method in methods {
            let event = crate::protocol::map_notification_to_event(method, None);
            assert!(
                !matches!(event, TurnStreamEvent::Other { .. }),
                "known notification fell through: {method}"
            );
        }
    }

    #[test]
    fn preserves_identity_and_human_detail() {
        let params = serde_json::json!({
            "threadId": "t1", "turnId": "u1", "message": "Network unavailable"
        });
        let event = LifecycleNotification::from_known("warning", Some(&params)).unwrap();
        assert_eq!(event.thread_id.as_deref(), Some("t1"));
        assert_eq!(event.detail, "Network unavailable");
        assert_eq!(event.severity, NotificationSeverity::Warning);
    }

    #[test]
    fn ambient_mcp_startup_status_does_not_pollute_the_chat_transcript() {
        let params = serde_json::json!({
            "threadId": "t1",
            "name": "example",
            "status": "ready"
        });
        let event =
            LifecycleNotification::from_known("mcpServer/startupStatus/updated", Some(&params))
                .unwrap();
        assert!(!event.is_transcript_activity());
        assert_eq!(event.family, NotificationFamily::Mcp);
    }
}
