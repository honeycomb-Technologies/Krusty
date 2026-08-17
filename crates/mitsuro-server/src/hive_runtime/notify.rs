use std::path::Path;
use std::sync::Arc;

use crate::apns::{ApnsEventType, ApnsPayload};
use crate::notifications::{
    fire_apns, fire_push, hive_session_notification_data, resolve_hive_focus, session_title,
    tool_approval_notification_data, with_hive_focus_ids, APNS_CATEGORY_HIVE_SESSION,
    APNS_CATEGORY_TOOL_APPROVAL,
};

fn hive_update_data(
    db_path: &Path,
    kind: &str,
    session_id: &str,
    level: Option<&str>,
    title: Option<&str>,
) -> serde_json::Value {
    let (worker_id, group_id) = resolve_hive_focus(db_path, session_id);
    with_hive_focus_ids(
        hive_session_notification_data(kind, session_id, level, title),
        worker_id.as_deref(),
        group_id.as_deref(),
    )
}

fn hive_approval_data(
    db_path: &Path,
    request_id: &str,
    session_id: &str,
    tool_name: &str,
) -> serde_json::Value {
    let (worker_id, group_id) = resolve_hive_focus(db_path, session_id);
    with_hive_focus_ids(
        tool_approval_notification_data(request_id, session_id, tool_name, "hive"),
        worker_id.as_deref(),
        group_id.as_deref(),
    )
}
use crate::push::{PushEventType, PushPayload};

pub(super) fn hive_notification_title(title: Option<&str>, session_label: &str) -> String {
    let label = title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(session_label);
    format!("Hive — {label}")
}

pub(super) fn notify_hive_user_message(
    db_path: &Path,
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
    title: Option<&str>,
    message: &str,
    level: &str,
) {
    let session_label = session_title(db_path, session_id);
    let notification_title = hive_notification_title(title, &session_label);
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: notification_title.clone(),
            body: message.to_string(),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("hive-{session_id}")),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "user_message",
                session_id,
                Some(level),
                title,
            )),
        },
        PushEventType::HiveUpdate,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: notification_title,
            body: message.to_string(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "user_message",
                session_id,
                Some(level),
                title,
            )),
        },
        ApnsEventType::HiveUpdate,
    );
}

pub(super) fn notify_hive_awaiting_input(
    db_path: &Path,
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Hive".into(),
            body: "Hive needs your input".into(),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("hive-{session_id}")),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "awaiting_input",
                session_id,
                Some("warning"),
                None,
            )),
        },
        PushEventType::AwaitingInput,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Hive".into(),
            body: "Hive needs your input".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "awaiting_input",
                session_id,
                Some("warning"),
                None,
            )),
        },
        ApnsEventType::AwaitingInput,
    );
}

pub(super) fn notify_hive_tool_approval(
    db_path: &Path,
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
    request_id: &str,
    tool_name: &str,
) {
    let data = hive_approval_data(db_path, request_id, session_id, tool_name);
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Tool Approval Required".into(),
            body: format!("Hive wants to run \"{tool_name}\"."),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("approval-{request_id}")),
            category: Some(APNS_CATEGORY_TOOL_APPROVAL.into()),
            data: Some(data.clone()),
        },
        PushEventType::ToolApproval,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Tool Approval Required".into(),
            body: format!("Hive wants to run \"{tool_name}\"."),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_TOOL_APPROVAL.into()),
            data: Some(data),
        },
        ApnsEventType::ToolApproval,
    );
}

pub(super) fn notify_hive_error(
    db_path: &Path,
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    let session_label = session_title(db_path, session_id);
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Hive".into(),
            body: format!("{session_label} encountered an error"),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("hive-{session_id}")),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "error",
                session_id,
                Some("error"),
                None,
            )),
        },
        PushEventType::Error,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: format!(
                "{} — Attention needed",
                hive_notification_title(None, &session_label)
            ),
            body: "Run encountered an error".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "error",
                session_id,
                Some("error"),
                None,
            )),
        },
        ApnsEventType::Error,
    );
}

pub(super) fn notify_hive_completion(
    db_path: &Path,
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    let session_label = session_title(db_path, session_id);
    let notification_title = format!(
        "{} — Complete",
        hive_notification_title(None, &session_label)
    );
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Hive".into(),
            body: format!("{session_label} is complete"),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("hive-{session_id}")),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "completion",
                session_id,
                Some("success"),
                None,
            )),
        },
        PushEventType::HiveUpdate,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: notification_title,
            body: "Run finished".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(hive_update_data(
                db_path,
                "completion",
                session_id,
                Some("success"),
                None,
            )),
        },
        ApnsEventType::HiveUpdate,
    );
}

pub(super) fn notify_hive_partial(
    db_path: &Path,
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    let session_label = session_title(db_path, session_id);
    let data = hive_update_data(db_path, "partial", session_id, Some("warning"), None);
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Hive".into(),
            body: format!("{session_label} finished with partial results"),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("hive-{session_id}")),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(data.clone()),
        },
        PushEventType::HiveUpdate,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: format!(
                "{} — Partial",
                hive_notification_title(None, &session_label)
            ),
            body: "Review the remaining work".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_HIVE_SESSION.into()),
            data: Some(data),
        },
        ApnsEventType::HiveUpdate,
    );
}
