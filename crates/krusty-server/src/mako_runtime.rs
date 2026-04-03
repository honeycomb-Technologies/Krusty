use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use krusty_core::agent::coordinator_prompt::COORDINATOR_SYSTEM_PROMPT;
use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::agent::{LoopEvent, LoopInput, OrchestratorConfig, OrchestratorServices};
use krusty_core::ai::client::CallOptions;
use krusty_core::ai::types::{ModelMessage, Role};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{
    Database, MakoRuntimeStateStatus, MakoRuntimeStateStore, SessionManager, SessionType,
};
use krusty_core::tools::registry::PermissionMode;

use crate::types::AgenticEvent;
use crate::AppState;

const MAKO_EVENT_BUFFER: usize = 256;

type SseItem = std::result::Result<Event, Infallible>;
type MakoSse = Sse<ReceiverStream<SseItem>>;

pub struct MakoRuntimeManager {
    runtimes: RwLock<HashMap<String, ActiveMakoRuntime>>,
    event_streams: RwLock<HashMap<String, broadcast::Sender<AgenticEvent>>>,
    scheduled_wakes: RwLock<HashMap<String, JoinHandle<()>>>,
    wake_tx: mpsc::UnboundedSender<WakeCommand>,
}

struct ActiveMakoRuntime {
    run_id: String,
    join_handle: JoinHandle<()>,
}

struct WakeCommand {
    state: AppState,
    session_id: String,
    wake_reason: String,
}

impl MakoRuntimeManager {
    pub fn new() -> Arc<Self> {
        let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
        let manager = Arc::new(Self {
            runtimes: RwLock::new(HashMap::new()),
            event_streams: RwLock::new(HashMap::new()),
            scheduled_wakes: RwLock::new(HashMap::new()),
            wake_tx,
        });

        let weak_manager = Arc::downgrade(&manager);
        tokio::spawn(async move {
            while let Some(command) = wake_rx.recv().await {
                let Some(manager) = weak_manager.upgrade() else {
                    break;
                };

                let WakeCommand {
                    state,
                    session_id,
                    wake_reason,
                } = command;

                if let Err(err) = manager
                    .start_or_restart_session(state, session_id.clone(), &wake_reason)
                    .await
                {
                    tracing::error!(
                        session_id = %session_id,
                        error = %err,
                        "Failed to resume sleeping Mako session"
                    );
                }
            }
        });

        manager
    }

    async fn event_sender(&self, session_id: &str) -> broadcast::Sender<AgenticEvent> {
        if let Some(sender) = self.event_streams.read().await.get(session_id).cloned() {
            return sender;
        }

        let mut streams = self.event_streams.write().await;
        streams
            .entry(session_id.to_string())
            .or_insert_with(|| {
                let (event_tx, _event_rx) = broadcast::channel(MAKO_EVENT_BUFFER);
                event_tx
            })
            .clone()
    }

    pub async fn forget_session(&self, session_id: &str) {
        self.cancel_scheduled_wake(session_id).await;
        self.event_streams.write().await.remove(session_id);
    }

    pub async fn restore_persisted_sessions(&self, state: AppState) -> Result<()> {
        let runtime_store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);
        let session_manager = SessionManager::new(Database::new(&state.db_path)?);

