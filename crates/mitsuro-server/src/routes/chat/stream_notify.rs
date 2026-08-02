use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use mitsuro_core::agent::loop_events::LoopStopReason;
use mitsuro_core::agent::LoopEvent;

use crate::apns::{ApnsEventType, ApnsPayload};
use crate::notifications::{
    chat_session_notification_data, fire_apns, fire_push, notification_terminal_disposition,
    session_title, tool_approval_notification_data, NotificationTerminalDisposition,
    APNS_CATEGORY_CHAT_SESSION, APNS_CATEGORY_TOOL_APPROVAL,
};
use crate::push::{PushEventType, PushPayload, PushService};

#[derive(Default)]
pub(super) struct ChatStreamRunOutcome {
    awaiting_input: bool,
    had_error: bool,
    stop_reason: Option<LoopStopReason>,
    notified_tool_approvals: HashSet<String>,
}

impl ChatStreamRunOutcome {
    pub(super) fn record_event(
        &mut self,
        push_service: &Option<Arc<PushService>>,
        apns_service: &Option<Arc<crate::apns::ApnsService>>,
        user_id: Option<&str>,
        session_id: &str,
        event: &LoopEvent,
    ) {
        if let LoopEvent::Finished {
            stop_reason: ref reason,
            ..
        } = event
        {
            self.stop_reason = Some(reason.clone());
        }

        if matches!(event, LoopEvent::AwaitingInput { .. }) && !self.awaiting_input {
            self.awaiting_input = true;
            notify_chat_awaiting_input(push_service, apns_service, user_id, session_id);
        }

        if let LoopEvent::ToolApprovalRequired {
            ref id, ref name, ..
        } = event
        {
            if self.notified_tool_approvals.insert(id.clone()) {
                notify_chat_tool_approval(
                    push_service,
                    apns_service,
                    user_id,
                    session_id,
                    id,
                    name,
                );
            }
        }

        if matches!(event, LoopEvent::Error { .. }) {
            self.had_error = true;
        }
    }

    pub(super) fn finalize(
        self,
        push_service: &Option<Arc<PushService>>,
        apns_service: &Option<Arc<crate::apns::ApnsService>>,
        user_id: Option<&str>,
        session_id: &str,
        db_path: &Path,
    ) {
        if self.awaiting_input {
            return;
        }

        let disposition = if self.had_error {
            NotificationTerminalDisposition::Attention
        } else {
            notification_terminal_disposition(self.stop_reason.as_ref())
        };
        match disposition {
            NotificationTerminalDisposition::Complete => {
                notify_chat_completion(push_service, apns_service, user_id, session_id, db_path);
            }
            NotificationTerminalDisposition::Attention => {
                notify_chat_error(push_service, apns_service, user_id, session_id);
            }
            NotificationTerminalDisposition::Skip => {
                tracing::info!(
                    session_id = %session_id,
                    stop_reason = ?self.stop_reason,
                    "Session did not complete; skipping completion push"
                );
            }
        }
    }
}

fn notify_chat_awaiting_input(
    push_service: &Option<Arc<PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Mitsuro".into(),
            body: "Agent needs your input".into(),
            session_id: Some(session_id.to_string()),
            tag: None,
            category: Some(APNS_CATEGORY_CHAT_SESSION.into()),
            data: Some(chat_session_notification_data("awaiting_input", session_id)),
        },
        PushEventType::AwaitingInput,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Mitsuro".into(),
            body: "Agent needs your input".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_CHAT_SESSION.into()),
            data: Some(chat_session_notification_data("awaiting_input", session_id)),
        },
        ApnsEventType::AwaitingInput,
    );
}

fn notify_chat_tool_approval(
    push_service: &Option<Arc<PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
    request_id: &str,
    tool_name: &str,
) {
    let data = tool_approval_notification_data(request_id, session_id, tool_name, "chat");
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Permission Required".into(),
            body: format!("\"{tool_name}\" is requesting permission to execute."),
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
            title: "Permission Required".into(),
            body: format!("\"{tool_name}\" is requesting permission to execute."),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_TOOL_APPROVAL.into()),
            data: Some(data),
        },
        ApnsEventType::ToolApproval,
    );
}

fn notify_chat_error(
    push_service: &Option<Arc<PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
) {
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Mitsuro".into(),
            body: "Session encountered an error".into(),
            session_id: Some(session_id.to_string()),
            tag: None,
            category: Some(APNS_CATEGORY_CHAT_SESSION.into()),
            data: Some(chat_session_notification_data("error", session_id)),
        },
        PushEventType::Error,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Mitsuro".into(),
            body: "Session encountered an error".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_CHAT_SESSION.into()),
            data: Some(chat_session_notification_data("error", session_id)),
        },
        ApnsEventType::Error,
    );
}

fn notify_chat_completion(
    push_service: &Option<Arc<PushService>>,
    apns_service: &Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<&str>,
    session_id: &str,
    db_path: &Path,
) {
    let title = session_title(db_path, session_id);
    fire_push(
        push_service,
        user_id,
        PushPayload {
            title: "Mitsuro".into(),
            body: format!("{title} is complete"),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("session-{session_id}")),
            category: Some(APNS_CATEGORY_CHAT_SESSION.into()),
            data: Some(chat_session_notification_data("completion", session_id)),
        },
        PushEventType::Completion,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: format!("{title} — Complete"),
            body: "Response finished".into(),
            session_id: Some(session_id.to_string()),
            category: Some(APNS_CATEGORY_CHAT_SESSION.into()),
            data: Some(chat_session_notification_data("completion", session_id)),
        },
        ApnsEventType::Completion,
    );
}
