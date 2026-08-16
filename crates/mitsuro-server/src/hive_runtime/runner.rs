use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, RwLock};

use mitsuro_core::agent::learning::{
    review_latest_completed_hive_turn, PostTurnLearningReviewRequest,
};
use mitsuro_core::agent::{
    LoopEvent, OrchestratorServices, RunBudget, RunProvenance, RunSpecBuilder,
};
use mitsuro_core::ai::client::CallOptions;
use mitsuro_core::ai::types::{Role, WebFetchConfig, WebSearchConfig};
use mitsuro_core::plan::PlanManager;
use mitsuro_core::skills::SkillsManager;
use mitsuro_core::storage::{
    Database, HiveProfileOwner, HiveProfileStore, HiveRuntimeStateStatus, HiveRuntimeStateStore,
    ProjectSettings, SessionManager, SessionType,
};

use super::outcome::HiveRunOutcome;
use super::state::{
    apply_runtime_event_state, load_conversation, persist_runtime_state,
    resolve_persisted_project_dir, with_registered_session_input,
};
use super::HiveRuntimeManager;
use crate::hive_execution_host::{validate_execution_spec, HiveExecutionSpec};
use crate::types::AgenticEvent;
use crate::AppState;

#[derive(Clone)]
pub(crate) enum HiveExecutionEventSink {
    Broadcast(broadcast::Sender<AgenticEvent>),
    Bounded(mpsc::Sender<AgenticEvent>),
}

impl HiveExecutionEventSink {
    pub(crate) async fn send(&self, event: AgenticEvent) -> Result<()> {
        match self {
            // Background embedded runs are valid without an attached observer.
            Self::Broadcast(sender) => {
                let _ = sender.send(event);
                Ok(())
            }
            // The daemon path is lossless and bounded. A vanished consumer
            // must stop the run instead of silently continuing unmanaged.
            Self::Bounded(sender) => sender
                .send(event)
                .await
                .map_err(|_| anyhow::anyhow!("Hive execution event consumer closed")),
        }
    }
}

pub(super) async fn run_hive_session(
    state: AppState,
    session_id: String,
    run_id: String,
    wake_reason: String,
    event_tx: broadcast::Sender<AgenticEvent>,
    manager: Arc<HiveRuntimeManager>,
) {
    let result = run_hive_session_inner(
        state.clone(),
        session_id.clone(),
        run_id.clone(),
        wake_reason,
        None,
        HiveExecutionEventSink::Broadcast(event_tx.clone()),
        manager.clone(),
        true,
    )
    .await;

    if let Err(err) = result {
        let _ = event_tx.send(AgenticEvent::Error {
            error: err.to_string(),
        });
        let _ = persist_runtime_state(
            &state.db_path,
            &session_id,
            HiveRuntimeStateStatus::Error,
            None,
            None,
            Some(&err.to_string()),
            Some(run_id.as_str()),
            Some("error"),
        );
    }

    manager.finish_run(&session_id, &run_id).await;
}

