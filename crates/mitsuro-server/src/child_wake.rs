//! Wake parent chat/code sessions when a background child agent completes.
//!
//! Mirrors process completion wake so the parent does not thrash-poll
//! `agent action=status` for a finished delegated_run_id.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{ensure, Context};
use mitsuro_core::agent::subagent::{AgentRuntimeManager, ChildCompletionEvent};
use mitsuro_core::agent::{DelegatedRunStage, LoopInput};
use mitsuro_core::storage::{Database, DelegatedRunRecord, DelegatedRunStore, SessionType};
use mitsuro_core::SessionManager;
use tokio::sync::mpsc;

use crate::routes::chat::{deliver_steering_with_rollover, resume_child_completion_session};
use crate::AppState;

const IDLE_RESUME_MAX_ATTEMPTS: usize = 3;
const IDLE_RESUME_RETRY_DELAY: Duration = Duration::from_millis(100);
const ABNORMAL_RECONCILE_MAX_ATTEMPTS: usize = 8;
const ABNORMAL_RECONCILE_RETRY_DELAY: Duration = Duration::from_millis(25);
const CHILD_WAKE_RECONCILE_INTERVAL: Duration = Duration::from_secs(15);

/// Wire the shared agent runtime manager to session wake handling.
pub async fn install_child_completion_wake(runtime: AgentRuntimeManager, state: AppState) {
    let (tx, mut rx) = mpsc::unbounded_channel::<ChildCompletionEvent>();
    runtime.set_completion_sender(tx.clone());
    let (reconcile_tx, mut reconcile_rx) = mpsc::unbounded_channel::<String>();
    runtime.set_completion_reconciliation_sender(reconcile_tx);

    let recovery_state = state.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_child_completion(&state, event).await {
                    tracing::warn!(%error, "Failed to deliver child agent completion wake");
                }
            });
        }
    });

    let reconciliation_state = recovery_state.clone();
    tokio::spawn(async move {
        while let Some(delegated_run_id) = reconcile_rx.recv().await {
            let state = reconciliation_state.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    reconcile_abnormal_child_completion(&state, &delegated_run_id).await
                {
                    tracing::warn!(
                        delegated_run_id,
                        %error,
                        "Failed to reconcile abnormal background Agent termination"
                    );
                }
            });
        }
    });

    let durable_recovery_state = recovery_state.clone();
    let durable_recovery_tx = tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHILD_WAKE_RECONCILE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Startup recovery below performs the initial scan.
        interval.tick().await;
        loop {
            interval.tick().await;
            // Re-scan both newly expired owners and every durable pending or
            // unqueued wake. If terminalization won but materialization or
            // live delivery failed transiently, the next tick retries without
            // requiring another server restart.
            match recover_pending_child_completions(&durable_recovery_state) {
                Ok(events) => {
                    for event in events {
                        if durable_recovery_tx.send(event).is_err() {
                            tracing::warn!(
                                "Child completion listener closed during durable wake recovery"
                            );
                            return;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "Failed to reconcile durable child Agent wakes");
                }
            }
        }
    });

    match recover_pending_child_completions(&recovery_state) {
        Ok(events) => {
            for event in events {
                if tx.send(event).is_err() {
                    tracing::warn!("Child completion listener closed during startup recovery");
                    break;
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "Failed to scan durable child completions during startup");
        }
    }
}

#[derive(Clone, Debug)]
struct ValidatedChildCompletion {
    event: ChildCompletionEvent,
    session_id: String,
    workspace_root: PathBuf,
}

