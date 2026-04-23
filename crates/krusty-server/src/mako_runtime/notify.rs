use std::path::Path;
use std::sync::Arc;

use crate::apns::{ApnsEventType, ApnsPayload};
use crate::notifications::{
    fire_apns, fire_push, mako_session_notification_data, session_title,
    tool_approval_notification_data, APNS_CATEGORY_MAKO_SESSION, APNS_CATEGORY_TOOL_APPROVAL,
};
use crate::push::{PushEventType, PushPayload};

pub(super) fn mako_notification_title(title: Option<&str>, session_label: &str) -> String {
    let label = title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(session_label);
    format!("Mako — {label}")
}

pub(super) fn notify_mako_user_message(
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
    let notification_title = mako_notification_title(title, &session_label);
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: notification_title.clone(),
            body: message.to_string(),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("mako-{session_id}")),
        },
        PushEventType::MakoUpdate,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: notification_title,
            body: message.to_string(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_MAKO_SESSION.into()),
            data: Some(mako_session_notification_data(
                "user_message",
                session_id,
                Some(level),
                title,
            )),
        },
        ApnsEventType::MakoUpdate,
    );
}

pub(super) fn notify_mako_awaiting_input(
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Mako".into(),
            body: "Mako needs your input".into(),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("mako-{session_id}")),
        },
        PushEventType::AwaitingInput,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Mako".into(),
            body: "Mako needs your input".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_MAKO_SESSION.into()),
            data: Some(mako_session_notification_data(
                "awaiting_input",
                session_id,
                Some("warning"),
                None,
            )),
        },
        ApnsEventType::AwaitingInput,
    );
}

pub(super) fn notify_mako_tool_approval(
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
    request_id: &str,
    tool_name: &str,
) {
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Tool Approval Required".into(),
            body: format!("Mako wants to run \"{tool_name}\"."),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_TOOL_APPROVAL.into()),
            data: Some(tool_approval_notification_data(
                request_id, session_id, tool_name, "mako",
            )),
        },
        ApnsEventType::ToolApproval,
    );
}

pub(super) fn notify_mako_error(
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
            title: "Mako".into(),
            body: format!("{session_label} encountered an error"),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("mako-{session_id}")),
        },
        PushEventType::Error,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: format!(
                "{} — Attention needed",
                mako_notification_title(None, &session_label)
            ),
            body: "Run encountered an error".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_MAKO_SESSION.into()),
            data: Some(mako_session_notification_data(
                "error",
                session_id,
                Some("error"),
                None,
            )),
        },
        ApnsEventType::Error,
    );
}

pub(super) fn notify_mako_completion(
    db_path: &Path,
    push_service: &Option<Arc<crate::push::PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    let session_label = session_title(db_path, session_id);
    let notification_title = format!(
        "{} — Complete",
        mako_notification_title(None, &session_label)
    );
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Mako".into(),
            body: format!("{session_label} is complete"),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("mako-{session_id}")),
        },
        PushEventType::MakoUpdate,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: notification_title,
            body: "Run finished".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_MAKO_SESSION.into()),
            data: Some(mako_session_notification_data(
                "completion",
                session_id,
                Some("success"),
                None,
            )),
        },
        ApnsEventType::MakoUpdate,
    );
}