pub(crate) async fn run_hive_session_inner(
    state: AppState,
    session_id: String,
    run_id: String,
    wake_reason: String,
    execution_spec: Option<HiveExecutionSpec>,
    event_sink: HiveExecutionEventSink,
    manager: Arc<HiveRuntimeManager>,
    allow_embedded_wakes: bool,
) -> Result<()> {
    let _guard = state
        .try_lock_session(&session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("session is busy"))?;

    if let Some(spec) = execution_spec.as_ref() {
        anyhow::ensure!(
            spec.session_id() == session_id && spec.run_id() == run_id,
            "Hive execution spec does not match the hosted session/run"
        );
        validate_execution_spec(state.db_path.as_ref().clone(), spec.clone()).await?;
    }

    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);
    let session = session_manager
        .get_session(&session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;
    if session.session_type != SessionType::Hive {
        anyhow::bail!("session is not a hive session")
    }

    // Steering accepted by an execution that exited before its next model
    // boundary remains staged under a non-canonical role. Promote it before
    // loading history so a replacement run observes it exactly once. If a
    // live steer races this recovery, its pending id still makes promotion at
    // the normal boundary idempotent.
    session_manager.promote_orphaned_pending_steering(&session_id)?;

    let claimed_model = execution_spec
        .as_ref()
        .map(|spec| Some(spec.model.as_str()))
        .unwrap_or(session.model.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Hive execution has no frozen model"))?;
    let claimed_model_key = execution_spec
        .as_ref()
        .map(|spec| spec.model_key.as_ref())
        .unwrap_or(session.model_key.as_ref());
    let model_key = match claimed_model_key {
        Some(key) => {
            anyhow::ensure!(
                key.model_id == claimed_model,
                "Hive frozen model does not match its exact model key"
            );
            key.clone()
        }
        None => state
            .model_registry
            .resolve_legacy_key(claimed_model)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "legacy Hive model cannot be resolved to one exact runtime: {error}"
                )
            })?,
    };
    let ai_client = state
        .resolve_ai_client_for_key_for_user(&model_key, session.user_id.as_deref())
        .await
        .ok_or_else(|| anyhow::anyhow!("No AI credentials configured"))?;
    let claimed_catalog_revision = execution_spec
        .as_ref()
        .map(|spec| spec.model_catalog_revision.as_deref())
        .unwrap_or(session.model_catalog_revision.as_deref());
    tracing::info!(
        model_key = ?model_key,
        claimed_catalog_revision,
        resolved_catalog_revision = ai_client.resolved_model().catalog_revision.as_deref(),
        "Resolved exact Hive model runtime"
    );
    let learning_ai_client = Arc::clone(&ai_client);
    let learning_model = ai_client.config().model.clone();

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
    let working_dir = execution_spec
        .as_ref()
        .and_then(|spec| {
            spec.working_dir
                .clone()
                .or_else(|| spec.project_dir.clone())
        })
        .or_else(|| {
            execution_spec.is_none().then(|| {
                session
                    .working_dir
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| (*state.working_dir).clone())
            })
        })
        .unwrap_or_else(|| (*state.working_dir).clone());
    let claimed_project_dir = execution_spec
        .as_ref()
        .map(|spec| spec.project_dir.as_ref().and_then(|path| path.to_str()))
        .unwrap_or(session.project_dir.as_deref());
    let project_dir = resolve_persisted_project_dir(claimed_project_dir, &working_dir);
    let skills_manager = execution_skills_manager(&working_dir, project_dir.as_deref());
    let hive_settings = ProjectSettings::load_hive_settings_checked(project_dir.as_deref())?;
    let runtime_state =
        HiveRuntimeStateStore::new(Database::new(&state.db_path)?).get_state(&session_id)?;
    let profile_owner = HiveProfileOwner::from_user_id(session.user_id.as_deref())?;
    let profile_store = HiveProfileStore::new(Database::new(&state.db_path)?);
    if profile_owner.is_local() {
        profile_store.import_local_legacy_home(&profile_owner, &mitsuro_core::paths::hive_dir())?;
    }
    let hive_profile =
        std::sync::Arc::new(profile_store.bootstrap_defaults(&profile_owner)?.snapshot);

    let options = CallOptions {
        tools: Some(state.tool_registry.get_ai_tools_all().await),
        session_id: Some(session_id.clone()),
        codex_parallel_tool_calls: true,
        web_search: Some(WebSearchConfig::default()),
        web_fetch: Some(WebFetchConfig::default()),
        // Hive's coordinator/persona layers are context sections so the base
        // Mitsuro safety/runtime contract remains intact.
        system_prompt: None,
        ..Default::default()
    };

    let run_spec = RunSpecBuilder::new(
        RunProvenance::Hive,
        session_id.clone(),
        working_dir,
        SessionType::Hive,
    )
    .project_dir(project_dir)
    .hive_crew_slug(
        execution_spec
            .as_ref()
            .map(|spec| spec.crew_slug.clone())
            .unwrap_or_else(|| runtime_state.and_then(|state| state.crew_slug)),
    )
    .hive_group_run(
        execution_spec
            .as_ref()
            .and_then(|spec| spec.hive_group_run.clone()),
    )
    .hive_profile(Some(hive_profile))
    .permission_mode(
        execution_spec
            .as_ref()
            .map(|spec| spec.permission_mode)
            .unwrap_or(session.permission_mode),
    )
    // Hive is persistent, but no individual autonomous tick is allowed an
    // unbounded parent loop. TickEngine clones this finite budget for each
    // subsequent tick in the same run.
    .run_budget(Some(RunBudget::with_max_turns(
        hive_settings.max_turns_per_tick,
    )))
    .user_id(session.user_id.clone())
    .initial_work_mode(work_mode)
    .generate_title(generate_title)
    .call_options(options)
    .build(ai_client.as_ref())?;
    let services = OrchestratorServices {
        ai_client,
        tool_registry: Arc::clone(&state.tool_registry),
        process_registry: Arc::clone(&state.process_registry),
        db_path: (*state.db_path).clone(),
        skills_manager,
    };

    let (mut event_rx, input_tx) = {
        use mitsuro_core::agent::autonomy::tick_engine::{TickEngine, TickEngineConfig};
        TickEngine::run(
            services,
            run_spec,
            TickEngineConfig {
                tick_interval: Duration::from_secs(hive_settings.tick_interval_secs),
                max_ticks: hive_settings.max_ticks,
                enabled: true,
            },
            conversation,
        )?
    };

    let session_inputs = Arc::clone(&state.session_inputs);
    let user_id = session.user_id.clone();
    let project_scope = execution_spec
        .as_ref()
        .map(|spec| {
            spec.project_dir
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| session.project_dir.clone());
    tracing::debug!(
        session_id,
        run_id,
        wake_reason,
        "Starting Hive agent execution"
    );
    with_registered_session_input(session_inputs, session_id.clone(), input_tx, async {
        let mut outcome = HiveRunOutcome::default();
        let mut saw_finished = false;

        while let Some(loop_event) = event_rx.recv().await {
            outcome
                .record_event(
                    &state,
                    &manager,
                    &session_id,
                    user_id.as_deref(),
                    &loop_event,
                    allow_embedded_wakes,
                )
                .await;

            apply_runtime_event_state(&state.db_path, &session_id, &run_id, &loop_event)?;
            if matches!(
                loop_event,
                LoopEvent::TurnComplete {
                    has_more: false,
                    ..
                }
            ) && outcome.allows_learning_review()
            {
                let review = PostTurnLearningReviewRequest::new(
                    (*state.db_path).clone(),
                    session_id.clone(),
                    Arc::clone(&learning_ai_client),
                    learning_model.clone(),
                );
                tokio::spawn(async move {
                    match review_latest_completed_hive_turn(review).await {
                        Ok(result) if result.skipped => {}
                        Ok(result) => tracing::debug!(
                            through_message_id = ?result.through_message_id,
                            candidates = result.candidates,
                            auto_promoted = result.auto_promoted,
                            tombstoned = result.tombstoned,
                            "Completed governed Hive post-turn learning review"
                        ),
                        Err(error) => tracing::warn!(
                            error = %error,
                            "Background Hive post-turn learning review failed"
                        ),
                    }
                });
            }
            let is_finished = matches!(loop_event, LoopEvent::Finished { .. });
            event_sink.send(loop_event.into()).await?;
            if is_finished {
                saw_finished = true;
                break;
            }
        }

        anyhow::ensure!(
            saw_finished,
            "agent event stream ended before LoopEvent::Finished; external side effects are uncertain"
        );

        outcome
            .finalize(
                &state,
                &manager,
                &session_id,
                user_id.as_deref(),
                project_scope.as_deref(),
                allow_embedded_wakes,
            )
            .await
    })
    .await
}

