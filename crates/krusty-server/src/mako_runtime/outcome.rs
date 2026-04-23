use std::sync::Arc;

use anyhow::Result;

use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::agent::LoopEvent;

use super::notify::{
    notify_mako_awaiting_input, notify_mako_completion, notify_mako_error,
    notify_mako_tool_approval, notify_mako_user_message,
};
use super::state::refresh_snapshot_after_run;
use super::{MakoRuntimeManager, WakeCommand};
use crate::AppState;

#[derive(Default)]
pub(super) struct MakoRunOutcome {
    awaiting_input: bool,
    had_error: bool,
    pinched_session_id: Option<String>,
    stop_reason: Option<LoopStopReason>,
    sent_user_message: bool,
}

impl MakoRunOutcome {
    pub(super) async fn record_event(
        &mut self,
        state: &AppState,
        manager: &Arc<MakoRuntimeManager>,
        session_id: &str,
        user_id: Option<&str>,
        event: &LoopEvent,
    ) {
        if let LoopEvent::Finished {
            stop_reason: ref reason,
            ..
        } = event
        {
            self.stop_reason = Some(reason.clone());
        }

        if let LoopEvent::AgentSleeping { duration_secs, .. } = event {
            let wake_at = chrono::Utc::now() + chrono::Duration::seconds(*duration_secs as i64);
            manager
                .schedule_wake_at(state.clone(), session_id.to_string(), wake_at, "sleep")
                .await;
        }

        if matches!(event, LoopEvent::AwaitingInput { .. }) {
            self.awaiting_input = true;
            notify_mako_awaiting_input(
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
            notify_mako_tool_approval(&state.apns_service, user_id, session_id, id, name);
        }

        if let LoopEvent::UserMessage {
            ref title,
            ref message,
            ref level,
        } = event
        {
            self.sent_user_message = true;
            notify_mako_user_message(
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
        manager: &Arc<MakoRuntimeManager>,
        session_id: &str,
        user_id: Option<&str>,
        project_scope: Option<&str>,
    ) -> Result<()> {
        refresh_snapshot_after_run(
            state.db_path.as_ref(),
            project_scope,
            user_id,
            self.stop_reason.as_ref(),
        );

        if let Some(new_session_id) = self.pinched_session_id {
            let _ = manager.wake_tx.send(WakeCommand {
                state: state.clone(),
                session_id: new_session_id,
                wake_reason: "pinch".to_string(),
            });
            return Ok(());
        }

        if !self.awaiting_input {
            if self.had_error {
                notify_mako_error(
                    state.db_path.as_ref(),
                    &state.push_service,
                    &state.apns_service,
                    user_id,
                    session_id,
                );
            } else if self.stop_reason == Some(LoopStopReason::Sleeping) {
                tracing::info!(
                    session_id = %session_id,
                    "Mako session entered sleeping state; skipping completion push"
                );
            } else if !self.sent_user_message {
                notify_mako_completion(
                    state.db_path.as_ref(),
                    &state.push_service,
                    &state.apns_service,
                    user_id,
                    session_id,
                );
            }
        }

        Ok(())
    }
}
