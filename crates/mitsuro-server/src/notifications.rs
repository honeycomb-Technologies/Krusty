use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use mitsuro_core::agent::loop_events::LoopStopReason;
use mitsuro_core::storage::Database;
use mitsuro_core::SessionManager;

use crate::apns::{ApnsEventType, ApnsPayload, ApnsService};
use crate::push::{PushEventType, PushPayload, PushService};

pub(crate) const APNS_CATEGORY_TOOL_APPROVAL: &str = "TOOL_APPROVAL";
pub(crate) const APNS_CATEGORY_CHAT_SESSION: &str = "CHAT_SESSION";
pub(crate) const APNS_CATEGORY_HIVE_SESSION: &str = "HIVE_SESSION";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotificationTerminalDisposition {
    Complete,
    Partial,
    Attention,
    Skip,
}

pub(crate) fn notification_terminal_disposition(
    stop_reason: Option<&LoopStopReason>,
) -> NotificationTerminalDisposition {
    match stop_reason {
        Some(LoopStopReason::Completed) => NotificationTerminalDisposition::Complete,
        Some(LoopStopReason::BudgetExhausted | LoopStopReason::LoopGuardTriggered) => {
            NotificationTerminalDisposition::Partial
        }
        Some(
            LoopStopReason::ProviderError
            | LoopStopReason::StreamIdleTimeout
            | LoopStopReason::PinchFailed,
        )
        | None => NotificationTerminalDisposition::Attention,
        Some(
            LoopStopReason::AwaitingInput
            | LoopStopReason::Sleeping
            | LoopStopReason::UserAbort
            | LoopStopReason::Pinched,
        ) => NotificationTerminalDisposition::Skip,
    }
}

pub(crate) fn session_title(db_path: &Path, session_id: &str) -> String {
    match Database::new(db_path) {
        Ok(db) => {
            let session_manager = SessionManager::new(db);
            match session_manager.get_session(session_id) {
                Ok(Some(session)) => {
                    let title = session.title.trim();
                    if title.is_empty() {
                        "Session".to_string()
                    } else {
                        title.to_string()
                    }
                }
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

pub(crate) fn hive_session_notification_data(
    kind: &str,
    session_id: &str,
    level: Option<&str>,
    title: Option<&str>,
) -> Value {
    let mut data = json!({
        "type": "hive_update",
        "kind": kind,
        "sessionId": session_id,
        "focus": "hive",
    });

    if let Some(level) = level.filter(|value| !value.trim().is_empty()) {
        data["level"] = Value::String(level.to_string());
    }

    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        data["title"] = Value::String(title.to_string());
    }

    data
}

pub(crate) fn with_hive_focus_ids(
    mut data: Value,
    worker_id: Option<&str>,
    group_id: Option<&str>,
) -> Value {
    if let Some(id) = worker_id.map(str::trim).filter(|value| !value.is_empty()) {
        data["workerId"] = Value::String(id.to_string());
    }
    if let Some(id) = group_id.map(str::trim).filter(|value| !value.is_empty()) {
        data["groupId"] = Value::String(id.to_string());
    }
    data
}

pub(crate) fn resolve_hive_focus(
    db_path: &Path,
    session_id: &str,
) -> (Option<String>, Option<String>) {
    let Ok(db) = Database::new(db_path) else {
        return (None, None);
    };
    let worker_id = db
        .conn()
        .query_row(
            "SELECT id FROM hive_workers WHERE dm_session_id = ?1 LIMIT 1",
            [session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let group_id = db
        .conn()
        .query_row(
            "SELECT group_id FROM hive_runs
             WHERE session_id = ?1 AND group_id IS NOT NULL
             ORDER BY created_at DESC LIMIT 1",
            [session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    (worker_id, group_id)
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
            let stats = svc
                .notify_user_durable(uid.as_deref(), payload, event_type)
                .await;
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
            let live_payload = payload.clone();
            let (stats, live_stats) = tokio::join!(
                svc.notify_user_durable(uid.as_deref(), payload, event_type),
                svc.notify_live_activities(uid.as_deref(), &live_payload, event_type),
            );
            tracing::info!(
                event_type = event_type.as_str(),
                attempted = stats.attempted,
                sent = stats.sent,
                stale_removed = stats.stale_removed,
                failed = stats.failed,
                live_activity_attempted = live_stats.attempted,
                live_activity_sent = live_stats.sent,
                live_activity_failed = live_stats.failed,
                "APNs event dispatched"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chat_session_notification_data, hive_session_notification_data,
        notification_terminal_disposition, tool_approval_notification_data, with_hive_focus_ids,
        NotificationTerminalDisposition,
    };
    use mitsuro_core::agent::loop_events::LoopStopReason;

    #[test]
    fn chat_session_notification_data_carries_focus_and_kind() {
        let data = chat_session_notification_data("completion", "session-1");

        assert_eq!(data["type"], "chat_update");
        assert_eq!(data["kind"], "completion");
        assert_eq!(data["sessionId"], "session-1");
        assert_eq!(data["focus"], "chat");
    }

    #[test]
    fn hive_session_notification_data_carries_focus_kind_and_metadata() {
        let data =
            hive_session_notification_data("user_message", "session-2", Some("info"), Some("Crew"));

        assert_eq!(data["type"], "hive_update");
        assert_eq!(data["kind"], "user_message");
        assert_eq!(data["sessionId"], "session-2");
        assert_eq!(data["focus"], "hive");
        assert_eq!(data["level"], "info");
        assert_eq!(data["title"], "Crew");
    }

    #[test]
    fn tool_approval_notification_data_carries_focus() {
        let data = tool_approval_notification_data("req-1", "session-3", "bash", "hive");

        assert_eq!(data["type"], "tool_approval");
        assert_eq!(data["requestId"], "req-1");
        assert_eq!(data["sessionId"], "session-3");
        assert_eq!(data["toolName"], "bash");
        assert_eq!(data["focus"], "hive");
    }

    #[test]
    fn hive_notification_data_can_carry_worker_and_group_ids() {
        let data = with_hive_focus_ids(
            hive_session_notification_data("user_message", "session-2", Some("info"), Some("Crew")),
            Some("worker-9"),
            Some("group-3"),
        );

        assert_eq!(data["workerId"], "worker-9");
        assert_eq!(data["groupId"], "group-3");
        assert_eq!(data["sessionId"], "session-2");
        assert_eq!(data["focus"], "hive");
    }

    #[test]
    fn only_completed_runs_are_reported_as_complete() {
        assert_eq!(
            notification_terminal_disposition(Some(&LoopStopReason::Completed)),
            NotificationTerminalDisposition::Complete
        );

        for reason in [
            None,
            Some(LoopStopReason::ProviderError),
            Some(LoopStopReason::StreamIdleTimeout),
            Some(LoopStopReason::PinchFailed),
        ] {
            assert_eq!(
                notification_terminal_disposition(reason.as_ref()),
                NotificationTerminalDisposition::Attention
            );
        }

        for reason in [
            LoopStopReason::BudgetExhausted,
            LoopStopReason::LoopGuardTriggered,
        ] {
            assert_eq!(
                notification_terminal_disposition(Some(&reason)),
                NotificationTerminalDisposition::Partial
            );
        }

        for reason in [
            LoopStopReason::AwaitingInput,
            LoopStopReason::Sleeping,
            LoopStopReason::UserAbort,
            LoopStopReason::Pinched,
        ] {
            assert_eq!(
                notification_terminal_disposition(Some(&reason)),
                NotificationTerminalDisposition::Skip
            );
        }
    }
}