        for runtime_state in runtime_store.list_recoverable_states()? {
            let Some(session) = session_manager.get_session(&runtime_state.session_id)? else {
                continue;
            };
            if session.session_type != SessionType::Mako {
                continue;
            }

            match runtime_state.status {
                MakoRuntimeStateStatus::Running => {
                    tracing::info!(
                        session_id = %runtime_state.session_id,
                        "Recovering running Mako session after server startup"
                    );
                    self.start_or_restart_session(
                        state.clone(),
                        runtime_state.session_id.clone(),
                        "startup_recover_running",
                    )
                    .await?;
                }
                MakoRuntimeStateStatus::Sleeping => {
                    let wake_at = runtime_state
                        .next_wake_at
                        .as_deref()
                        .and_then(parse_wake_at);
                    match wake_at {
                        Some(wake_at) if wake_at > chrono::Utc::now() => {
                            tracing::info!(
                                session_id = %runtime_state.session_id,
                                wake_at = %wake_at,
                                "Scheduling persisted sleeping Mako session after server startup"
                            );
                            self.schedule_wake_at(
                                state.clone(),
                                runtime_state.session_id.clone(),
                                wake_at,
                                "startup_resume_sleep",
                            )
                            .await;
                        }
                        _ => {
                            tracing::info!(
                                session_id = %runtime_state.session_id,
                                "Resuming persisted sleeping Mako session immediately after server startup"
                            );
                            self.start_or_restart_session(
                                state.clone(),
                                runtime_state.session_id.clone(),
                                "startup_resume_sleep",
                            )
                            .await?;
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    pub async fn start_or_restart_session(
        &self,
        state: AppState,
        session_id: String,
        wake_reason: &str,
    ) -> Result<()> {
        self.stop_active_run(&state, &session_id).await;

        let event_tx = self.event_sender(&session_id).await;
        let run_id = Uuid::new_v4().to_string();
        persist_runtime_state(
            &state.db_path,
            &session_id,
            MakoRuntimeStateStatus::Running,
            None,
            None,
            None,
            Some(run_id.as_str()),
            Some(wake_reason),
        )?;

        let state_clone = state.clone();
        let session_id_clone = session_id.clone();
        let run_id_clone = run_id.clone();
        let event_tx_clone = event_tx.clone();
        let wake_reason_owned = wake_reason.to_string();
        let manager = state.mako_runtime.clone();
        let join_handle = tokio::spawn(async move {
            run_mako_session(
                state_clone,
                session_id_clone,
                run_id_clone,
                wake_reason_owned,
                event_tx_clone,
                manager,
            )
            .await;
        });

        self.runtimes.write().await.insert(
            session_id,
            ActiveMakoRuntime {
                run_id,
                join_handle,
            },
        );
        Ok(())
    }

    pub async fn subscribe(&self, session_id: &str) -> broadcast::Receiver<AgenticEvent> {
        self.event_sender(session_id).await.subscribe()
    }

    pub async fn observe(&self, session_id: &str) -> MakoSse {
        let receiver = self.subscribe(session_id).await;

        let (tx, rx) = mpsc::channel::<SseItem>(MAKO_EVENT_BUFFER);
        tokio::spawn(async move {
            let mut receiver = receiver;
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let Ok(sse_event) = Event::default().json_data(event) else {
                            continue;
                        };
                        if tx.send(Ok(sse_event)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
    }

    pub async fn pause_session(&self, state: &AppState, session_id: &str) -> Result<()> {
        self.stop_active_run(state, session_id).await;
        persist_runtime_state(
            &state.db_path,
            session_id,
            MakoRuntimeStateStatus::Paused,
            None,
            None,
            None,
            None,
            Some("paused"),
        )
    }

    pub async fn stop_active_run(&self, state: &AppState, session_id: &str) {
        self.cancel_scheduled_wake(session_id).await;

        let runtime = self.runtimes.write().await.remove(session_id);
        let Some(runtime) = runtime else {
            return;
        };

        let maybe_input = {
            let inputs = state.session_inputs.read().await;
            inputs.get(session_id).cloned()
        };

        let join_handle = runtime.join_handle;
        let sent_cancel = maybe_input
            .as_ref()
            .map(|sender| sender.send(LoopInput::Cancel).is_ok())
            .unwrap_or(false);

        if !sent_cancel {
            join_handle.abort();
        }
        let _ = join_handle.await;
    }

    pub async fn finish_run(&self, session_id: &str, run_id: &str) {
        let mut runtimes = self.runtimes.write().await;
        let should_remove = runtimes
            .get(session_id)
            .map(|runtime| runtime.run_id == run_id)
            .unwrap_or(false);
        if should_remove {
            runtimes.remove(session_id);
        }
    }

    async fn schedule_wake_at(
        &self,
        state: AppState,
        session_id: String,
        wake_at: chrono::DateTime<chrono::Utc>,
        wake_reason: &'static str,
    ) {
        self.cancel_scheduled_wake(&session_id).await;

        let delay = (wake_at - chrono::Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO);
        let manager = state.mako_runtime.clone();
        let wake_tx = self.wake_tx.clone();
        let session_id_clone = session_id.clone();
        let wake_reason_owned = wake_reason.to_string();
        let join_handle = tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            manager.clear_scheduled_wake(&session_id_clone).await;
            if wake_tx
                .send(WakeCommand {
                    state,
                    session_id: session_id_clone.clone(),
                    wake_reason: wake_reason_owned,
                })
                .is_err()
            {
                tracing::error!(
                    session_id = %session_id_clone,
                    "Failed to queue sleeping Mako session for wake"
                );
            }
        });

        self.scheduled_wakes
            .write()
            .await
            .insert(session_id, join_handle);
    }

    async fn clear_scheduled_wake(&self, session_id: &str) {
        self.scheduled_wakes.write().await.remove(session_id);
    }

    async fn cancel_scheduled_wake(&self, session_id: &str) {
        let handle = self.scheduled_wakes.write().await.remove(session_id);
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }
    }
}

async fn run_mako_session(
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
    let session_lock = {
        let mut locks = state.session_locks.write().await;
        let (lock, _) = locks.entry(session_id.clone()).or_insert_with(|| {
            (
                Arc::new(tokio::sync::Mutex::new(())),
                std::time::Instant::now(),
            )
        });
        lock.clone()
    };
    let _guard = Arc::clone(&session_lock)
        .try_lock_owned()
        .map_err(|_| anyhow::anyhow!("session is busy"))?;

    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);
    let session = session_manager
        .get_session(&session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;
    if session.session_type != SessionType::Mako {
        anyhow::bail!("session is not a mako session")
    }

    let ai_client = state
        .resolve_ai_client(session.model.as_deref())
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

    let options = CallOptions {
        tools: Some(state.tool_registry.get_ai_tools().await),
        session_id: Some(session_id.clone()),
        codex_parallel_tool_calls: true,
        system_prompt: Some(COORDINATOR_SYSTEM_PROMPT.to_string()),
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
        permission_mode: PermissionMode::Autonomous,
        user_id: session.user_id.clone(),
        initial_work_mode: work_mode,
        generate_title,
        ..Default::default()
    };

    let (mut event_rx, input_tx) = {
        use krusty_core::agent::tick_engine::{TickEngine, TickEngineConfig};
        TickEngine::run(
            services,
            config,
            TickEngineConfig {
                tick_interval: Duration::from_secs(30),
                max_ticks: 1000,
                enabled: true,
            },
            conversation,
            options,
        )
    };

    state
        .session_inputs
        .write()
        .await
        .insert(session_id.clone(), input_tx);

    while let Some(loop_event) = event_rx.recv().await {
        if let LoopEvent::AgentSleeping { duration_secs, .. } = &loop_event {
            let wake_at = chrono::Utc::now() + chrono::Duration::seconds(*duration_secs as i64);
            manager
                .schedule_wake_at(state.clone(), session_id.clone(), wake_at, "sleep")
                .await;
        }
        apply_runtime_event_state(&state.db_path, &session_id, &run_id, &loop_event)?;
        let is_finished = matches!(loop_event, LoopEvent::Finished { .. });
        let _ = event_tx.send(loop_event.into());
        if is_finished {
            break;
        }
    }

    state.session_inputs.write().await.remove(&session_id);
    Ok(())
}

fn load_conversation(raw_messages: Vec<(String, String)>) -> Vec<ModelMessage> {
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

fn apply_runtime_event_state(
    db_path: &PathBuf,
    session_id: &str,
    run_id: &str,
    event: &LoopEvent,
) -> Result<()> {
    match event {
        LoopEvent::AgentSleeping {
            duration_secs,
            reason,
        } => {
            let wake_at = chrono::Utc::now() + chrono::Duration::seconds(*duration_secs as i64);
            persist_runtime_state(
                db_path,
                session_id,
                MakoRuntimeStateStatus::Sleeping,
                Some(&wake_at.to_rfc3339()),
                Some(reason),
                None,
                None,
                Some("sleep"),
            )?;
        }
        LoopEvent::AwaitingInput { .. } => {
            persist_runtime_state(
                db_path,
                session_id,
                MakoRuntimeStateStatus::AwaitingInput,
                None,
                None,
                None,
                Some(run_id),
                Some("awaiting_input"),
            )?;
        }
        LoopEvent::Finished { stop_reason, .. } => {
            let status = match stop_reason {
                LoopStopReason::Sleeping => return Ok(()),
                LoopStopReason::AwaitingInput => MakoRuntimeStateStatus::AwaitingInput,
                LoopStopReason::ProviderError | LoopStopReason::ContextCompactionFailed => {
                    MakoRuntimeStateStatus::Error
                }
                _ => MakoRuntimeStateStatus::Idle,
            };
            persist_runtime_state(db_path, session_id, status, None, None, None, None, None)?;
        }
        LoopEvent::Error { error } => {
            persist_runtime_state(
                db_path,
                session_id,
                MakoRuntimeStateStatus::Error,
                None,
                None,
                Some(error),
                Some(run_id),
                Some("error"),
            )?;
        }
        _ => {
            persist_runtime_state(
                db_path,
                session_id,
                MakoRuntimeStateStatus::Running,
                None,
                None,
                None,
                Some(run_id),
                Some("running"),
            )?;
        }
    }
    Ok(())
}

fn parse_wake_at(next_wake_at: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(next_wake_at)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn persist_runtime_state(
    db_path: &PathBuf,
    session_id: &str,
    status: MakoRuntimeStateStatus,
    next_wake_at: Option<&str>,
    sleep_reason: Option<&str>,
    last_error: Option<&str>,
    current_run_id: Option<&str>,
    last_wake_reason: Option<&str>,
) -> Result<()> {
    let store = MakoRuntimeStateStore::new(Database::new(db_path)?);
    store.set_state(
        session_id,
        status,
        next_wake_at,
        sleep_reason,
        last_error,
        current_run_id,
        last_wake_reason,
    )
}
