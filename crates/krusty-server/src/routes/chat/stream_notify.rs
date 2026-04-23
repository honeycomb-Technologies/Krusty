use std::path::Path;
use std::sync::Arc;

use serde_json::json;

use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::agent::LoopEvent;

use crate::apns::{ApnsEventType, ApnsPayload};
use crate::notifications::{fire_apns, fire_push, session_title};
use crate::push::{PushEventType, PushPayload, PushService};

#[derive(Default)]
pub(super) struct ChatStreamRunOutcome {
    awaiting_input: bool,
    had_error: bool,
    stop_reason: Option<LoopStopReason>,
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

        if matches!(event, LoopEvent::AwaitingInput { .. }) {
            self.awaiting_input = true;
            notify_chat_awaiting_input(push_service, apns_service, user_id, session_id);
        }

        if let LoopEvent::ToolApprovalRequired {
            ref id, ref name, ..
        } = event
        {
            notify_chat_tool_approval(apns_service, user_id, session_id, id, name);
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

        if self.had_error {
            notify_chat_error(push_service, apns_service, user_id, session_id);
        } else if self.stop_reason == Some(LoopStopReason::Sleeping) {
            tracing::info!(
                session_id = %session_id,
                "Session entered sleeping state; skipping completion push"
            );
        } else {
            notify_chat_completion(push_service, apns_service, user_id, session_id, db_path);
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
            title: "Krusty".into(),
            body: "Krusty needs your input".into(),
            session_id: Some(session_id.to_string()),
            tag: None,
        },
        PushEventType::AwaitingInput,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Krusty".into(),
            body: "Krusty needs your input".into(),
            session_id: Some(session_id.to_string()),
            category: Some("TOOL_APPROVAL".into()),
            data: Some(json!({
                "type": "awaiting_input",
                "sessionId": session_id,
            })),
        },
        ApnsEventType::AwaitingInput,
    );
}

fn notify_chat_tool_approval(
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
            title: "Permission Required".into(),
            body: format!("\"{tool_name}\" is requesting permission to execute."),
            session_id: Some(session_id.to_string()),
            category: Some("TOOL_APPROVAL".into()),
            data: Some(json!({
                "requestId": request_id,
                "sessionId": session_id,
                "toolName": tool_name,
                "type": "tool_approval",
            })),
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
            title: "Krusty".into(),
            body: "Session encountered an error".into(),
            session_id: Some(session_id.to_string()),
            tag: None,
        },
        PushEventType::Error,
    );
    fire_apns(
        apns_service,
        user_id,
        ApnsPayload {
            title: "Krusty".into(),
            body: "Session encountered an error".into(),
            session_id: Some(session_id.to_string()),
            category: None,
            data: Some(json!({
                "type": "error",
                "sessionId": session_id,
            })),
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
            title: "Krusty".into(),
            body: format!("{title} is complete"),
            session_id: Some(session_id.to_string()),
            tag: Some(format!("session-{session_id}")),
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
            category: Some("STREAM_COMPLETE".into()),
            data: Some(json!({
                "type": "stream_complete",
                "sessionId": session_id,
            })),
        },
        ApnsEventType::Completion,
    );
}
