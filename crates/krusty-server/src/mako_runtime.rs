mod notify;
mod outcome;
mod runner;
mod state;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use krusty_core::agent::LoopInput;
use krusty_core::storage::{
    Database, MakoRuntimeStateStatus, MakoRuntimeStateStore, SessionManager, SessionType,
};

#[cfg(test)]
use self::notify::mako_notification_title;
use self::runner::run_mako_session;
#[cfg(test)]
use self::state::{
    apply_runtime_event_state, refresh_snapshot_after_run, resolve_persisted_project_dir,
    with_registered_session_input,
};
use self::state::{ensure_runnable_mako_session, parse_wake_at, persist_runtime_state};
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
    started_at: Instant,
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

#[derive(Debug, Clone, Copy, Default)]
pub struct MakoRuntimeStats {
    pub active_runtime_count: usize,
    pub scheduled_wake_count: usize,
    pub event_stream_count: usize,
    pub uptime_secs: u64,
}

impl MakoRuntimeManager {
    pub fn new() -> Arc<Self> {
        let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
        let manager = Arc::new(Self {
            runtimes: RwLock::new(HashMap::new()),
            event_streams: RwLock::new(HashMap::new()),
            scheduled_wakes: RwLock::new(HashMap::new()),
            wake_tx,
            started_at: Instant::now(),
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

            self.recover_persisted_state(state.clone(), &runtime_state, "startup_recover")
                .await?;
        }

        Ok(())
    }

    pub async fn recover_persisted_state(
        &self,
        state: AppState,
        runtime_state: &krusty_core::storage::MakoRuntimeState,
        wake_reason: &'static str,
    ) -> Result<()> {
        match runtime_state.status {
            MakoRuntimeStateStatus::Running => {
                tracing::info!(
                    session_id = %runtime_state.session_id,
                    "Recovering persisted running Mako session"
                );
                self.start_or_restart_session(state, runtime_state.session_id.clone(), wake_reason)
                    .await
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
                            "Scheduling persisted sleeping Mako session"
                        );
                        self.stop_active_run(&state, &runtime_state.session_id)
                            .await;
                        ensure_runnable_mako_session(&state.db_path, &runtime_state.session_id)?;
                        persist_runtime_state(
                            &state.db_path,
                            &runtime_state.session_id,
                            MakoRuntimeStateStatus::Sleeping,
                            Some(&wake_at.to_rfc3339()),
                            runtime_state.sleep_reason.as_deref(),
                            None,
                            None,
                            Some(wake_reason),
                        )?;
                        self.schedule_wake_at(
                            state,
                            runtime_state.session_id.clone(),
                            wake_at,
                            wake_reason,
                        )
                        .await;
                        Ok(())
                    }
                    _ => {
                        tracing::info!(
                            session_id = %runtime_state.session_id,
                            "Resuming persisted sleeping Mako session immediately"
                        );
                        self.start_or_restart_session(
                            state,
                            runtime_state.session_id.clone(),
                            wake_reason,
                        )
                        .await
                    }
                }
            }
            _ => Ok(()),
        }
    }

    pub async fn stats_for_sessions(&self, session_ids: &[String]) -> MakoRuntimeStats {
        let session_ids = session_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let active_runtime_count = self
            .runtimes
            .read()
            .await
            .keys()
            .filter(|session_id| session_ids.contains(session_id.as_str()))
            .count();
        let scheduled_wake_count = self
            .scheduled_wakes
            .read()
            .await
            .keys()
            .filter(|session_id| session_ids.contains(session_id.as_str()))
            .count();
        let event_stream_count = self
            .event_streams
            .read()
            .await
            .keys()
            .filter(|session_id| session_ids.contains(session_id.as_str()))
            .count();

        MakoRuntimeStats {
            active_runtime_count,
            scheduled_wake_count,
            event_stream_count,
            uptime_secs: self.started_at.elapsed().as_secs(),
        }
    }

    pub async fn start_or_restart_session(
        &self,
        state: AppState,
        session_id: String,
        wake_reason: &str,
    ) -> Result<()> {
        self.stop_active_run(&state, &session_id).await;
        ensure_runnable_mako_session(&state.db_path, &session_id)?;
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

        let event_tx = self.event_sender(&session_id).await;
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

    pub async fn schedule_session(
        &self,
        state: &AppState,
        session_id: String,
        wake_at: chrono::DateTime<chrono::Utc>,
        wake_reason: &'static str,
        sleep_reason: &'static str,
    ) -> Result<()> {
        self.stop_active_run(state, &session_id).await;
        ensure_runnable_mako_session(&state.db_path, &session_id)?;
        persist_runtime_state(
            &state.db_path,
            &session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some(&wake_at.to_rfc3339()),
            Some(sleep_reason),
            None,
            None,
            Some(wake_reason),
        )?;
        self.schedule_wake_at(state.clone(), session_id, wake_at, wake_reason)
            .await;
        Ok(())
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
        state.session_inputs.write().await.remove(session_id);
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::loop_events::LoopStopReason;
    use krusty_core::agent::{AgentCancellation, LoopEvent, LoopInput, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::reports::CreateReportInput;
    use krusty_core::storage::{
        Database, MakoRuntimeStateStatus, MakoRuntimeStateStore, MemoryStore, MemoryType,
        ReportStore, SessionType, WorkspaceMode, CURRENT_SNAPSHOT_TITLE,
    };
    use krusty_core::tools::registry::ToolRegistry;
    use krusty_core::SessionManager;

    use super::{
        apply_runtime_event_state, mako_notification_title, persist_runtime_state,
        refresh_snapshot_after_run, resolve_persisted_project_dir, with_registered_session_input,
        ActiveMakoRuntime, MakoRuntimeManager,
    };
    use crate::AppState;

    fn create_test_state() -> (AppState, PathBuf) {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-server-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("krusty.db");
        Database::new(&db_path).expect("database should initialize");
        let working_dir = temp_dir.join("workspace");
        std::fs::create_dir_all(&working_dir).expect("workspace should exist");

        (
            AppState {
                server_port: 3000,
                db_path: Arc::new(db_path),
                working_dir: Arc::new(working_dir.clone()),
                ai_client: None,
                tool_registry: Arc::new(ToolRegistry::new()),
                process_registry: Arc::new(ProcessRegistry::new()),
                model_registry: create_model_registry(),
                credential_store: Arc::new(RwLock::new(CredentialStore::default())),
                mcp_manager: Arc::new(McpManager::new(working_dir.clone())),
                hook_manager: Arc::new(RwLock::new(UserHookManager::new())),
                skills_manager: Arc::new(RwLock::new(SkillsManager::with_defaults(&working_dir))),
                cancellation: AgentCancellation::new(),
                session_locks: Arc::new(RwLock::new(HashMap::new())),
                session_inputs: Arc::new(RwLock::new(HashMap::new())),
                session_presence: Arc::new(RwLock::new(HashMap::new())),
                delegated_state: Arc::new(RwLock::new(HashMap::new())),
                remote_access: Arc::new(RwLock::new(crate::remote_access::RemoteAccessConfig {
                    enabled: true,
                    token: "test-token".to_string(),
                })),
                active_agent_streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                peak_rss_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                peak_virtual_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                push_service: None,
                apns_service: None,
                oauth_flows: Arc::new(Mutex::new(HashMap::new())),
                mako_runtime: MakoRuntimeManager::new(),
            },
            temp_dir,
        )
    }

    #[test]
    fn mako_notification_title_prefers_explicit_title() {
        assert_eq!(
            mako_notification_title(Some("Verification complete"), "Auth refactor"),
            "Mako — Verification complete"
        );
    }

    #[test]
    fn mako_notification_title_falls_back_to_session_label() {
        assert_eq!(
            mako_notification_title(Some("   "), "Auth refactor"),
            "Mako — Auth refactor"
        );
        assert_eq!(
            mako_notification_title(None, "Auth refactor"),
            "Mako — Auth refactor"
        );
    }

    fn create_session(state: &AppState, session_type: SessionType) -> String {
        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        session_manager
            .create_session_for_user_with_config(
                "Test Session",
                None,
                None,
                None,
                WorkspaceMode::Neutral,
                None,
                None,
                session_type,
            )
            .expect("session should create")
    }

    fn create_test_user(state: &AppState, user_id: &str) {
        let db = Database::new(&state.db_path).expect("database should open");
        db.conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
                (user_id, format!("{user_id}@example.com"), "free"),
            )
            .expect("user should insert");
    }

    #[test]
    fn resolve_persisted_project_dir_supports_relative_and_absolute_paths() {
        let workspace = Path::new("/workspace");

        assert_eq!(
            resolve_persisted_project_dir(Some("repo"), workspace),
            Some(workspace.join("repo"))
        );
        assert_eq!(
            resolve_persisted_project_dir(Some("/tmp/project"), workspace),
            Some(PathBuf::from("/tmp/project"))
        );
        assert_eq!(resolve_persisted_project_dir(Some("   "), workspace), None);
    }

    #[tokio::test]
    async fn start_or_restart_session_rejects_missing_session_without_leaking_runtime_state() {
        let (state, _temp_dir) = create_test_state();

        let result = state
            .mako_runtime
            .start_or_restart_session(state.clone(), "missing-session".to_string(), "test")
            .await;

        assert!(result.is_err());
        assert!(!state
            .mako_runtime
            .event_streams
            .read()
            .await
            .contains_key("missing-session"));
        assert!(!state
            .mako_runtime
            .runtimes
            .read()
            .await
            .contains_key("missing-session"));

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
        assert!(runtime_store
            .get_state("missing-session")
            .expect("runtime state lookup should succeed")
            .is_none());
    }

    #[tokio::test]
    async fn start_or_restart_session_rejects_non_mako_session_without_persisting_runtime_state() {
        let (state, _temp_dir) = create_test_state();
        let session_id = create_session(&state, SessionType::Code);

        let result = state
            .mako_runtime
            .start_or_restart_session(state.clone(), session_id.clone(), "test")
            .await;

        assert!(result.is_err());
        assert!(!state
            .mako_runtime
            .event_streams
            .read()
            .await
            .contains_key(&session_id));
        assert!(!state
            .mako_runtime
            .runtimes
            .read()
            .await
            .contains_key(&session_id));

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
        assert!(runtime_store
            .get_state(&session_id)
            .expect("runtime state lookup should succeed")
            .is_none());
    }

    #[tokio::test]
    async fn with_registered_session_input_clears_entry_on_error() {
        let session_inputs = Arc::new(RwLock::new(HashMap::new()));
        let (input_tx, _input_rx) = tokio::sync::mpsc::unbounded_channel::<LoopInput>();

        let result: anyhow::Result<()> = with_registered_session_input(
            Arc::clone(&session_inputs),
            "session-1".to_string(),
            input_tx,
            async { anyhow::bail!("boom") },
        )
        .await;

        assert!(result.is_err());
        assert!(session_inputs.read().await.is_empty());
    }

    #[tokio::test]
    async fn stop_active_run_clears_session_input_when_runtime_is_aborted() {
        let (state, _temp_dir) = create_test_state();
        let session_id = "session-1".to_string();
        let join_handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });

        state.mako_runtime.runtimes.write().await.insert(
            session_id.clone(),
            ActiveMakoRuntime {
                run_id: "run-1".to_string(),
                join_handle,
            },
        );

        let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<LoopInput>();
        drop(input_rx);
        state
            .session_inputs
            .write()
            .await
            .insert(session_id.clone(), input_tx);

        state
            .mako_runtime
            .stop_active_run(&state, &session_id)
            .await;

        assert!(!state
            .mako_runtime
            .runtimes
            .read()
            .await
            .contains_key(&session_id));
        assert!(state.session_inputs.read().await.get(&session_id).is_none());
    }

    #[tokio::test]
    async fn apply_runtime_event_state_preserves_existing_wake_reason_for_running_updates() {
        let (state, _temp_dir) = create_test_state();
        let session_id = create_session(&state, SessionType::Mako);
        persist_runtime_state(
            &state.db_path,
            &session_id,
            MakoRuntimeStateStatus::Running,
            None,
            None,
            None,
            Some("run-1"),
            Some("dispatch"),
        )
        .expect("seed runtime state");

        apply_runtime_event_state(
            &state.db_path,
            &session_id,
            "run-2",
            &LoopEvent::ToolCallStart {
                id: "tool-1".to_string(),
                name: "read".to_string(),
            },
        )
        .expect("running update should persist");

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
        let runtime = runtime_store
            .get_state(&session_id)
            .expect("runtime lookup should succeed")
            .expect("runtime state should exist");

        assert_eq!(runtime.status, MakoRuntimeStateStatus::Running);
        assert_eq!(runtime.current_run_id.as_deref(), Some("run-2"));
        assert_eq!(runtime.last_wake_reason.as_deref(), Some("dispatch"));
    }

    #[tokio::test]
    async fn refresh_snapshot_after_run_updates_stale_snapshot_content() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let project_dir = user_root.join("repo");
        std::fs::create_dir_all(&project_dir).expect("project dir should exist");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session_for_user_with_config(
                "Knowledge Run",
                None,
                Some(project_dir.to_string_lossy().as_ref()),
                Some(project_dir.to_string_lossy().as_ref()),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Mako,
            )
            .expect("session should create");

        let report_store =
            ReportStore::new(Database::new(&state.db_path).expect("database should open"));
        report_store
            .create_report(CreateReportInput {
                title: "Fresh findings",
                session_id: session_id.as_str(),
                project_dir: Some(project_dir.to_string_lossy().as_ref()),
                report_root: None,
                content: "Fresh findings",
                summary: "Fresh findings",
                tags: &[],
                sources: &[],
            })
            .expect("report should persist");

        let memory_store =
            MemoryStore::new(Database::new(&state.db_path).expect("database should open"));
        let snapshot = memory_store
            .save(
                MemoryType::Project,
                CURRENT_SNAPSHOT_TITLE,
                "Stale snapshot",
                Some(project_dir.to_string_lossy().as_ref()),
                Some("alice"),
            )
            .expect("snapshot should persist");
        Database::new(&state.db_path)
            .expect("database should open")
            .conn()
            .execute(
                "UPDATE agent_memories SET updated_at = ?1 WHERE id = ?2",
                ("2025-01-01T00:00:00Z", snapshot.id.as_str()),
            )
            .expect("snapshot should backdate");

        refresh_snapshot_after_run(
            &state.db_path,
            Some(project_dir.to_string_lossy().as_ref()),
            Some("alice"),
            Some(&LoopStopReason::Completed),
        );

        let refreshed = memory_store
            .find_by_title_in_exact_scope(
                CURRENT_SNAPSHOT_TITLE,
                Some(project_dir.to_string_lossy().as_ref()),
                Some("alice"),
            )
            .expect("snapshot should still exist");

        assert_ne!(refreshed.updated_at, "2025-01-01T00:00:00Z");
        assert!(refreshed.content.contains("Fresh findings"));
    }
}
