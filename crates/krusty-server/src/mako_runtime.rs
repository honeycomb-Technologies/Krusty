mod ipc;
mod notify;
mod outcome;
pub(crate) mod runner;
mod state;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use krusty_core::agent::LoopInput;
use krusty_core::storage::{
    Database, MakoRunPriority, MakoRuntimeStateStatus, MakoRuntimeStateStore, SessionManager,
    SessionType,
};
use krusty_mako_protocol::ClientError;

use self::ipc::{map_daemon_event, MakoDaemonControl, MakoDaemonError};
#[cfg(test)]
use self::notify::mako_notification_title;
use self::runner::run_mako_session;
#[cfg(test)]
use self::state::{
    apply_runtime_event_state, refresh_snapshot_after_run, resolve_persisted_project_dir,
    with_registered_session_input,
};
use self::state::{ensure_runnable_mako_session, parse_wake_at, persist_runtime_state};
use crate::error::AppError;
use crate::types::AgenticEvent;
use crate::AppState;

const MAKO_EVENT_BUFFER: usize = 256;
const MAKO_SUBSCRIPTION_RECONNECT_ATTEMPTS: usize = 5;
#[cfg(not(test))]
const MAKO_SUBSCRIPTION_RECONNECT_BASE_DELAY: Duration = Duration::from_millis(100);
#[cfg(test)]
const MAKO_SUBSCRIPTION_RECONNECT_BASE_DELAY: Duration = Duration::from_millis(5);
#[cfg(not(test))]
const MAKO_SUBSCRIPTION_RECONNECT_MAX_DELAY: Duration = Duration::from_secs(2);
#[cfg(test)]
const MAKO_SUBSCRIPTION_RECONNECT_MAX_DELAY: Duration = Duration::from_millis(25);
#[cfg(not(test))]
const MAKO_SUBSCRIPTION_STABLE_WINDOW: Duration = Duration::from_secs(30);
#[cfg(test)]
const MAKO_SUBSCRIPTION_STABLE_WINDOW: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const MAKO_SUBSCRIPTION_RECONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const MAKO_SUBSCRIPTION_RECONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(250);
type SseItem = std::result::Result<Event, Infallible>;
type MakoSse = Sse<ReceiverStream<SseItem>>;

pub struct MakoRuntimeManager {
    daemon: Option<MakoDaemonControl>,
    runtimes: RwLock<HashMap<String, ActiveMakoRuntime>>,
    event_streams: RwLock<HashMap<String, broadcast::Sender<AgenticEvent>>>,
    scheduled_wakes: RwLock<HashMap<String, JoinHandle<()>>>,
    wake_tx: mpsc::UnboundedSender<WakeCommand>,
    subscription_shutdown: broadcast::Sender<()>,
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
    /// Canonical independently supervised daemon counts.
    pub active_controller_count: usize,
    pub active_run_count: usize,
    pub queued_run_count: usize,
    pub recovery_required_run_count: usize,
    /// Compatibility projections used by the current diagnostics response.
    pub active_runtime_count: usize,
    pub scheduled_wake_count: usize,
    pub event_stream_count: usize,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoDispatchResult {
    pub session_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakoSteerStatus {
    Accepted,
    Queued,
}

impl MakoSteerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Queued => "queued",
        }
    }
}

pub fn control_plane_app_error(error: anyhow::Error) -> AppError {
    let daemon_error = error
        .chain()
        .find_map(|source| source.downcast_ref::<MakoDaemonError>());
    match daemon_error {
        Some(MakoDaemonError::Remote { code, message }) => match code.as_str() {
            "not_found" | "session_not_found" | "ownership_denied" | "ownership_mismatch"
            | "forbidden" => AppError::NotFound(message.clone()),
            "conflict"
            | "inactive"
            | "no_active_session"
            | "no_pending_interaction"
            | "already_running"
            | "idempotency_conflict"
            | "revision_conflict"
            | "state_conflict" => AppError::Conflict(message.clone()),
            "request_in_progress" => AppError::ServiceUnavailable(message.clone()),
            "invalid_request" | "invalid_command" | "bad_request" => {
                AppError::BadRequest(message.clone())
            }
            _ => AppError::BadGateway(format!("Mako daemon rejected the request: {message}")),
        },
        Some(MakoDaemonError::Unavailable(message)) => {
            AppError::BadGateway(format!("Mako daemon unavailable: {message}"))
        }
        None => AppError::BadGateway(format!("Mako daemon request failed: {error}")),
    }
}

