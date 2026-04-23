use crate::storage::SessionRecoveryState;
use crate::tui::app::App;

impl App {
    pub(crate) fn load_persisted_recovery_state(
        &self,
        session_id: &str,
    ) -> Option<SessionRecoveryState> {
        self.services
            .session_manager
            .as_ref()
            .and_then(|sm| sm.load_recovery_state(session_id).ok().flatten())
    }

    pub(crate) fn push_recovery_notice(
        &mut self,
        recovery_state: &SessionRecoveryState,
        detail: Option<String>,
    ) {
        let mut message = recovery_state.notice();
        if let Some(detail) = detail {
            message.push_str(&detail);
        }
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), message));
    }
}