fn recover_pending_child_completions(
    state: &AppState,
) -> anyhow::Result<Vec<ChildCompletionEvent>> {
    let delegated_store = DelegatedRunStore::new(Database::new(&state.db_path)?);
    let expired = delegated_store.expire_stale_background_host_leases()?;
    if !expired.is_empty() {
        tracing::warn!(
            count = expired.len(),
            "Recovered background Agent runs whose previous host lease expired"
        );
    }
    // First close the crash window where a background run persisted its
    // terminal artifact but the process died before pending steering was
    // queued. The receipt and pending row are committed atomically, so this is
    // safe to repeat on every startup.
    let unqueued = delegated_store.list_unqueued_parent_wakes()?;
    for delegated in unqueued {
        match materialize_durable_child_completion(state, &delegated.delegated_run_id) {
            Ok(
                DurableWakeMaterialization::Ready(_)
                | DurableWakeMaterialization::AlreadyPromoted
                | DurableWakeMaterialization::Suppressed,
            ) => {}
            Ok(DurableWakeMaterialization::NotTerminal) => {
                tracing::warn!(
                    delegated_run_id = %delegated.delegated_run_id,
                    "Startup wake scan returned a non-terminal delegated run"
                );
            }
            Err(error) => {
                tracing::warn!(
                    delegated_run_id = %delegated.delegated_run_id,
                    %error,
                    "Skipping unsafe unqueued child completion during startup recovery"
                );
            }
        }
    }

    let db = Database::new(&state.db_path)?;
    let mut stmt = db.conn().prepare(
        "SELECT session_id, role, content
           FROM messages
          WHERE role LIKE 'pending_user:child-wake-%'
          ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut pending = Vec::new();
    for row in rows {
        pending.push(row?);
    }
    drop(stmt);
    drop(db);

    let mut events = Vec::new();
    for (session_id, role, content_json) in pending {
        match recover_pending_child_completion(state, &session_id, &role, &content_json) {
            Ok(event) => events.push(event),
            Err(error) => {
                tracing::warn!(
                    session_id,
                    role,
                    %error,
                    "Skipping unsafe durable child completion during startup recovery"
                );
            }
        }
    }
    Ok(events)
}

#[derive(Debug)]
enum DurableWakeMaterialization {
    Ready(Box<ChildCompletionEvent>),
    NotTerminal,
    Suppressed,
    AlreadyPromoted,
}

fn existing_wake_is_publishable(delegated: &DelegatedRunRecord) -> bool {
    delegated.should_wake_parent()
        // Compatibility for pending child-wake rows written before migration
        // 53. The pending row itself proves the old background launch intent;
        // old explicit cancellations were never queued.
        || (!delegated.wake_parent
            && matches!(
                delegated.stage,
                DelegatedRunStage::Complete
                    | DelegatedRunStage::Degraded
                    | DelegatedRunStage::Failed
            ))
}

fn materialize_durable_child_completion(
    state: &AppState,
    delegated_run_id: &str,
) -> anyhow::Result<DurableWakeMaterialization> {
    let delegated = DelegatedRunStore::new(Database::new(&state.db_path)?)
        .get_run(delegated_run_id)?
        .with_context(|| format!("unknown delegated run '{delegated_run_id}'"))?;
    if !matches!(
        delegated.stage,
        DelegatedRunStage::Complete
            | DelegatedRunStage::Degraded
            | DelegatedRunStage::Failed
            | DelegatedRunStage::Cancelled
    ) {
        return Ok(DurableWakeMaterialization::NotTerminal);
    }
    if !delegated.should_wake_parent() {
        return Ok(DurableWakeMaterialization::Suppressed);
    }

    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    let session = session_manager
        .get_session(&delegated.parent_session_id)?
        .context("background child parent session no longer exists")?;
    ensure!(
        matches!(session.session_type, SessionType::Chat | SessionType::Code),
        "background child completion cannot wake a Hive-owned session"
    );
    let event = ChildCompletionEvent::from_durable_run(&delegated, session.user_id.clone())?;
    let event_workspace = event
        .workspace_root
        .as_deref()
        .context("durable child completion has no workspace")?;
    let session_workspace = session
        .project_dir
        .as_deref()
        .or(session.working_dir.as_deref())
        .context("background child parent session has no current project workspace")?;
    let session_workspace = PathBuf::from(session_workspace)
        .canonicalize()
        .context("canonicalizing background child parent workspace")?;
    ensure!(
        session_workspace == event_workspace,
        "background child parent session no longer matches its durable launch workspace"
    );

    let content_json = serde_json::to_string(&event.content)?;
    let queued = session_manager.queue_pending_steering_once(
        &delegated.parent_session_id,
        &event.pending_id,
        &content_json,
    )?;
    if !queued {
        let Some(existing) = session_manager
            .load_pending_steering(&delegated.parent_session_id, &event.pending_id)?
        else {
            return Ok(DurableWakeMaterialization::AlreadyPromoted);
        };
        ensure!(
            existing == content_json,
            "existing durable child completion differs from its authoritative terminal artifact"
        );
    }

    validate_child_completion(state, event.clone())?;
    Ok(DurableWakeMaterialization::Ready(Box::new(event)))
}

async fn reconcile_abnormal_child_completion(
    state: &AppState,
    delegated_run_id: &str,
) -> anyhow::Result<()> {
    let mut last_error = None;
    for attempt in 1..=ABNORMAL_RECONCILE_MAX_ATTEMPTS {
        match materialize_durable_child_completion(state, delegated_run_id) {
            Ok(DurableWakeMaterialization::Ready(event)) => {
                return handle_child_completion(state, *event).await;
            }
            Ok(
                DurableWakeMaterialization::Suppressed
                | DurableWakeMaterialization::AlreadyPromoted,
            ) => return Ok(()),
            Ok(DurableWakeMaterialization::NotTerminal) => {}
            Err(error) => last_error = Some(error),
        }

        if attempt < ABNORMAL_RECONCILE_MAX_ATTEMPTS {
            tokio::time::sleep(ABNORMAL_RECONCILE_RETRY_DELAY.saturating_mul(attempt as u32)).await;
        }
    }

    if let Some(error) = last_error {
        return Err(error.context("abnormal child wake reconciliation exhausted retries"));
    }
    anyhow::bail!(
        "delegated run '{delegated_run_id}' remained non-terminal after abnormal ownership ended"
    )
}

fn recover_pending_child_completion(
    state: &AppState,
    session_id: &str,
    role: &str,
    content_json: &str,
) -> anyhow::Result<ChildCompletionEvent> {
    let pending_id = role
        .strip_prefix("pending_user:")
        .context("recovered completion role is not pending steering")?;
    let delegated_run_id = pending_id
        .strip_prefix("child-wake-")
        .context("recovered completion is not a child wake")?;
    ensure!(
        !delegated_run_id.is_empty(),
        "recovered child wake has no run ID"
    );

    let delegated = DelegatedRunStore::new(Database::new(&state.db_path)?)
        .get_run(delegated_run_id)?
        .context("recovered child wake references an unknown delegated run")?;
    ensure!(
        delegated.parent_session_id == session_id,
        "recovered delegated run belongs to another parent session"
    );
    ensure!(
        existing_wake_is_publishable(&delegated),
        "recovered delegated run is not publishable"
    );
    let workspace_scopes = delegated
        .target_scope
        .iter()
        .filter(|scope| scope.kind == "workspace")
        .collect::<Vec<_>>();
    let [workspace_scope] = workspace_scopes.as_slice() else {
        anyhow::bail!("recovered delegated run has no unique launch workspace");
    };
    let workspace_root = PathBuf::from(&workspace_scope.path)
        .canonicalize()
        .context("canonicalizing recovered launch workspace")?;
    ensure!(
        workspace_root.is_dir(),
        "recovered launch workspace is not a directory"
    );

    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    let session = session_manager
        .get_session(session_id)?
        .context("recovered parent session no longer exists")?;
    ensure!(
        matches!(session.session_type, SessionType::Chat | SessionType::Code),
        "recovered child completion cannot wake a Hive-owned session"
    );
    let summary = delegated
        .human_review
        .clone()
        .context("recovered delegated run has no durable review summary")?;
    let terminal_stage = delegated.stage;
    let outcome = delegated
        .artifact
        .as_ref()
        .and_then(|artifact| artifact.get("outcome"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| terminal_stage_label(terminal_stage).to_string());
    let usable_agents = delegated
        .artifact
        .as_ref()
        .and_then(|artifact| artifact.get("usable_agents"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(usize::from(terminal_stage == DelegatedRunStage::Complete));
    let event = ChildCompletionEvent {
        session_id: Some(session_id.to_string()),
        user_id: session.user_id,
        workspace_root: Some(workspace_root),
        pending_id: pending_id.to_string(),
        content: serde_json::from_str(content_json)
            .context("decoding recovered child completion content")?,
        delegated_run_id: delegated_run_id.to_string(),
        task_name: delegated.child_name.unwrap_or_else(|| "child".to_string()),
        terminal_stage,
        outcome,
        usable_agents,
        success: terminal_stage == DelegatedRunStage::Complete,
        summary,
    };
    validate_child_completion(state, event.clone())?;
    Ok(event)
}

async fn handle_child_completion(
    state: &AppState,
    event: ChildCompletionEvent,
) -> anyhow::Result<()> {
    if event.session_id.is_none() {
        tracing::debug!(
            delegated_run_id = %event.delegated_run_id,
            "Child agent completed without bound session; no wake"
        );
        return Ok(());
    }
    let completion = validate_child_completion(state, event)?;
    let session_id = completion.session_id.as_str();
    let sender = state.session_inputs.read().await.get(session_id).cloned();
    if let Some(sender) = sender {
        let input = LoopInput::Steer {
            pending_id: Some(completion.event.pending_id.clone()),
            content: completion.event.content.clone(),
        };
        let delivered = deliver_steering_with_rollover(state, session_id, sender, input).await;
        if delivered {
            tracing::info!(
                session_id,
                delegated_run_id = %completion.event.delegated_run_id,
                name = %completion.event.task_name,
                pending_id = %completion.event.pending_id,
                "Delivered durable child completion to active session"
            );

            // Acceptance by an input channel is not proof that the finishing
            // run promoted the durable row. Re-check after its canonical lock
            // is released and resume only if this exact pending ID remains.
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(error) = ensure_completion_resumed(&state, completion).await {
                    tracing::warn!(%error, "Failed child completion post-run recovery");
                }
            });
            return Ok(());
        }
    }

    ensure_completion_resumed(state, completion).await?;
    Ok(())
}

fn validate_child_completion(
    state: &AppState,
    event: ChildCompletionEvent,
) -> anyhow::Result<ValidatedChildCompletion> {
    let session_id = event
        .session_id
        .clone()
        .context("child completion has no parent session")?;
    ensure!(
        event.pending_id == format!("child-wake-{}", event.delegated_run_id),
        "child completion pending ID does not match its delegated run"
    );

    let delegated = DelegatedRunStore::new(Database::new(&state.db_path)?)
        .get_run(&event.delegated_run_id)?
        .context("child completion references an unknown delegated run")?;
    ensure!(
        delegated.parent_session_id == session_id,
        "child completion delegated run belongs to a different parent session"
    );
    ensure!(
        existing_wake_is_publishable(&delegated),
        "child completion delegated run is not publishable"
    );
    ensure!(
        event.success == (delegated.stage == DelegatedRunStage::Complete),
        "child completion outcome does not match its durable terminal stage"
    );
    ensure!(
        event.terminal_stage == delegated.stage,
        "child completion terminal stage does not match its durable run"
    );
    let durable_outcome = delegated
        .artifact
        .as_ref()
        .and_then(|artifact| artifact.get("outcome"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| terminal_stage_label(delegated.stage));
    ensure!(
        event.outcome == durable_outcome,
        "child completion outcome label does not match its durable artifact"
    );
    let durable_usable_agents = delegated
        .artifact
        .as_ref()
        .and_then(|artifact| artifact.get("usable_agents"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(usize::from(delegated.stage == DelegatedRunStage::Complete));
    ensure!(
        event.usable_agents == durable_usable_agents,
        "child completion usable-agent count does not match its durable artifact"
    );
    ensure!(
        delegated.human_review.as_deref() == Some(event.summary.as_str()),
        "child completion summary does not match its durable result"
    );
    ensure!(
        delegated.completed_at.is_some(),
        "child completion delegated run has no durable completion timestamp"
    );
    ensure!(
        delegated.artifact.is_some(),
        "child completion delegated run has no durable artifact"
    );

    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    let session = session_manager
        .get_session(&session_id)?
        .context("child completion parent session no longer exists")?;
    ensure!(
        session.user_id == event.user_id,
        "child completion owner does not match its parent session"
    );
    ensure!(
        matches!(session.session_type, SessionType::Chat | SessionType::Code),
        "child completion cannot wake a Hive-owned session"
    );
    let session_workspace = session
        .project_dir
        .as_deref()
        .or(session.working_dir.as_deref())
        .context("child completion parent session has no current project workspace")?;

    let durable_content = session_manager
        .load_pending_steering(&session_id, &event.pending_id)?
        .context("child completion has no durable pending steering row")?;
    ensure!(
        durable_content == serde_json::to_string(&event.content)?,
        "child completion live content does not match its durable row"
    );

    let workspace_root = event
        .workspace_root
        .as_deref()
        .context("child completion has no captured workspace authority")?
        .canonicalize()
        .context("canonicalizing child completion workspace authority")?;
    ensure!(
        workspace_root.is_dir(),
        "child completion workspace authority is not a directory"
    );
    let workspace_scopes = delegated
        .target_scope
        .iter()
        .filter(|scope| scope.kind == "workspace")
        .collect::<Vec<_>>();
    let [workspace_scope] = workspace_scopes.as_slice() else {
        anyhow::bail!("child completion delegated run has no unique launch workspace");
    };
    let durable_workspace_root = PathBuf::from(&workspace_scope.path)
        .canonicalize()
        .context("canonicalizing delegated launch workspace")?;
    ensure!(
        durable_workspace_root.starts_with(&workspace_root),
        "child completion durable launch workspace escapes its captured authority"
    );
    let current_session_workspace = PathBuf::from(session_workspace)
        .canonicalize()
        .context("canonicalizing parent session project workspace")?;
    ensure!(
        current_session_workspace == durable_workspace_root,
        "child completion parent session project no longer matches its durable launch workspace"
    );

    Ok(ValidatedChildCompletion {
        event,
        session_id,
        workspace_root: durable_workspace_root,
    })
}

fn terminal_stage_label(stage: DelegatedRunStage) -> &'static str {
    match stage {
        DelegatedRunStage::Created => "created",
        DelegatedRunStage::Running => "running",
        DelegatedRunStage::Synthesizing => "synthesizing",
        DelegatedRunStage::Complete => "complete",
        DelegatedRunStage::Degraded => "degraded",
        DelegatedRunStage::Failed => "failed",
        DelegatedRunStage::Cancelled => "cancelled",
    }
}

async fn ensure_completion_resumed(
    state: &AppState,
    completion: ValidatedChildCompletion,
) -> anyhow::Result<bool> {
    let pending_id = completion.event.pending_id.clone();
    ensure_completion_resumed_with(
        state,
        completion,
        move |state, session_id, user_id, workspace_root, guard| {
            let pending_id = pending_id.clone();
            async move {
                let promoted = SessionManager::new(Database::new(&state.db_path)?)
                    .promote_pending_steering(&session_id, &pending_id)?;
                if promoted.is_none() {
                    return Ok(());
                }

                resume_child_completion_session(&state, &session_id, user_id, workspace_root, guard)
                    .await
            }
        },
    )
    .await
}

async fn ensure_completion_resumed_with<R, F>(
    state: &AppState,
    completion: ValidatedChildCompletion,
    resume: R,
) -> anyhow::Result<bool>
where
    R: FnMut(AppState, String, Option<String>, PathBuf, tokio::sync::OwnedMutexGuard<()>) -> F,
    F: std::future::Future<Output = Result<(), crate::error::AppError>>,
{
    ensure_completion_resumed_with_policy(
        state,
        completion,
        IDLE_RESUME_MAX_ATTEMPTS,
        IDLE_RESUME_RETRY_DELAY,
        resume,
    )
    .await
}

async fn ensure_completion_resumed_with_policy<R, F>(
    state: &AppState,
    completion: ValidatedChildCompletion,
    max_attempts: usize,
    retry_delay: Duration,
    mut resume: R,
) -> anyhow::Result<bool>
where
    R: FnMut(AppState, String, Option<String>, PathBuf, tokio::sync::OwnedMutexGuard<()>) -> F,
    F: std::future::Future<Output = Result<(), crate::error::AppError>>,
{
    let max_attempts = max_attempts.max(1);
    for attempt in 1..=max_attempts {
        // Every attempt reacquires the canonical session lock. The resume
        // future owns this guard, so a failed attempt releases it before the
        // bounded delay and next durable pending-row check.
        let guard = state.lock_session(&completion.session_id).await;
        let session_manager = SessionManager::new(Database::new(&state.db_path)?);
        if !session_manager
            .has_pending_steering(&completion.session_id, &completion.event.pending_id)?
        {
            tracing::debug!(
                session_id = %completion.session_id,
                delegated_run_id = %completion.event.delegated_run_id,
                pending_id = %completion.event.pending_id,
                "Child completion was already promoted by an active or replacement run"
            );
            return Ok(false);
        }

        match resume(
            state.clone(),
            completion.session_id.clone(),
            completion.event.user_id.clone(),
            completion.workspace_root.clone(),
            guard,
        )
        .await
        {
            Ok(()) => {
                tracing::info!(
                    session_id = %completion.session_id,
                    delegated_run_id = %completion.event.delegated_run_id,
                    pending_id = %completion.event.pending_id,
                    attempt,
                    "Started detached parent continuation for child completion"
                );
                return Ok(true);
            }
            Err(error) if resume_error_is_transient(&error) && attempt < max_attempts => {
                tracing::warn!(
                    session_id = %completion.session_id,
                    delegated_run_id = %completion.event.delegated_run_id,
                    pending_id = %completion.event.pending_id,
                    attempt,
                    max_attempts,
                    error = ?error,
                    "Detached child completion resume failed transiently; retrying"
                );
                tokio::time::sleep(retry_delay.saturating_mul(attempt as u32)).await;
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "child completion resume failed on attempt {attempt}/{max_attempts}: {error:?}"
                ));
            }
        }
    }

    unreachable!("at least one child completion resume attempt is required")
}

fn resume_error_is_transient(error: &crate::error::AppError) -> bool {
    matches!(
        error,
        crate::error::AppError::Conflict(_)
            | crate::error::AppError::ServiceUnavailable(_)
            | crate::error::AppError::BadGateway(_)
            | crate::error::AppError::Internal(_)
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use mitsuro_core::agent::{AgentCancellation, DelegatedRunStage};
    use mitsuro_core::ai::models::create_model_registry;
    use mitsuro_core::ai::types::Content;
    use mitsuro_core::mcp::McpManager;
    use mitsuro_core::process::ProcessRegistry;
    use mitsuro_core::skills::SkillsManager;
    use mitsuro_core::storage::credentials::CredentialStore;
    use mitsuro_core::storage::{
        DelegatedRunRole, DelegatedRunScope, DelegatedRunStartInput, WorkspaceMode,
    };
    use mitsuro_core::tools::registry::ToolRegistry;
    use tokio::sync::{mpsc, Mutex, RwLock};

    use super::*;

    fn test_state() -> (AppState, tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("krusty.db");
        Database::new(&db_path).expect("database should initialize");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace should exist");
        let state = AppState {
            server_port: 3000,
            db_path: Arc::new(db_path),
            working_dir: Arc::new(workspace.clone()),
            ai_client: None,
            tool_registry: Arc::new(ToolRegistry::new()),
            process_registry: Arc::new(ProcessRegistry::new()),
            model_registry: create_model_registry(),
            credential_store: Arc::new(RwLock::new(CredentialStore::default())),
            mcp_manager: Arc::new(McpManager::new(workspace.clone())),
            hook_manager: Arc::new(RwLock::new(mitsuro_core::agent::UserHookManager::new())),
            skills_manager: Arc::new(RwLock::new(SkillsManager::with_defaults(&workspace))),
            cancellation: AgentCancellation::new(),
            session_locks: Arc::new(RwLock::new(HashMap::new())),
            session_inputs: Arc::new(RwLock::new(HashMap::new())),
            session_presence: Arc::new(RwLock::new(HashMap::new())),
            delegated_state: Arc::new(RwLock::new(HashMap::new())),
            remote_access: Arc::new(RwLock::new(crate::remote_access::RemoteAccessConfig {
                enabled: true,
                token: String::new(),
            })),
            active_agent_streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            peak_rss_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            peak_virtual_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            push_service: None,
            apns_service: None,
            oauth_flows: Arc::new(Mutex::new(HashMap::new())),
            hive_runtime: crate::hive_runtime::HiveRuntimeManager::new(),
        };
        (state, temp, workspace)
    }

    fn seed_completion(
        state: &AppState,
        workspace: &std::path::Path,
    ) -> (ChildCompletionEvent, String) {
        let db = Database::new(&state.db_path).expect("database should open");
        db.conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES ('alice', 'a@test', 'free')",
                [],
            )
            .expect("user should insert");
        let manager = SessionManager::new(db);
        let session_id = manager
            .create_session_for_user_with_config(
                "Parent",
                None,
                Some(workspace.to_string_lossy().as_ref()),
                Some(workspace.to_string_lossy().as_ref()),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Code,
            )
            .expect("session should create");
        let delegated_run_id = "child-run-1".to_string();
        let store = DelegatedRunStore::new(
            Database::new(&state.db_path).expect("delegated database should open"),
        );
        store
            .create_background_run_with_child_contract(
                &DelegatedRunStartInput {
                    delegated_run_id: delegated_run_id.clone(),
                    parent_session_id: session_id.clone(),
                    parent_tool_call_id: Some("tool-1".into()),
                    role: DelegatedRunRole::Explore,
                    stage: DelegatedRunStage::Running,
                    provider: None,
                    model: None,
                    resumable: true,
                    resumed_from_run_id: None,
                    target_scope: vec![
                        DelegatedRunScope {
                            label: "launch workspace".into(),
                            path: workspace
                                .canonicalize()
                                .expect("canonical workspace")
                                .to_string_lossy()
                                .into_owned(),
                            kind: "workspace".into(),
                        },
                        DelegatedRunScope {
                            label: "project".into(),
                            path: ".".into(),
                            kind: "project".into(),
                        },
                    ],
                },
                Some("research"),
                &Default::default(),
            )
            .expect("delegated run should create");
        store
            .finalize_run(
                &delegated_run_id,
                DelegatedRunStage::Complete,
                &serde_json::json!({"result": "done"}),
                Some("done"),
                true,
            )
            .expect("delegated run should finalize");

        let pending_id = format!("child-wake-{delegated_run_id}");
        let content = vec![Content::Text {
            text: "[CHILD AGENT COMPLETE]\nsummary:\ndone".into(),
        }];
        let content_json = serde_json::to_string(&content).expect("content should serialize");
        assert!(SessionManager::new(
            Database::new(&state.db_path).expect("queue database should open")
        )
        .queue_pending_steering_once(&session_id, &pending_id, &content_json)
        .expect("completion should queue"));

        (
            ChildCompletionEvent {
                session_id: Some(session_id.clone()),
                user_id: Some("alice".into()),
                workspace_root: Some(workspace.to_path_buf()),
                pending_id,
                content,
                delegated_run_id,
                task_name: "research".into(),
                terminal_stage: DelegatedRunStage::Complete,
                outcome: "complete".into(),
                usable_agents: 1,
                success: true,
                summary: "done".into(),
            },
            session_id,
        )
    }

    #[tokio::test]
    async fn active_completion_delivers_the_exact_durable_id() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let guard = state.lock_session(&session_id).await;
        let (input_tx, mut input_rx) = mpsc::unbounded_channel();
        state
            .session_inputs
            .write()
            .await
            .insert(session_id.clone(), input_tx);

        handle_child_completion(&state, event.clone())
            .await
            .expect("active completion should deliver");
        let delivered = input_rx.recv().await.expect("completion should arrive");
        let LoopInput::Steer {
            pending_id: Some(delivered_id),
            content: delivered_content,
        } = delivered
        else {
            panic!("completion should retain its exact durable steering identity");
        };
        assert_eq!(delivered_id, event.pending_id);
        assert_eq!(
            serde_json::to_string(&delivered_content).expect("serialize delivered completion"),
            serde_json::to_string(&event.content).expect("serialize expected completion")
        );

        SessionManager::new(Database::new(&state.db_path).expect("database should open"))
            .promote_pending_steering(&session_id, &event.pending_id)
            .expect("completion should promote");
        drop(guard);
    }

    #[tokio::test]
    async fn startup_recovery_reconstructs_a_safe_pending_child_completion() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);

        let recovered =
            recover_pending_child_completions(&state).expect("startup recovery should scan");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].pending_id, event.pending_id);
        assert_eq!(
            recovered[0].session_id.as_deref(),
            Some(session_id.as_str())
        );
        assert_eq!(
            recovered[0].workspace_root.as_deref(),
            Some(
                workspace
                    .canonicalize()
                    .expect("canonical workspace")
                    .as_path()
            )
        );
        validate_child_completion(&state, recovered[0].clone())
            .expect("recovered event should pass the live validator");
    }

    #[tokio::test]
    async fn startup_recovery_materializes_terminal_artifact_crash_window_once() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let manager = SessionManager::new(
            Database::new(&state.db_path).expect("recovery database should open"),
        );
        let pending_role = format!("pending_user:{}", event.pending_id);
        manager
            .db()
            .conn()
            .execute(
                "DELETE FROM messages WHERE session_id = ?1 AND role = ?2",
                [&session_id, &pending_role],
            )
            .expect("remove preexisting pending fixture");
        manager
            .db()
            .conn()
            .execute(
                "DELETE FROM steering_idempotency WHERE session_id = ?1 AND pending_id = ?2",
                [&session_id, &event.pending_id],
            )
            .expect("remove preexisting receipt fixture");

        let recovered =
            recover_pending_child_completions(&state).expect("crash window should reconcile");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].delegated_run_id, event.delegated_run_id);
        assert!(manager
            .has_pending_steering(&session_id, &event.pending_id)
            .expect("pending completion should load"));

        manager
            .promote_pending_steering(&session_id, &event.pending_id)
            .expect("promote recovered completion");
        assert!(recover_pending_child_completions(&state)
            .expect("second recovery should scan")
            .is_empty());
    }

    #[tokio::test]
    async fn durable_reconciliation_retries_terminal_wake_after_materialization_failure() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let db = Database::new(&state.db_path).expect("reconciliation database should open");
        let pending_role = format!("pending_user:{}", event.pending_id);
        db.conn()
            .execute(
                "DELETE FROM messages WHERE session_id = ?1 AND role = ?2",
                [&session_id, &pending_role],
            )
            .expect("remove pending fixture");
        db.conn()
            .execute(
                "DELETE FROM steering_idempotency WHERE session_id = ?1 AND pending_id = ?2",
                [&session_id, &event.pending_id],
            )
            .expect("remove receipt fixture");
        let original_scope: String = db
            .conn()
            .query_row(
                "SELECT target_scope_json FROM delegated_runs WHERE delegated_run_id = ?1",
                [&event.delegated_run_id],
                |row| row.get(0),
            )
            .expect("load original durable scope");
        db.conn()
            .execute(
                "UPDATE delegated_runs SET target_scope_json = '[]' WHERE delegated_run_id = ?1",
                [&event.delegated_run_id],
            )
            .expect("make first materialization fail");
        drop(db);

        assert!(recover_pending_child_completions(&state)
            .expect("failed materialization should not fail the whole scan")
            .is_empty());
        let manager = SessionManager::new(
            Database::new(&state.db_path).expect("receipt database should open"),
        );
        assert!(!manager
            .has_pending_steering(&session_id, &event.pending_id)
            .expect("failed materialization must not write a pending wake"));
        manager
            .db()
            .conn()
            .execute(
                "UPDATE delegated_runs SET target_scope_json = ?2 WHERE delegated_run_id = ?1",
                [&event.delegated_run_id, &original_scope],
            )
            .expect("restore durable scope after transient failure");

        let recovered = recover_pending_child_completions(&state)
            .expect("next periodic-style scan should retry the terminal row");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].delegated_run_id, event.delegated_run_id);
    }

    #[tokio::test]
    async fn startup_recovery_expires_a_dead_background_host_and_wakes_with_uncertainty() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let db = Database::new(&state.db_path).expect("host lease database should open");
        let pending_role = format!("pending_user:{}", event.pending_id);
        db.conn()
            .execute(
                "DELETE FROM messages WHERE session_id = ?1 AND role = ?2",
                [&session_id, &pending_role],
            )
            .expect("remove preexisting pending fixture");
        db.conn()
            .execute(
                "DELETE FROM steering_idempotency WHERE session_id = ?1 AND pending_id = ?2",
                [&session_id, &event.pending_id],
            )
            .expect("remove preexisting receipt fixture");
        db.conn()
            .execute(
                "UPDATE delegated_runs
                    SET stage = 'running',
                        artifact_json = NULL,
                        human_review = NULL,
                        completed_at = NULL,
                        host_lease_expires_at_ms = 0
                  WHERE delegated_run_id = ?1",
                [&event.delegated_run_id],
            )
            .expect("simulate a previous server dying before terminal persistence");
        drop(db);

        let recovered = recover_pending_child_completions(&state)
            .expect("startup should recover the expired host lease");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].delegated_run_id, event.delegated_run_id);
        assert_eq!(recovered[0].terminal_stage, DelegatedRunStage::Cancelled);
        assert!(!recovered[0].success);
        assert_eq!(recovered[0].outcome, "cancelled");

        let durable = DelegatedRunStore::new(
            Database::new(&state.db_path).expect("recovered database should open"),
        )
        .get_run(&event.delegated_run_id)
        .expect("recovered run should load")
        .expect("recovered run should exist");
        assert_eq!(durable.stage, DelegatedRunStage::Cancelled);
        assert_eq!(
            durable.artifact.as_ref().unwrap()["outcome_reason"],
            "background_host_lease_expired"
        );
    }

    #[tokio::test]
    async fn abnormal_cancel_materializes_but_explicit_cancel_stays_quiet() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let db = Database::new(&state.db_path).expect("cancellation database should open");
        let pending_role = format!("pending_user:{}", event.pending_id);
        db.conn()
            .execute(
                "DELETE FROM messages WHERE session_id = ?1 AND role = ?2",
                [&session_id, &pending_role],
            )
            .expect("remove pending fixture");
        db.conn()
            .execute(
                "DELETE FROM steering_idempotency WHERE session_id = ?1 AND pending_id = ?2",
                [&session_id, &event.pending_id],
            )
            .expect("remove receipt fixture");
        let abnormal_artifact = serde_json::json!({
            "outcome": "cancelled",
            "outcome_reason": "caller_aborted_before_terminal",
            "side_effects_may_have_occurred": true,
            "quiescent": false,
        })
        .to_string();
        db.conn()
            .execute(
                "UPDATE delegated_runs
                    SET stage = 'cancelled',
                        artifact_json = ?2,
                        human_review = 'caller disappeared',
                        completed_at = updated_at
                  WHERE delegated_run_id = ?1",
                [&event.delegated_run_id, &abnormal_artifact],
            )
            .expect("seed abnormal cancellation");
        drop(db);

        let materialized = materialize_durable_child_completion(&state, &event.delegated_run_id)
            .expect("abnormal cancellation should reconcile");
        let DurableWakeMaterialization::Ready(abnormal) = materialized else {
            panic!("abnormal cancellation must become a durable parent wake");
        };
        assert_eq!(abnormal.terminal_stage, DelegatedRunStage::Cancelled);
        assert_eq!(abnormal.outcome, "cancelled");
        assert!(!abnormal.success);

        let db = Database::new(&state.db_path).expect("explicit cancellation database");
        db.conn()
            .execute(
                "DELETE FROM messages WHERE session_id = ?1 AND role = ?2",
                [&session_id, &pending_role],
            )
            .expect("remove abnormal pending fixture");
        db.conn()
            .execute(
                "DELETE FROM steering_idempotency WHERE session_id = ?1 AND pending_id = ?2",
                [&session_id, &event.pending_id],
            )
            .expect("remove abnormal receipt fixture");
        let explicit_artifact = serde_json::json!({
            "outcome": "cancelled",
            "outcome_reason": "cancelled",
        })
        .to_string();
        db.conn()
            .execute(
                "UPDATE delegated_runs SET artifact_json = ?2, human_review = 'cancelled by user'
                  WHERE delegated_run_id = ?1",
                [&event.delegated_run_id, &explicit_artifact],
            )
            .expect("seed explicit cancellation");
        drop(db);

        assert!(matches!(
            materialize_durable_child_completion(&state, &event.delegated_run_id)
                .expect("explicit cancellation should classify"),
            DurableWakeMaterialization::Suppressed
        ));
        assert!(!SessionManager::new(
            Database::new(&state.db_path).expect("pending verification database")
        )
        .has_pending_steering(&session_id, &event.pending_id)
        .expect("pending state should load"));
    }

    #[tokio::test]
    async fn idle_completion_resumes_once_and_duplicate_event_is_a_noop() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let completion = validate_child_completion(&state, event.clone())
            .expect("completion authority should validate");
        let (resume_tx, mut resume_rx) = mpsc::unbounded_channel();

        assert!(ensure_completion_resumed_with(
            &state,
            completion.clone(),
            move |_state, resumed_session, owner, root, _guard| {
                let resume_tx = resume_tx.clone();
                async move {
                    resume_tx
                        .send((resumed_session, owner, root))
                        .expect("resume should be observed");
                    Ok(())
                }
            },
        )
        .await
        .expect("idle completion should dispatch resume"));
        let (resumed_session, owner, root) = resume_rx.recv().await.expect("resume marker");
        assert_eq!(resumed_session, session_id);
        assert_eq!(owner.as_deref(), Some("alice"));
        assert_eq!(root, workspace.canonicalize().expect("canonical workspace"));

        SessionManager::new(Database::new(&state.db_path).expect("database should open"))
            .promote_pending_steering(&session_id, &event.pending_id)
            .expect("completion should promote");
        assert!(!ensure_completion_resumed_with(
            &state,
            completion,
            |_state, _session, _owner, _root, _guard| async move {
                panic!("duplicate completion must not start another parent run")
            },
        )
        .await
        .expect("duplicate completion should be harmless"));
    }

    #[tokio::test]
    async fn idle_completion_retries_transient_resume_failures_with_a_fresh_lock() {
        let (state, _temp, workspace) = test_state();
        let (event, _session_id) = seed_completion(&state, &workspace);
        let completion =
            validate_child_completion(&state, event).expect("completion authority should validate");
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed_attempts = Arc::clone(&attempts);

        assert!(ensure_completion_resumed_with_policy(
            &state,
            completion,
            3,
            Duration::ZERO,
            move |_state, _session, _owner, _root, _guard| {
                let attempt = observed_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    if attempt < 3 {
                        Err(crate::error::AppError::ServiceUnavailable(
                            "temporary startup failure".to_string(),
                        ))
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .await
        .expect("third attempt should start"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_does_not_start_twice_after_pending_completion_was_claimed() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let pending_id = event.pending_id.clone();
        let completion =
            validate_child_completion(&state, event).expect("completion authority should validate");
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed_attempts = Arc::clone(&attempts);

        assert!(!ensure_completion_resumed_with_policy(
            &state,
            completion,
            3,
            Duration::ZERO,
            move |state, resumed_session, _owner, _root, _guard| {
                let attempt = observed_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                let pending_id = pending_id.clone();
                async move {
                    assert_eq!(attempt, 1, "resume closure must not run twice");
                    SessionManager::new(
                        Database::new(&state.db_path).expect("retry database should open"),
                    )
                    .promote_pending_steering(&resumed_session, &pending_id)
                    .expect("partial starter should claim pending completion");
                    Err(crate::error::AppError::ServiceUnavailable(
                        "starter response was lost".to_string(),
                    ))
                }
            },
        )
        .await
        .expect("claimed completion should make retry a no-op"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(
            !SessionManager::new(Database::new(&state.db_path).expect("database should open"))
                .has_pending_steering(&session_id, "child-wake-child-run-1")
                .expect("pending state should load")
        );
    }

    #[tokio::test]
    async fn idle_completion_stops_after_the_bounded_attempt_count() {
        let (state, _temp, workspace) = test_state();
        let (event, _session_id) = seed_completion(&state, &workspace);
        let completion =
            validate_child_completion(&state, event).expect("completion authority should validate");
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed_attempts = Arc::clone(&attempts);

        let error = ensure_completion_resumed_with_policy(
            &state,
            completion,
            3,
            Duration::ZERO,
            move |_state, _session, _owner, _root, _guard| {
                observed_attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    Err(crate::error::AppError::ServiceUnavailable(
                        "still unavailable".to_string(),
                    ))
                }
            },
        )
        .await
        .expect_err("resume must stop after its bounded attempts");
        assert!(error.to_string().contains("attempt 3/3"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn completion_authority_rejects_foreign_session_owner() {
        let (state, _temp, workspace) = test_state();
        let (mut event, _session_id) = seed_completion(&state, &workspace);
        event.user_id = Some("bob".into());

        let error = validate_child_completion(&state, event)
            .expect_err("foreign completion owner must be rejected");
        assert!(error.to_string().contains("owner does not match"));
    }

    #[tokio::test]
    async fn completion_authority_rejects_stale_outcome_metadata() {
        let (state, _temp, workspace) = test_state();
        let (mut event, _session_id) = seed_completion(&state, &workspace);
        event.success = false;

        let error = validate_child_completion(&state, event)
            .expect_err("stale completion outcome must be rejected");
        assert!(error
            .to_string()
            .contains("outcome does not match its durable terminal stage"));
    }

    #[tokio::test]
    async fn completion_authority_rejects_workspace_not_in_durable_lineage() {
        let (state, _temp, workspace) = test_state();
        let (mut event, _session_id) = seed_completion(&state, &workspace);
        let foreign_workspace = workspace
            .parent()
            .expect("workspace parent")
            .join("foreign-workspace");
        std::fs::create_dir_all(&foreign_workspace).expect("foreign workspace");
        event.workspace_root = Some(foreign_workspace);

        let error = validate_child_completion(&state, event)
            .expect_err("foreign workspace authority must be rejected");
        assert!(error.to_string().contains("escapes its captured authority"));
    }

    #[tokio::test]
    async fn changed_session_project_cannot_canonicalize_a_pending_child_wake() {
        let (state, _temp, workspace) = test_state();
        let (event, session_id) = seed_completion(&state, &workspace);
        let changed_project = workspace
            .parent()
            .expect("workspace parent")
            .join("changed-project");
        std::fs::create_dir_all(&changed_project).expect("changed project");
        Database::new(&state.db_path)
            .expect("database should open")
            .conn()
            .execute(
                "UPDATE sessions SET project_dir = ?1 WHERE id = ?2",
                [
                    changed_project.to_string_lossy().as_ref(),
                    session_id.as_str(),
                ],
            )
            .expect("session project should change");

        let error = validate_child_completion(&state, event.clone())
            .expect_err("changed session project must block automatic continuation");
        assert!(error
            .to_string()
            .contains("project no longer matches its durable launch workspace"));
        let manager = SessionManager::new(
            Database::new(&state.db_path).expect("promotion database should open"),
        );
        let promotion_error = manager
            .promote_pending_steering(&session_id, &event.pending_id)
            .expect_err("model-boundary promotion must recheck workspace authority");
        assert!(promotion_error
            .to_string()
            .contains("project no longer matches its durable launch workspace"));
        assert_eq!(
            manager
                .promote_orphaned_pending_steering(&session_id)
                .expect("ordinary chat recovery should ignore child wakes"),
            0,
            "ordinary chat recovery must not bypass child-wake authority"
        );
        assert!(manager
            .has_pending_steering(&session_id, &event.pending_id)
            .expect("pending completion should remain for user review"));
        assert!(
            manager
                .load_session_messages(&session_id)
                .expect("canonical history should load")
                .iter()
                .all(|(_, content)| !content.contains("[CHILD AGENT COMPLETE]")),
            "the rejected child wake must never enter canonical user history"
        );
    }

    #[tokio::test]
    async fn completion_authority_rejects_cancelled_terminal_winner() {
        let (state, _temp, workspace) = test_state();
        let (mut event, _session_id) = seed_completion(&state, &workspace);
        Database::new(&state.db_path)
            .expect("database should open")
            .conn()
            .execute(
                "UPDATE delegated_runs SET stage = 'cancelled' WHERE delegated_run_id = ?1",
                [&event.delegated_run_id],
            )
            .expect("test should install cancelled terminal winner");
        event.success = false;

        let error = validate_child_completion(&state, event)
            .expect_err("cancelled completion must never wake the parent");
        assert!(error.to_string().contains("not publishable"));
    }
}
