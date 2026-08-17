use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;

use mitsuro_core::agent::loop_events::LoopStopReason;
use mitsuro_core::agent::LoopEvent;

use super::notify::{
    notify_hive_awaiting_input, notify_hive_completion, notify_hive_error, notify_hive_partial,
    notify_hive_tool_approval, notify_hive_user_message,
};
use super::state::refresh_snapshot_after_run;
use super::{HiveRuntimeManager, WakeCommand};
use crate::notifications::{notification_terminal_disposition, NotificationTerminalDisposition};
use crate::AppState;

#[derive(Default)]
pub(super) struct HiveRunOutcome {
    awaiting_input: bool,
    had_error: bool,
    pinched_session_id: Option<String>,
    stop_reason: Option<LoopStopReason>,
    sent_user_message: bool,
    notified_tool_approvals: HashSet<String>,
}

impl HiveRunOutcome {
    pub(super) fn allows_learning_review(&self) -> bool {
        !self.had_error && !self.awaiting_input
    }

    pub(super) async fn record_event(
        &mut self,
        state: &AppState,
        manager: &Arc<HiveRuntimeManager>,
        session_id: &str,
        user_id: Option<&str>,
        event: &LoopEvent,
        allow_embedded_wakes: bool,
    ) {
        if let LoopEvent::Finished {
            stop_reason: ref reason,
            ..
        } = event
        {
            self.stop_reason = Some(reason.clone());
        }

        if allow_embedded_wakes {
            if let LoopEvent::AgentSleeping { duration_secs, .. } = event {
                let wake_at = chrono::Utc::now() + chrono::Duration::seconds(*duration_secs as i64);
                manager
                    .schedule_wake_at(state.clone(), session_id.to_string(), wake_at, "sleep")
                    .await;
            }
        }

        if matches!(event, LoopEvent::AwaitingInput { .. }) && !self.awaiting_input {
            self.awaiting_input = true;
            notify_hive_awaiting_input(
                state.db_path.as_ref(),
                &state.push_service,
                &state.apns_service,
                user_id,
                session_id,
            );
        }

        if let LoopEvent::ToolApprovalRequired {
            ref id, ref name, ..
        } = event
        {
            if self.notified_tool_approvals.insert(id.clone()) {
                notify_hive_tool_approval(
                    state.db_path.as_ref(),
                    &state.push_service,
                    &state.apns_service,
                    user_id,
                    session_id,
                    id,
                    name,
                );
            }
        }

        if let LoopEvent::UserMessage {
            ref title,
            ref message,
            ref level,
        } = event
        {
            self.sent_user_message = true;
            notify_hive_user_message(
                state.db_path.as_ref(),
                &state.push_service,
                &state.apns_service,
                user_id,
                session_id,
                title.as_deref(),
                message,
                level,
            );
        }

        if let LoopEvent::SessionPinched {
            ref new_session_id, ..
        } = event
        {
            self.pinched_session_id = Some(new_session_id.clone());
        }

        if let LoopEvent::Error { .. } = event {
            self.had_error = true;
        }
    }

    pub(super) async fn finalize(
        self,
        state: &AppState,
        manager: &Arc<HiveRuntimeManager>,
        session_id: &str,
        user_id: Option<&str>,
        project_scope: Option<&str>,
        allow_embedded_wakes: bool,
    ) -> Result<()> {
        refresh_snapshot_after_run(
            state.db_path.as_ref(),
            project_scope,
            user_id,
            self.stop_reason.as_ref(),
        );

        if allow_embedded_wakes {
            if let Some(new_session_id) = self.pinched_session_id {
                let _ = manager.wake_tx.send(WakeCommand {
                    state: state.clone(),
                    session_id: new_session_id,
                    wake_reason: "pinch".to_string(),
                });
                return Ok(());
            }
        }

        if !self.awaiting_input {
            let disposition = if self.had_error {
                NotificationTerminalDisposition::Attention
            } else {
                notification_terminal_disposition(self.stop_reason.as_ref())
            };
            match disposition {
                NotificationTerminalDisposition::Attention => {
                    notify_hive_error(
                        state.db_path.as_ref(),
                        &state.push_service,
                        &state.apns_service,
                        user_id,
                        session_id,
                    );
                }
                NotificationTerminalDisposition::Skip => {
                    tracing::info!(
                        session_id = %session_id,
                        stop_reason = ?self.stop_reason,
                        "Hive session did not complete; skipping completion push"
                    );
                }
                NotificationTerminalDisposition::Partial => {
                    notify_hive_partial(
                        state.db_path.as_ref(),
                        &state.push_service,
                        &state.apns_service,
                        user_id,
                        session_id,
                    );
                }
                NotificationTerminalDisposition::Complete if !self.sent_user_message => {
                    notify_hive_completion(
                        state.db_path.as_ref(),
                        &state.push_service,
                        &state.apns_service,
                        user_id,
                        session_id,
                    );
                }
                NotificationTerminalDisposition::Complete => {}
            }
        }

        Ok(())
    }
}