fn execution_skills_manager(
    working_dir: &Path,
    project_dir: Option<&Path>,
) -> Arc<RwLock<SkillsManager>> {
    let skill_root = project_dir.unwrap_or(working_dir);
    Arc::new(RwLock::new(SkillsManager::with_defaults(skill_root)))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use mitsuro_core::skills::{SkillSource, SkillsManager};
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    use super::execution_skills_manager;

    fn write_skill(root: &Path, name: &str) {
        let directory = root.join(".mitsuro").join("skills").join(name);
        fs::create_dir_all(&directory).expect("project skill directory should exist");
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: project-scoped test skill\n---\n\n# {name}\n"),
        )
        .expect("project skill should be written");
    }

    async fn project_skill_names(manager: &RwLock<SkillsManager>) -> Vec<String> {
        manager
            .write()
            .await
            .list_skills()
            .into_iter()
            .filter(|skill| skill.source == SkillSource::Project)
            .map(|skill| skill.name)
            .collect()
    }

    #[tokio::test]
    async fn execution_skills_are_scoped_to_the_frozen_project() {
        let temp = TempDir::new().expect("temporary root should exist");
        let working = temp.path().join("working");
        let project = temp.path().join("project");
        write_skill(&working, "working-only");
        write_skill(&project, "project-only");

        let project_manager = execution_skills_manager(&working, Some(&project));
        assert_eq!(
            project_skill_names(&project_manager).await,
            vec!["project-only".to_string()]
        );

        let working_manager = execution_skills_manager(&working, None);
        assert_eq!(
            project_skill_names(&working_manager).await,
            vec!["working-only".to_string()]
        );
    }
}