impl MakoRuntimeManager {
    pub fn new() -> Arc<Self> {
        Self::build(None, true)
    }

    /// Construct the compatibility manager needed by the standalone
    /// execution host without spawning the legacy in-process wake consumer.
    /// Durable sleeps and retries are exclusively owned by `krusty-mako`.
    pub(crate) fn execution_host() -> Arc<Self> {
        Self::build(None, false)
    }

    pub async fn daemon_from_discovered() -> Result<Arc<Self>> {
        let daemon = MakoDaemonControl::connect_discovered().await?;
        Ok(Self::build(Some(daemon), false))
    }

    fn build(daemon: Option<MakoDaemonControl>, enable_embedded_wakes: bool) -> Arc<Self> {
        let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
        let (subscription_shutdown, _subscription_shutdown_rx) = broadcast::channel(1);
        let manager = Arc::new(Self {
            daemon,
            runtimes: RwLock::new(HashMap::new()),
            event_streams: RwLock::new(HashMap::new()),
            scheduled_wakes: RwLock::new(HashMap::new()),
            wake_tx,
            subscription_shutdown,
            started_at: Instant::now(),
        });

        if enable_embedded_wakes {
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
        } else {
            drop(wake_rx);
        }

        manager
    }

    pub fn is_daemon_backed(&self) -> bool {
        self.daemon.is_some()
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
        if let Some(daemon) = &self.daemon {
            daemon.recover(None, None, None).await?;
            return Ok(());
        }

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

    pub async fn recover_persisted_state_for_user(
        &self,
        state: AppState,
        runtime_state: &krusty_core::storage::MakoRuntimeState,
        wake_reason: &'static str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            daemon
                .recover(user_id, Some(&runtime_state.session_id), idempotency_key)
                .await?;
            return Ok(());
        }
        self.recover_persisted_state(state, runtime_state, wake_reason)
            .await
    }

    pub async fn recover_all_for_user(
        &self,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<usize> {
        self.daemon
            .as_ref()
            .context("Mako recovery requires the daemon control plane")?
            .recover(user_id, None, idempotency_key)
            .await
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
            active_controller_count: active_runtime_count.saturating_add(scheduled_wake_count),
            active_run_count: active_runtime_count,
            queued_run_count: scheduled_wake_count,
            recovery_required_run_count: 0,
            active_runtime_count,
            scheduled_wake_count,
            event_stream_count,
            uptime_secs: self.started_at.elapsed().as_secs(),
        }
    }

    pub async fn stats_for_sessions_for_user(
        &self,
        session_ids: &[String],
        user_id: Option<&str>,
    ) -> Result<MakoRuntimeStats> {
        if let Some(daemon) = &self.daemon {
            return daemon.stats(user_id).await;
        }
        Ok(self.stats_for_sessions(session_ids).await)
    }

