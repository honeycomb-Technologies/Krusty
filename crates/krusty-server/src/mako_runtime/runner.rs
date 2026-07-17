use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::broadcast;

use krusty_core::agent::autonomy::coordinator_prompt::system_prompt_for_session;
use krusty_core::agent::{LoopEvent, OrchestratorConfig, OrchestratorServices};
use krusty_core::ai::client::CallOptions;
use krusty_core::ai::types::{Role, WebFetchConfig, WebSearchConfig};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{
    Database, MakoRuntimeStateStatus, MakoRuntimeStateStore, ProjectSettings, SessionManager,
    SessionType,
};
use krusty_core::tools::registry::PermissionMode;

use super::outcome::MakoRunOutcome;
use super::state::{
    apply_runtime_event_state, load_conversation, persist_runtime_state,
    resolve_persisted_project_dir, with_registered_session_input,
};
use super::MakoRuntimeManager;
use crate::types::AgenticEvent;
use crate::AppState;

pub(super) async fn run_mako_session(
    state: AppState,
    session_id: String,
    run_id: String,
    wake_reason: String,
    event_tx: broadcast::Sender<AgenticEvent>,
    manager: Arc<MakoRuntimeManager>,
) {
    let result = run_mako_session_inner(
        state.clone(),
        session_id.clone(),
        run_id.clone(),
        wake_reason,
        event_tx.clone(),
        manager.clone(),
    )
    .await;

    if let Err(err) = result {
        let _ = event_tx.send(AgenticEvent::Error {
            error: err.to_string(),
        });
        let _ = persist_runtime_state(
            &state.db_path,
            &session_id,
            MakoRuntimeStateStatus::Error,
            None,
            None,
            Some(&err.to_string()),
            Some(run_id.as_str()),
            Some("error"),
        );
    }

    manager.finish_run(&session_id, &run_id).await;
}

async fn run_mako_session_inner(
    state: AppState,
    session_id: String,
    run_id: String,
    _wake_reason: String,
    event_tx: broadcast::Sender<AgenticEvent>,
    manager: Arc<MakoRuntimeManager>,
) -> Result<()> {
    let _guard = state
        .try_lock_session(&session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("session is busy"))?;

    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);
    let session = session_manager
        .get_session(&session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;
    if session.session_type != SessionType::Mako {
        anyhow::bail!("session is not a mako session")
    }

    let ai_client = state
        .resolve_ai_client_for_user(session.model.as_deref(), session.user_id.as_deref())
        .await
        .ok_or_else(|| anyhow::anyhow!("No AI credentials configured"))?;

    let raw_messages = session_manager.load_session_messages(&session_id)?;
    let conversation = load_conversation(raw_messages);
    let generate_title = !conversation
        .iter()
        .any(|message| message.role == Role::Assistant);
    let work_mode = PlanManager::new((*state.db_path).clone())
        .ok()
        .and_then(|pm| pm.get_lifecycle_state(&session_id, session.work_mode).ok())
        .map(|state| state.effective_work_mode)
        .unwrap_or(session.work_mode);
    let working_dir = session
        .working_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| (*state.working_dir).clone());
    let project_dir = resolve_persisted_project_dir(session.project_dir.as_deref(), &working_dir);
    let mako_settings = ProjectSettings::load_mako_settings(project_dir.as_deref());
    let runtime_state =
        MakoRuntimeStateStore::new(Database::new(&state.db_path)?).get_state(&session_id)?;

    let options = CallOptions {
        tools: Some(state.tool_registry.get_ai_tools_all().await),
        session_id: Some(session_id.clone()),
        codex_parallel_tool_calls: true,
        web_search: Some(WebSearchConfig::default()),
        web_fetch: Some(WebFetchConfig::default()),
        system_prompt: system_prompt_for_session(SessionType::Mako),
        ..Default::default()
    };

    let services = OrchestratorServices {
        ai_client,
        tool_registry: Arc::clone(&state.tool_registry),
        process_registry: Arc::clone(&state.process_registry),
        db_path: (*state.db_path).clone(),
        skills_manager: Arc::clone(&state.skills_manager),
    };
    let config = OrchestratorConfig {
        session_id: session_id.clone(),
        working_dir,
        project_dir,
        mako_crew_slug: runtime_state.and_then(|state| state.crew_slug),
        session_type: SessionType::Mako,
        permission_mode: PermissionMode::Autonomous,
        user_id: session.user_id.clone(),
        initial_work_mode: work_mode,
        generate_title,
        ..Default::default()
    };

    let (mut event_rx, input_tx) = {
        use krusty_core::agent::autonomy::tick_engine::{TickEngine, TickEngineConfig};
        TickEngine::run(
            services,
            config,
            TickEngineConfig {
                tick_interval: Duration::from_secs(mako_settings.tick_interval_secs),
                max_ticks: mako_settings.max_ticks,
                enabled: true,
            },
            conversation,
            options,
        )
    };

    let session_inputs = Arc::clone(&state.session_inputs);
    let user_id = session.user_id.clone();
    let project_scope = session.project_dir.clone();
    with_registered_session_input(session_inputs, session_id.clone(), input_tx, async {
        let mut outcome = MakoRunOutcome::default();

        while let Some(loop_event) = event_rx.recv().await {
            outcome
                .record_event(
                    &state,
                    &manager,
                    &session_id,
                    user_id.as_deref(),
                    &loop_event,
                )
                .await;

            apply_runtime_event_state(&state.db_path, &session_id, &run_id, &loop_event)?;
            let is_finished = matches!(loop_event, LoopEvent::Finished { .. });
            let _ = event_tx.send(loop_event.into());
            if is_finished {
                break;
            }
        }

        outcome
            .finalize(
                &state,
                &manager,
                &session_id,
                user_id.as_deref(),
                project_scope.as_deref(),
            )
            .await
    })
    .await
}
