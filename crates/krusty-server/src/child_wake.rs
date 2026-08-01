//! Wake parent chat/code sessions when a background child agent completes.
//!
//! Mirrors process completion wake so the parent does not thrash-poll
//! `agent action=status` for a finished delegated_run_id.

use std::path::PathBuf;

use anyhow::{ensure, Context};
use krusty_core::agent::subagent::{AgentRuntimeManager, ChildCompletionEvent};
use krusty_core::agent::{DelegatedRunStage, LoopInput};
use krusty_core::storage::{Database, DelegatedRunStore, SessionType};
use krusty_core::SessionManager;
use tokio::sync::mpsc;

use crate::routes::chat::{deliver_steering_with_rollover, resume_child_completion_session};
use crate::AppState;

/// Wire the shared agent runtime manager to session wake handling.
pub async fn install_child_completion_wake(runtime: AgentRuntimeManager, state: AppState) {
    let (tx, mut rx) = mpsc::unbounded_channel::<ChildCompletionEvent>();
    runtime.set_completion_sender(tx);

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
}

#[derive(Clone)]
struct ValidatedChildCompletion {
    event: ChildCompletionEvent,
    session_id: String,
    workspace_root: PathBuf,
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
        matches!(
            delegated.stage,
            DelegatedRunStage::Complete
                | DelegatedRunStage::Degraded
                | DelegatedRunStage::Failed
                | DelegatedRunStage::Cancelled
        ),
        "child completion delegated run is not terminal"
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

    Ok(ValidatedChildCompletion {
        event,
        session_id,
        workspace_root,
    })
}

async fn ensure_completion_resumed(
    state: &AppState,
    completion: ValidatedChildCompletion,
) -> anyhow::Result<bool> {
    ensure_completion_resumed_with(
        state,
        completion,
        |state, session_id, user_id, workspace_root, guard| async move {
            resume_child_completion_session(
                &state,
                &session_id,
                user_id,
                workspace_root,
                guard,
            )
            .await
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
    R: FnOnce(
        AppState,
        String,
        Option<String>,
        PathBuf,
        tokio::sync::OwnedMutexGuard<()>,
    ) -> F,
    F: std::future::Future<Output = Result<(), crate::error::AppError>>,
{
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

    resume(
        state.clone(),
        completion.session_id.clone(),
        completion.event.user_id.clone(),
        completion.workspace_root,
        guard,
    )
    .await
    .map_err(|error| anyhow::anyhow!("child completion resume failed: {error:?}"))?;
    tracing::info!(
        session_id = %completion.session_id,
        delegated_run_id = %completion.event.delegated_run_id,
        pending_id = %completion.event.pending_id,
        "Started detached parent continuation for child completion"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use krusty_core::agent::{AgentCancellation, DelegatedRunStage};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::ai::types::Content;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::{
        DelegatedRunRole, DelegatedRunScope, DelegatedRunStartInput, WorkspaceMode,
    };
    use krusty_core::tools::registry::ToolRegistry;
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
            hook_manager: Arc::new(RwLock::new(
                krusty_core::agent::UserHookManager::new(),
            )),
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
            mako_runtime: crate::mako_runtime::MakoRuntimeManager::new(),
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
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: delegated_run_id.clone(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: Some("tool-1".into()),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Running,
                provider: None,
                model: None,
                resumable: true,
                resumed_from_run_id: None,
                target_scope: vec![DelegatedRunScope {
                    label: "workspace".into(),
                    path: workspace.to_string_lossy().into_owned(),
                    kind: "directory".into(),
                }],
            })
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
        assert!(matches!(
            input_rx.recv().await,
            Some(LoopInput::Steer { pending_id: Some(id), content })
                if id == event.pending_id && content == event.content
        ));

        SessionManager::new(Database::new(&state.db_path).expect("database should open"))
            .promote_pending_steering(&session_id, &event.pending_id)
            .expect("completion should promote");
        drop(guard);
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
            move |_state, resumed_session, owner, root, _guard| async move {
                resume_tx
                    .send((resumed_session, owner, root))
                    .expect("resume should be observed");
                Ok(())
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

    #[test]
    fn completion_authority_rejects_foreign_session_owner() {
        let (state, _temp, workspace) = test_state();
        let (mut event, _session_id) = seed_completion(&state, &workspace);
        event.user_id = Some("bob".into());

        let error = validate_child_completion(&state, event)
            .expect_err("foreign completion owner must be rejected");
        assert!(error.to_string().contains("owner does not match"));
    }
}