    pub async fn start_or_restart_session_for_user(
        &self,
        state: AppState,
        session_id: String,
        wake_reason: &str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon
                .start(user_id, &session_id, wake_reason, idempotency_key)
                .await;
        }
        self.start_or_restart_session(state, session_id, wake_reason)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch_for_user(
        &self,
        user_id: Option<&str>,
        task: &str,
        working_dir: &str,
        project_dir: Option<&str>,
        model: Option<&str>,
        model_key: Option<&krusty_mako_protocol::ModelKey>,
        model_catalog_revision: Option<&str>,
        start_at: Option<chrono::DateTime<chrono::Utc>>,
        priority: MakoRunPriority,
        crew_slug: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<MakoDispatchResult> {
        let daemon = self
            .daemon
            .as_ref()
            .context("Mako dispatch requires the daemon control plane")?;
        let (session_id, status) = daemon
            .dispatch(
                user_id,
                task,
                working_dir,
                project_dir,
                model,
                model_key,
                model_catalog_revision,
                start_at.map(|value| value.timestamp_millis()),
                Some(priority.as_str()),
                crew_slug,
                idempotency_key,
            )
            .await?;
        Ok(MakoDispatchResult { session_id, status })
    }

    pub async fn create_schedule_for_user(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        definition: krusty_mako_protocol::ScheduleDefinition,
        idempotency_key: Option<&str>,
    ) -> Result<krusty_mako_protocol::ScheduleResponse> {
        self.daemon
            .as_ref()
            .context("Mako schedule mutations require the daemon control plane")?
            .create_schedule(user_id, session_id, definition, idempotency_key)
            .await
    }

    pub async fn replace_schedule_for_user(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        schedule_id: &str,
        expected_revision: u64,
        definition: krusty_mako_protocol::ScheduleDefinition,
        idempotency_key: Option<&str>,
    ) -> Result<krusty_mako_protocol::ScheduleResponse> {
        self.daemon
            .as_ref()
            .context("Mako schedule mutations require the daemon control plane")?
            .replace_schedule(
                user_id,
                session_id,
                schedule_id,
                expected_revision,
                definition,
                idempotency_key,
            )
            .await
    }

    pub async fn set_schedule_status_for_user(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        schedule_id: &str,
        expected_revision: u64,
        status: &str,
        idempotency_key: Option<&str>,
    ) -> Result<krusty_mako_protocol::ScheduleResponse> {
        self.daemon
            .as_ref()
            .context("Mako schedule mutations require the daemon control plane")?
            .set_schedule_status(
                user_id,
                session_id,
                schedule_id,
                expected_revision,
                status,
                idempotency_key,
            )
            .await
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

    pub async fn subscribe_for_user(
        &self,
        session_id: &str,
        user_id: Option<&str>,
    ) -> Result<broadcast::Receiver<AgenticEvent>> {
        self.subscribe_for_user_from(session_id, user_id, None, Some(0))
            .await
    }

    pub async fn subscribe_for_user_from(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        after_sequence: Option<i64>,
        replay_limit: Option<usize>,
    ) -> Result<broadcast::Receiver<AgenticEvent>> {
        let Some(daemon) = &self.daemon else {
            return Ok(self.subscribe(session_id).await);
        };

        // Every request opens its own authenticated daemon subscription. Sharing
        // a bridge by session ID would let a later caller inherit the first
        // caller's ownership check and would also destroy per-client replay
        // cursor semantics.
        let mut subscription = daemon
            .subscribe(user_id, session_id, after_sequence, replay_limit)
            .await?;
        let (event_sender, receiver) = broadcast::channel(MAKO_EVENT_BUFFER);
        let daemon = daemon.clone();
        let session_id_owned = session_id.to_string();
        let user_id_owned = user_id.map(ToOwned::to_owned);
        let mut shutdown = self.subscription_shutdown.subscribe();
        tokio::spawn(async move {
            let mut last_sequence = after_sequence.filter(|sequence| *sequence >= 0);
            if replay_limit == Some(0) {
                // A live-only subscription is established atomically at this
                // high-water mark. Keeping it as the cursor lets a reconnect
                // replay anything committed after that point instead of
                // silently losing the outage window.
                last_sequence =
                    max_sequence(last_sequence, subscription.accepted.high_water_sequence);
            }
            // Recovery is an internal catch-up operation, not a repeat of a
            // caller's initial sampling preference (including live-only or a
            // tiny replay page). Bound it to the bridge capacity so an outage
            // can be replayed without creating an immediate local lag burst.
            let reconnect_replay_limit = MAKO_EVENT_BUFFER;
            let mut reconnect_attempts = 0usize;
            let mut subscription_connected_at = Instant::now();

            loop {
                let received = tokio::select! {
                    _ = event_sender.closed() => break,
                    _ = shutdown.recv() => break,
                    received = subscription.next_event() => received,
                };
                match received {
                    Ok(Some(event)) => {
                        let session_matches =
                            event.session_id.as_deref() == Some(session_id_owned.as_str());
                        let global_shutdown = event.session_id.is_none()
                            && matches!(
                                &event.event,
                                krusty_mako_protocol::MakoEvent::DaemonShuttingDown { .. }
                            );
                        if !session_matches && !global_shutdown {
                            tracing::warn!(
                                session_id = %session_id_owned,
                                "Mako daemon event subscription returned an unexpected session"
                            );
                        } else {
                            let sequence = event.sequence;
                            if sequence
                                .zip(last_sequence)
                                .is_some_and(|(sequence, cursor)| sequence <= cursor)
                            {
                                continue;
                            }
                            if event_sender.send(map_daemon_event(event)).is_err() {
                                break;
                            }
                            last_sequence = max_sequence(last_sequence, sequence);
                            // Any successfully forwarded event proves that the
                            // replacement stream is healthy. A later outage gets
                            // its own bounded retry budget.
                            reconnect_attempts = 0;
                            continue;
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(
                            session_id = %session_id_owned,
                            "Mako daemon event subscription closed unexpectedly"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session_id_owned,
                            error_kind = daemon_subscription_stream_error_kind(&error),
                            "Mako daemon event subscription failed"
                        );
                    }
                }

                // An idle stream that stayed connected for a meaningful
                // interval was healthy even if it carried no events. Do not
                // accumulate retry debt across unrelated, widely separated
                // daemon restarts; only flapping connections exhaust the
                // consecutive retry budget.
                if subscription_connected_at.elapsed() >= MAKO_SUBSCRIPTION_STABLE_WINDOW {
                    reconnect_attempts = 0;
                }

                loop {
                    if reconnect_attempts >= MAKO_SUBSCRIPTION_RECONNECT_ATTEMPTS {
                        let _ = event_sender.send(AgenticEvent::Error {
                            error: "Mako daemon event stream unavailable after repeated reconnect attempts; reconnect to continue"
                                .to_string(),
                        });
                        tracing::error!(
                            session_id = %session_id_owned,
                            attempts = reconnect_attempts,
                            "Mako daemon event subscription reconnect budget exhausted"
                        );
                        return;
                    }

                    reconnect_attempts += 1;
                    let delay =
                        daemon_subscription_reconnect_delay(&session_id_owned, reconnect_attempts);
                    tokio::select! {
                        _ = event_sender.closed() => return,
                        _ = shutdown.recv() => return,
                        _ = tokio::time::sleep(delay) => {}
                    }

                    let reconnect = tokio::select! {
                        _ = event_sender.closed() => return,
                        _ = shutdown.recv() => return,
                        reconnect = tokio::time::timeout(
                            MAKO_SUBSCRIPTION_RECONNECT_ATTEMPT_TIMEOUT,
                            daemon.subscribe(
                                user_id_owned.as_deref(),
                                &session_id_owned,
                                last_sequence,
                                Some(reconnect_replay_limit),
                            ),
                        ) => reconnect,
                    };
                    match reconnect {
                        Ok(Ok(reconnected)) => {
                            subscription = reconnected;
                            subscription_connected_at = Instant::now();
                            break;
                        }
                        Ok(Err(error)) => {
                            tracing::warn!(
                                session_id = %session_id_owned,
                                attempt = reconnect_attempts,
                                error_kind = daemon_subscription_reconnect_error_kind(&error),
                                "Mako daemon event subscription reconnect failed"
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                session_id = %session_id_owned,
                                attempt = reconnect_attempts,
                                "Mako daemon event subscription reconnect timed out"
                            );
                        }
                    }
                }
            }
        });
        Ok(receiver)
    }

    pub async fn begin_daemon_chat_turn_for_user(
        &self,
        session_id: &str,
        message: &str,
        user_id: Option<&str>,
        is_first_message: bool,
        idempotency_key: Option<&str>,
    ) -> Result<broadcast::Receiver<AgenticEvent>> {
        let daemon = self
            .daemon
            .as_ref()
            .context("Mako chat requires the daemon control plane")?;
        if is_first_message {
            // Send creates the controller for legacy/new-chat sessions. Start
            // then queues the first run, and sequence-zero replay closes the
            // race between those durable mutations and subscription.
            daemon
                .send_message(user_id, session_id, message, idempotency_key)
                .await?;
            daemon
                .start(user_id, session_id, "chat_first_message", idempotency_key)
                .await?;
            return self
                .subscribe_for_user_from(session_id, user_id, Some(0), Some(256))
                .await;
        }

        // Existing interactive controllers can subscribe live before the
        // message so no response event races past the stream.
        let receiver = self.subscribe_for_user(session_id, user_id).await?;
        daemon
            .send_message(user_id, session_id, message, idempotency_key)
            .await?;
        Ok(receiver)
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

    pub async fn pause_session_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon.pause(user_id, session_id, idempotency_key).await;
        }
        self.pause_session(state, session_id).await
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

    pub async fn schedule_session_for_user(
        &self,
        state: &AppState,
        session_id: String,
        wake_at: chrono::DateTime<chrono::Utc>,
        wake_reason: &'static str,
        sleep_reason: &'static str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon
                .schedule(
                    user_id,
                    &session_id,
                    wake_at.timestamp_millis(),
                    wake_reason,
                    idempotency_key,
                )
                .await;
        }
        self.schedule_session(state, session_id, wake_at, wake_reason, sleep_reason)
            .await
    }

    pub async fn resume_session_for_user(
        &self,
        state: AppState,
        session_id: String,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon.resume(user_id, &session_id, idempotency_key).await;
        }
        self.start_or_restart_session(state, session_id, "resume")
            .await
    }

    pub async fn cancel_session_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon.cancel(user_id, session_id, idempotency_key).await;
        }
        self.stop_active_run(state, session_id).await;
        Ok(())
    }

    pub async fn delete_session_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            daemon.delete(user_id, session_id, idempotency_key).await?;
            self.forget_session(session_id).await;
            state.session_locks.write().await.remove(session_id);
            return Ok(());
        }

        self.stop_active_run(state, session_id).await;
        self.forget_session(session_id).await;
        SessionManager::new(Database::new(&state.db_path)?).delete_session(session_id)?;
        state.session_locks.write().await.remove(session_id);
        Ok(())
    }

    pub async fn send_message_for_user(
        &self,
        state: AppState,
        session_id: String,
        message: &str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon
                .send_message(user_id, &session_id, message, idempotency_key)
                .await;
        }

        let session_manager = SessionManager::new(Database::new(&state.db_path)?);
        let content_json = serde_json::json!([{ "type": "text", "text": message }]).to_string();
        session_manager.save_message(&session_id, "user", &content_json)?;
        self.start_or_restart_session(state, session_id, "user_message")
            .await
    }

    pub async fn set_priority_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        priority: MakoRunPriority,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon
                .set_priority(user_id, session_id, priority.as_str(), idempotency_key)
                .await;
        }
        MakoRuntimeStateStore::new(Database::new(&state.db_path)?)
            .set_priority(session_id, priority)?;
        Ok(())
    }

    pub async fn set_crew_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        crew_slug: Option<&str>,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon
                .set_crew(user_id, session_id, crew_slug, idempotency_key)
                .await;
        }
        MakoRuntimeStateStore::new(Database::new(&state.db_path)?)
            .set_crew_slug(session_id, crew_slug)?;
        Ok(())
    }

    pub async fn steer_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        pending_id: &str,
        content: Vec<krusty_core::ai::types::Content>,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<MakoSteerStatus> {
        if let Some(daemon) = &self.daemon {
            let content = serde_json::to_value(content)?;
            let acknowledgement = daemon
                .steer(user_id, session_id, pending_id, content, idempotency_key)
                .await?;
            if acknowledgement.accepted {
                return Ok(MakoSteerStatus::Accepted);
            }
            if acknowledgement.message.as_deref() == Some("queued") {
                return Ok(MakoSteerStatus::Queued);
            }
            return Err(MakoDaemonError::Remote {
                code: "conflict".to_string(),
                message: format!(
                    "Mako daemon declined steering: {}",
                    acknowledgement
                        .message
                        .unwrap_or_else(|| "no reason provided".to_string())
                ),
            }
            .into());
        }

        let content_json = serde_json::to_string(&content)?;
        SessionManager::new(Database::new(&state.db_path)?).queue_pending_steering(
            session_id,
            pending_id,
            &content_json,
        )?;
        let sender = state.session_inputs.read().await.get(session_id).cloned();
        let Some(sender) = sender else {
            return Ok(MakoSteerStatus::Queued);
        };
        let input = LoopInput::Steer {
            pending_id: Some(pending_id.to_string()),
            content,
        };
        Ok(if sender.send(input).is_ok() {
            MakoSteerStatus::Accepted
        } else {
            MakoSteerStatus::Queued
        })
    }

    pub async fn tool_approval_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        run_id: &str,
        tool_call_id: &str,
        approved: bool,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        if let Some(daemon) = &self.daemon {
            return daemon
                .tool_approval(
                    user_id,
                    session_id,
                    run_id,
                    tool_call_id,
                    approved,
                    idempotency_key,
                )
                .await;
        }
        let sender = state
            .session_inputs
            .read()
            .await
            .get(session_id)
            .cloned()
            .context("No active Mako session")?;
        sender
            .send(LoopInput::ToolApproval {
                tool_call_id: tool_call_id.to_string(),
                approved,
            })
            .context("Mako session is no longer accepting tool approvals")
    }

    pub async fn user_response_and_subscribe_for_user(
        &self,
        state: &AppState,
        session_id: &str,
        run_id: &str,
        tool_call_id: &str,
        response: &str,
        user_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<broadcast::Receiver<AgenticEvent>> {
        let receiver = self.subscribe_for_user(session_id, user_id).await?;
        if let Some(daemon) = &self.daemon {
            daemon
                .user_response(
                    user_id,
                    session_id,
                    run_id,
                    tool_call_id,
                    response,
                    idempotency_key,
                )
                .await?;
            return Ok(receiver);
        }
        let sender = state
            .session_inputs
            .read()
            .await
            .get(session_id)
            .cloned()
            .context("No active Mako session")?;
        sender
            .send(LoopInput::UserResponse {
                tool_call_id: tool_call_id.to_string(),
                response: response.to_string(),
            })
            .context("Mako session is no longer accepting user responses")?;
        Ok(receiver)
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

impl Drop for MakoRuntimeManager {
    fn drop(&mut self) {
        // Subscription bridges otherwise retain their daemon client after the
        // manager has gone away. Waking them here closes the authenticated IPC
        // streams promptly during shutdown and in short-lived server hosts.
        let _ = self.subscription_shutdown.send(());
    }
}

fn max_sequence(current: Option<i64>, candidate: Option<i64>) -> Option<i64> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.max(candidate)),
        (Some(current), None) => Some(current),
        (None, Some(candidate)) => Some(candidate),
        (None, None) => None,
    }
}

