use std::path::Path;

use crate::agent::context_ledger::ContextLedger;
use crate::ai::types::{ModelMessage, Role};
use crate::storage::{Database, SessionManager, SessionRecoveryState};

pub(super) fn save_message(db_path: &Path, session_id: &str, message: &ModelMessage) {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        _ => return,
    };

    match serde_json::to_string(&message.content) {
        Ok(json) => {
            let Some(session_manager) = open_session_manager(db_path, "saving message") else {
                return;
            };
            if let Err(e) = session_manager.save_message(session_id, role, &json) {
                tracing::error!("Failed to save message: {}", e);
            }
        }
        Err(e) => tracing::error!("Failed to serialize message: {}", e),
    }
}

pub(super) fn persist_context_state(db_path: &Path, session_id: &str, ledger: &ContextLedger) {
    let ledger_json = match serde_json::to_string(&ledger.persistence_record()) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                "Failed to serialize context ledger snapshot: {}", e
            );
            return;
        }
    };

    let continuation_json = match serde_json::to_string(&ledger.continuation_contract()) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                "Failed to serialize continuation contract: {}", e
            );
            return;
        }
    };

    let Some(session_manager) =
        open_session_manager(db_path, "persisting context continuation state")
    else {
        return;
    };
    if let Err(e) = session_manager.update_context_continuation_state(
        session_id,
        &ledger_json,
        &continuation_json,
    ) {
        tracing::warn!(
            session_id = %session_id,
            "Failed to persist context continuation state: {}", e
        );
    }
}

pub(super) fn persist_recovery_state(
    db_path: &Path,
    session_id: &str,
    recovery: &SessionRecoveryState,
) {
    update_recovery_state(db_path, session_id, Some(recovery));
}

pub(super) fn clear_recovery_state(db_path: &Path, session_id: &str) {
    update_recovery_state(db_path, session_id, None);
}

pub(super) fn save_title(db_path: &Path, session_id: &str, title: &str) {
    let Some(session_manager) = open_session_manager(db_path, "saving title") else {
        return;
    };
    if let Err(e) = session_manager.update_session_title(session_id, title) {
        tracing::warn!(session_id = %session_id, "Failed to save title: {}", e);
    }
}

pub(super) fn set_agent_state(db_path: &Path, session_id: &str, state: &str) {
    let Some(session_manager) = open_session_manager(db_path, "setting agent state") else {
        return;
    };
    if let Err(e) = session_manager.set_agent_state(session_id, state) {
        tracing::warn!(
            session_id = %session_id,
            "Failed to set agent state '{state}': {}", e
        );
    }
}

pub(super) fn update_token_count(db_path: &Path, session_id: &str, token_count: usize) {
    let Some(session_manager) = open_session_manager(db_path, "updating token count") else {
        return;
    };
    if let Err(e) = session_manager.update_token_count(session_id, token_count) {
        tracing::warn!(
            session_id = %session_id,
            "Failed to update token count: {}", e
        );
    }
}

fn update_recovery_state(
    db_path: &Path,
    session_id: &str,
    recovery: Option<&SessionRecoveryState>,
) {
    let action = if recovery.is_some() {
        "persisting recovery state"
    } else {
        "clearing recovery state"
    };
    let Some(session_manager) = open_session_manager(db_path, action) else {
        return;
    };

    let result = match recovery {
        Some(recovery) => session_manager.update_recovery_state(session_id, recovery),
        None => session_manager.clear_recovery_state(session_id),
    };
    if let Err(e) = result {
        let verb = if recovery.is_some() {
            "persist"
        } else {
            "clear"
        };
        tracing::warn!(session_id = %session_id, "Failed to {verb} recovery state: {}", e);
    }
}

fn open_session_manager(db_path: &Path, action: &str) -> Option<SessionManager> {
    match Database::new(db_path) {
        Ok(db) => Some(SessionManager::new(db)),
        Err(e) => {
            tracing::error!("Failed to open database while {}: {}", action, e);
            None
        }
    }
}
