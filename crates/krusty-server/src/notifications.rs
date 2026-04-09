use std::path::Path;
use std::sync::Arc;

use krusty_core::storage::Database;
use krusty_core::SessionManager;

use crate::apns::{ApnsEventType, ApnsPayload, ApnsService};
use crate::push::{PushEventType, PushPayload, PushService};

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