fn daemon_subscription_reconnect_delay(session_id: &str, attempt: usize) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    let multiplier = 2u32.checked_pow(exponent).unwrap_or(u32::MAX);
    let base = MAKO_SUBSCRIPTION_RECONNECT_BASE_DELAY
        .checked_mul(multiplier)
        .unwrap_or(MAKO_SUBSCRIPTION_RECONNECT_MAX_DELAY)
        .min(MAKO_SUBSCRIPTION_RECONNECT_MAX_DELAY);
    let jitter_room = MAKO_SUBSCRIPTION_RECONNECT_MAX_DELAY.saturating_sub(base);
    if jitter_room.is_zero() {
        return base;
    }

    // Stable per-session jitter avoids reconnect herds while keeping tests and
    // incident timing reproducible. It is deliberately bounded to 25% of the
    // exponential delay and by the absolute retry ceiling.
    let jitter_cap = (base / 4).min(jitter_room);
    if jitter_cap.is_zero() {
        return base;
    }
    let seed = session_id.bytes().fold(attempt as u64, |seed, byte| {
        seed.wrapping_mul(1_099_511_628_211)
            .wrapping_add(u64::from(byte))
    });
    let jitter_millis = seed % (jitter_cap.as_millis() as u64 + 1);
    base + Duration::from_millis(jitter_millis)
}

