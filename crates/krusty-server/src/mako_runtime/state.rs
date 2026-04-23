use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::agent::{LoopEvent, LoopInput};
use krusty_core::ai::types::{ModelMessage, Role};
use krusty_core::storage::{
    refresh_current_snapshot, Database, MakoRunPriority, MakoRuntimeStateStatus,
    MakoRuntimeStateStore, SessionManager, SessionType,
};

pub(super) fn refresh_snapshot_after_run(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    stop_reason: Option<&LoopStopReason>,
) {
    if matches!(stop_reason, Some(LoopStopReason::Sleeping) | None) {
        return;
    }

    if let Err(error) = refresh_current_snapshot(db_path, project_dir, user_id) {
        tracing::warn!(
            ?error,
            project_dir,
            "Failed to refresh Mako current snapshot after run"
        );
    }
}

pub(super) fn ensure_runnable_mako_session(db_path: &Path, session_id: &str) -> Result<()> {
    let session_manager = SessionManager::new(Database::new(db_path)?);
    let session = session_manager
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;
    if session.session_type != SessionType::Mako {
        anyhow::bail!("session is not a mako session");
    }
    Ok(())
}

pub(super) async fn with_registered_session_input<T, F>(
    session_inputs: Arc<RwLock<crate::SessionInputMap>>,
    session_id: String,
    input_tx: tokio::sync::mpsc::UnboundedSender<LoopInput>,
    future: F,
) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    session_inputs
        .write()
        .await
        .insert(session_id.clone(), input_tx);
    let result = future.await;
    session_inputs.write().await.remove(&session_id);
    result
}

pub(super) fn load_conversation(raw_messages: Vec<(String, String)>) -> Vec<ModelMessage> {
    raw_messages
        .into_iter()
        .filter_map(|(role_str, content_json)| {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            serde_json::from_str(&content_json)
                .ok()
                .map(|content| ModelMessage { role, content })
        })
        .collect()
}

pub(super) fn resolve_persisted_project_dir(
    stored_project_dir: Option<&str>,
    workspace_base: &Path,
) -> Option<PathBuf> {
    let raw = stored_project_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let candidate = PathBuf::from(raw);
    Some(if candidate.is_absolute() {
        candidate
    } else {
        workspace_base.join(candidate)
    })
}

pub(super) fn apply_runtime_event_state(
    db_path: &Path,
    session_id: &str,
    run_id: &str,
    event: &LoopEvent,
) -> Result<()> {
    let store = MakoRuntimeStateStore::new(Database::new(db_path)?);
    let existing_state = store.get_state(session_id)?;
    let existing_wake_reason = existing_state
        .as_ref()
        .and_then(|state| state.last_wake_reason.as_deref());
    let existing_priority = existing_state
        .as_ref()
        .map(|state| state.priority)
        .unwrap_or(MakoRunPriority::Normal);

    match event {
        LoopEvent::AgentSleeping {
            duration_secs,
            reason,
        } => {
            let wake_at = chrono::Utc::now() + chrono::Duration::seconds(*duration_secs as i64);
            store.set_state(
                session_id,
                MakoRuntimeStateStatus::Sleeping,
                Some(&wake_at.to_rfc3339()),
                Some(reason),
                None,
                Some(run_id),
                existing_wake_reason.or(Some("sleep")),
                existing_priority,
            )?;
        }
        LoopEvent::AwaitingInput { .. } => {
            store.set_state(
                session_id,
                MakoRuntimeStateStatus::AwaitingInput,
                None,
                None,
                None,
                Some(run_id),
                existing_wake_reason.or(Some("awaiting_input")),
                existing_priority,
            )?;
        }
        LoopEvent::Finished { stop_reason, .. } => {
            let status = match stop_reason {
                LoopStopReason::Sleeping => return Ok(()),
                LoopStopReason::AwaitingInput => MakoRuntimeStateStatus::AwaitingInput,
                LoopStopReason::ProviderError | LoopStopReason::PinchFailed => {
                    MakoRuntimeStateStatus::Error
                }
                _ => MakoRuntimeStateStatus::Idle,
            };
            store.set_state(
                session_id,
                status,
                None,
                None,
                None,
                None,
                existing_wake_reason,
                existing_priority,
            )?;
        }
        LoopEvent::Error { error } => {
            store.set_state(
                session_id,
                MakoRuntimeStateStatus::Error,
                None,
                None,
                Some(error),
                Some(run_id),
                existing_wake_reason.or(Some("error")),
                existing_priority,
            )?;
        }
        _ => {
            store.set_state(
                session_id,
                MakoRuntimeStateStatus::Running,
                None,
                None,
                None,
                Some(run_id),
                existing_wake_reason.or(Some("running")),
                existing_priority,
            )?;
        }
    }
    Ok(())
}

pub(super) fn parse_wake_at(next_wake_at: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(next_wake_at)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

pub(super) fn persist_runtime_state(
    db_path: &Path,
    session_id: &str,
    status: MakoRuntimeStateStatus,
    next_wake_at: Option<&str>,
    sleep_reason: Option<&str>,
    last_error: Option<&str>,
    current_run_id: Option<&str>,
    last_wake_reason: Option<&str>,
) -> Result<()> {
    let store = MakoRuntimeStateStore::new(Database::new(db_path)?);
    let priority = store
        .get_state(session_id)?
        .map(|state| state.priority)
        .unwrap_or(MakoRunPriority::Normal);
    store.set_state(
        session_id,
        status,
        next_wake_at,
        sleep_reason,
        last_error,
        current_run_id,
        last_wake_reason,
        priority,
    )
}