fn daemon_subscription_stream_error_kind(error: &ClientError) -> &'static str {
    match error {
        ClientError::Remote { .. } => "remote",
        ClientError::Auth(_) | ClientError::Peer(_) => "authentication",
        ClientError::Protocol(_) | ClientError::Frame(_) => "protocol",
        ClientError::ConnectTimeout | ClientError::RequestTimeout => "timeout",
        ClientError::Closed => "closed",
        ClientError::UnsupportedPlatform => "unsupported",
        ClientError::Io(_) => "io",
    }
}

fn daemon_subscription_reconnect_error_kind(error: &anyhow::Error) -> &'static str {
    match error
        .chain()
        .find_map(|source| source.downcast_ref::<MakoDaemonError>())
    {
        Some(MakoDaemonError::Remote { .. }) => "remote",
        Some(MakoDaemonError::Unavailable(_)) => "unavailable",
        None => "unexpected",
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
        get_current_snapshot, refresh_current_snapshot, Database, MakoRuntimeStateStatus,
        MakoRuntimeStateStore, MemoryStore, MemoryType, ReportStore, SessionType, WorkspaceMode,
    };
    use krusty_core::tools::registry::ToolRegistry;
    use krusty_core::SessionManager;

    use super::{
        apply_runtime_event_state, control_plane_app_error, mako_notification_title,
        persist_runtime_state, refresh_snapshot_after_run, resolve_persisted_project_dir,
        with_registered_session_input, ActiveMakoRuntime, MakoDaemonError, MakoRuntimeManager,
    };
    use crate::error::AppError;
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
    fn daemon_protocol_codes_map_to_stable_http_classes() {
        let mapped = |code: &str| {
            control_plane_app_error(
                MakoDaemonError::Remote {
                    code: code.to_string(),
                    message: "test".to_string(),
                }
                .into(),
            )
        };

        assert!(matches!(mapped("ownership_denied"), AppError::NotFound(_)));
        assert!(matches!(mapped("invalid_command"), AppError::BadRequest(_)));
        assert!(matches!(
            mapped("idempotency_conflict"),
            AppError::Conflict(_)
        ));
        assert!(matches!(
            mapped("request_in_progress"),
            AppError::ServiceUnavailable(_)
        ));
        assert!(matches!(mapped("revision_conflict"), AppError::Conflict(_)));
        assert!(matches!(mapped("internal_error"), AppError::BadGateway(_)));
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

        let memory_store =
            MemoryStore::new(Database::new(&state.db_path).expect("database should open"));
        memory_store
            .save(
                MemoryType::Project,
                "Initial context",
                "Stale snapshot source",
                Some(project_dir.to_string_lossy().as_ref()),
                Some("alice"),
            )
            .expect("initial context should persist");
        let snapshot = refresh_current_snapshot(
            &state.db_path,
            Some(project_dir.to_string_lossy().as_ref()),
            Some("alice"),
        )
        .expect("initial snapshot should refresh")
        .expect("initial snapshot should exist");
        Database::new(&state.db_path)
            .expect("database should open")
            .conn()
            .execute(
                "UPDATE knowledge_snapshots SET updated_at = ?1 WHERE id = ?2",
                ("2025-01-01T00:00:00Z", snapshot.id.as_str()),
            )
            .expect("snapshot should backdate");

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

        refresh_snapshot_after_run(
            &state.db_path,
            Some(project_dir.to_string_lossy().as_ref()),
            Some("alice"),
            Some(&LoopStopReason::Completed),
        );

        let refreshed = get_current_snapshot(
            &state.db_path,
            Some(project_dir.to_string_lossy().as_ref()),
            Some("alice"),
        )
        .expect("snapshot should load")
        .expect("snapshot should still exist");

        assert_ne!(refreshed.updated_at, "2025-01-01T00:00:00Z");
        assert!(refreshed.content.contains("Fresh findings"));
    }
}
