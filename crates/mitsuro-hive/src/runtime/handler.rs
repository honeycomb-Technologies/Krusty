use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use mitsuro_core::ai::models::ModelKey as CoreModelKey;
use mitsuro_core::hive::{
    canonical_timestamp, parse_timezone, DstPolicy, MisfireConfig, RecurrenceV1, RetryPolicy,
};
use mitsuro_core::storage::{hash_request_bytes, is_valid_crew_slug, OverlapPolicy};
use mitsuro_core::Content;
use mitsuro_hive_protocol::{
    unix_time_millis, AckResponse, Actor, Command, CreateScheduleCommand, DaemonRuntimeStats,
    DispatchCommand, DispatchResponse, EventEnvelope, ExtensionResponse, HiveEvent, LaggedEvent,
    ModelKey, ProtocolErrorPayload, ProtocolVersion, RecoverResponse, ReplaceScheduleCommand,
    ReplayGapEvent, ResponsePayload, ScheduleDefinition, ScheduleResponse, SessionResponse,
    SetScheduleStatusCommand, SubscribeCommand, SubscriptionAccepted,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::{CommandContext, CommandHandler, HandlerReply, HandlerResult};

use super::backend::{ExecutionBackend, ExecutionControl};
use super::config::HiveRuntimeConfig;
use super::events::EventHub;
use super::persistence::{
    append_event, get_or_create_controller, request_hash, require_owned_session,
    unix_millis_to_utc, ControllerRecord, Mutation, MutationOutcome, PersistedEvent,
    RuntimePersistence, RuntimeStoreError,
};
use super::pump;

pub(crate) const DAEMON_LEASE_NAME: &str = "hive-scheduler";
pub(crate) const MAX_RETRY_ATTEMPTS: u32 = 100;
pub(crate) const MAX_RETRY_DELAY_SECS: u64 = 7 * 24 * 60 * 60;

const PUMP_STARTING: u8 = 0;
const PUMP_RUNNING: u8 = 1;
const PUMP_STOPPED: u8 = 2;
const CANCELLATION_SIGNAL_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct CommittedCancellation {
    pub(crate) session_id: String,
}

#[derive(Debug)]
pub(crate) struct RuntimeHealth {
    pump_state: AtomicU8,
    scheduler_activated: AtomicBool,
    pump_stopped: Notify,
}

impl RuntimeHealth {
    fn new() -> Self {
        Self {
            pump_state: AtomicU8::new(PUMP_STARTING),
            scheduler_activated: AtomicBool::new(false),
            pump_stopped: Notify::new(),
        }
    }

    pub(crate) fn mark_pump_running(&self) {
        self.pump_state.store(PUMP_RUNNING, Ordering::Release);
    }

    pub(crate) fn mark_pump_stopped(&self) {
        self.scheduler_activated.store(false, Ordering::Release);
        self.pump_state.store(PUMP_STOPPED, Ordering::Release);
        self.pump_stopped.notify_waiters();
    }

    pub(crate) fn set_scheduler_activated(&self, activated: bool) {
        self.scheduler_activated.store(activated, Ordering::Release);
    }

    fn pump_alive(&self) -> bool {
        self.pump_state.load(Ordering::Acquire) == PUMP_RUNNING
    }

    fn scheduler_activated(&self) -> bool {
        self.scheduler_activated.load(Ordering::Acquire)
    }

    async fn wait_for_pump_stop(&self) {
        loop {
            let stopped = self.pump_stopped.notified();
            if self.pump_state.load(Ordering::Acquire) == PUMP_STOPPED {
                return;
            }
            stopped.await;
        }
    }
}

pub(crate) struct RuntimeShared {
    pub(crate) config: HiveRuntimeConfig,
    pub(crate) instance_id: String,
    pub(crate) persistence: RuntimePersistence,
    pub(crate) backend: Arc<dyn ExecutionBackend>,
    pub(crate) events: EventHub,
    pub(crate) cancellation_tx: broadcast::Sender<CommittedCancellation>,
    pub(crate) mutation_gate: Mutex<()>,
    pub(crate) control_gate: Mutex<()>,
    pub(crate) health: RuntimeHealth,
}

pub struct DurableHiveCommandHandler {
    pub(crate) shared: Arc<RuntimeShared>,
}

pub struct HiveRuntimeHandle {
    handler: Arc<DurableHiveCommandHandler>,
    shutdown_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl HiveRuntimeHandle {
    pub fn handler(&self) -> Arc<DurableHiveCommandHandler> {
        Arc::clone(&self.handler)
    }

    pub async fn shutdown(mut self) {
        self.shutdown_tx.send_replace(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    /// Wait for an unrequested scheduler-pump exit. The task is not removed
    /// until its liveness guard has reported a stop, so cancelling this future
    /// (for example because the IPC server shut down first) leaves graceful
    /// shutdown ownership intact.
    pub async fn wait_for_scheduler_failure(&mut self) -> anyhow::Error {
        self.handler.shared.health.wait_for_pump_stop().await;
        match self.task.take() {
            Some(task) => match task.await {
                Ok(()) => anyhow::anyhow!("Hive scheduler pump exited unexpectedly"),
                Err(error) if error.is_panic() => {
                    anyhow::anyhow!("Hive scheduler pump panicked: {error}")
                }
                Err(error) => anyhow::anyhow!("Hive scheduler pump stopped: {error}"),
            },
            None => anyhow::anyhow!("Hive scheduler pump stopped without a task handle"),
        }
    }

    #[cfg(test)]
    pub(crate) fn force_pump_exit_for_test(&self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Drop for HiveRuntimeHandle {
    fn drop(&mut self) {
        self.shutdown_tx.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub async fn start_runtime(
    config: HiveRuntimeConfig,
    daemon_instance_id: impl Into<String>,
    backend: Arc<dyn ExecutionBackend>,
) -> anyhow::Result<HiveRuntimeHandle> {
    config.validate()?;
    let persistence = RuntimePersistence::new(config.database_path.clone(), config.idempotency_ttl);
    persistence
        .initialize()
        .await
        .map_err(|error| anyhow::anyhow!(error.protocol().message))?;
    let instance_label = daemon_instance_id.into();
    let (cancellation_tx, _) = broadcast::channel(CANCELLATION_SIGNAL_CAPACITY);
    let shared = Arc::new(RuntimeShared {
        instance_id: format!("{instance_label}:boot:{}", uuid::Uuid::new_v4()),
        events: EventHub::new(config.live_event_capacity),
        persistence,
        backend,
        mutation_gate: Mutex::new(()),
        control_gate: Mutex::new(()),
        cancellation_tx,
        health: RuntimeHealth::new(),
        config,
    });
    let handler = Arc::new(DurableHiveCommandHandler {
        shared: Arc::clone(&shared),
    });
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(pump::run(shared, shutdown_rx));
    Ok(HiveRuntimeHandle {
        handler,
        shutdown_tx,
        task: Some(task),
    })
}

#[async_trait]
impl CommandHandler for DurableHiveCommandHandler {
    async fn handle(&self, context: CommandContext, command: Command) -> HandlerResult {
        if let Command::Subscribe(subscribe) = command {
            return self.subscribe(context.actor, subscribe).await;
        }
        if matches!(
            command,
            Command::Ping | Command::Stats | Command::Shutdown(_)
        ) {
            return Err(ProtocolErrorPayload::new(
                "invalid_routing",
                "foundation command reached the runtime handler",
                false,
            ));
        }

        let operation = command.name();
        let hash = request_hash(&context.actor, &command);
        let actor = context.actor.clone();
        let idempotency_key = context.idempotency_key.clone();
        let control = control_after_commit(&command, &actor, &idempotency_key);
        let committed_cancellation = match &command {
            Command::CancelSession(command) => Some(CommittedCancellation {
                session_id: command.session_id.clone(),
            }),
            _ => None,
        };
        let _gate = self.shared.mutation_gate.lock().await;
        let mut outcome = self
            .mutate(actor, idempotency_key, operation, hash, command)
            .await
            .map_err(RuntimeStoreError::protocol)?;
        outcome.events.sort_by_key(|event| event.sequence);
        for event in &outcome.events {
            self.shared.events.publish(event.envelope());
        }
        // Reserve control-delivery order before releasing mutation order. A
        // later Resume/Start must not reach the backend before an earlier
        // Pause/Cancel, and steering must follow durable commit order.
        let control_guard = if !outcome.replayed && control.is_some() {
            Some(self.shared.control_gate.lock().await)
        } else {
            None
        };
        drop(_gate);

        if !outcome.replayed {
            // Wake every scheduler-owned execution only after the exact-owner
            // cancellation transaction has committed. Receivers subscribe
            // before entering the durable running boundary, so this closes
            // the mark-running/control-delivery race without treating a
            // best-effort host acknowledgement as the source of truth.
            if let Some(cancellation) = committed_cancellation {
                let _ = self.shared.cancellation_tx.send(cancellation);
            }
            if let Some((session_id, control)) = control {
                if let Err(error) = self.shared.backend.control(&session_id, control).await {
                    tracing::warn!(session_id, error = %error, "Hive backend control delivery failed after durable acceptance");
                }
            }
        }
        drop(control_guard);
        Ok(HandlerReply::Response(outcome.response))
    }

    async fn runtime_stats(&self, actor: &Actor) -> DaemonRuntimeStats {
        let pump_alive = self.shared.health.pump_alive();
        let mut stats = match self.shared.persistence.stats(actor).await {
            Ok(stats) => stats,
            Err(error) => {
                let failure = error.protocol();
                tracing::warn!(
                    error_code = %failure.code,
                    "Hive runtime stats are unavailable; reporting scheduler not ready"
                );
                return DaemonRuntimeStats {
                    pump_alive,
                    ..DaemonRuntimeStats::default()
                };
            }
        };
        let scheduler_ready = if pump_alive && self.shared.health.scheduler_activated() {
            self.shared
                .persistence
                .daemon_lease_is_current(DAEMON_LEASE_NAME, &self.shared.instance_id)
                .await
                .unwrap_or(false)
        } else {
            false
        };
        stats.pump_alive = pump_alive;
        stats.scheduler_ready = scheduler_ready;
        stats
    }
}

impl DurableHiveCommandHandler {
    async fn mutate(
        &self,
        actor: Actor,
        idempotency_key: String,
        operation: &'static str,
        hash: String,
        command: Command,
    ) -> Result<MutationOutcome, RuntimeStoreError> {
        let mutation_idempotency_key = idempotency_key.clone();
        self.shared
            .persistence
            .mutate(
                actor,
                idempotency_key,
                operation,
                hash,
                move |tx, actor, now| match command {
                    Command::Dispatch(command) => dispatch(tx, actor, now, command),
                    Command::CreateSchedule(command) => {
                        create_recurring_schedule(tx, actor, now, command)
                    }
                    Command::ReplaceSchedule(command) => {
                        replace_recurring_schedule(tx, actor, now, command)
                    }
                    Command::SetScheduleStatus(command) => {
                        set_schedule_status(tx, actor, now, command)
                    }
                    Command::StartSession(command) => {
                        start_session(tx, actor, now, &command.session_id)
                    }
                    Command::ScheduleSession(command) => schedule_session(
                        tx,
                        actor,
                        now,
                        &command.session_id,
                        command.wake_at_unix_ms,
                        &command.reason,
                    ),
                    Command::PauseSession(command) => {
                        set_controller_status(tx, actor, now, &command.session_id, "paused")
                    }
                    Command::ResumeSession(command) => {
                        set_controller_status(tx, actor, now, &command.session_id, "active")
                    }
                    Command::CancelSession(command) => {
                        cancel_session(tx, actor, now, &command.session_id)
                    }
                    Command::DeleteSession(command) => {
                        delete_session(tx, actor, now, &command.session_id)
                    }
                    Command::SendMessage(command) => {
                        let pending_id = pending_message_id(
                            actor,
                            &command.session_id,
                            &mutation_idempotency_key,
                        );
                        send_message(
                            tx,
                            actor,
                            now,
                            &command.session_id,
                            &command.message,
                            &pending_id,
                        )
                    }
                    Command::GroupMessage(command) => super::groups::group_message(
                        tx,
                        actor,
                        now,
                        command,
                        &mutation_idempotency_key,
                    ),
                    Command::GroupStop(command) => {
                        super::groups::group_stop(tx, actor, now, &command.group_id)
                    }
                    Command::Steer(command) => {
                        let pending_id = steer_pending_id(
                            actor,
                            &command.session_id,
                            &mutation_idempotency_key,
                            command.pending_id.as_deref(),
                        );
                        stage_steer(
                            tx,
                            actor,
                            now,
                            &command.session_id,
                            &pending_id,
                            command.content,
                        )
                    }
                    Command::ToolApproval(command) => stage_tool_approval(
                        tx,
                        actor,
                        now,
                        &command.session_id,
                        &command.run_id,
                        &command.tool_call_id,
                        command.approved,
                    ),
                    Command::UserResponse(command) => {
                        let pending_id = pending_message_id(
                            actor,
                            &command.session_id,
                            &mutation_idempotency_key,
                        );
                        user_response(
                            tx,
                            actor,
                            now,
                            &command.session_id,
                            &command.run_id,
                            &command.tool_call_id,
                            &command.response,
                            &pending_id,
                        )
                    }
                    Command::SetPriority(command) => {
                        set_priority(tx, actor, now, &command.session_id, &command.priority)
                    }
                    Command::SetCrew(command) => set_crew(
                        tx,
                        actor,
                        now,
                        &command.session_id,
                        command.crew_slug.as_deref(),
                    ),
                    Command::Recover(command) => {
                        recover(tx, actor, now, command.session_id.as_deref())
                    }
                    Command::Extension(command) => extension(tx, actor, now, command),
                    Command::Ping
                    | Command::Stats
                    | Command::Shutdown(_)
                    | Command::Subscribe(_) => Err(RuntimeStoreError::Invalid(
                        "command is not a mutable runtime operation".into(),
                    )),
                },
            )
            .await
    }

    async fn subscribe(&self, actor: Actor, command: SubscribeCommand) -> HandlerResult {
        let mut live = self.shared.events.subscribe(&command.session_id);
        let requested_after = command.after_sequence.unwrap_or(0).max(0);
        let live_only = command.replay_limit == Some(0);
        let limit = if live_only {
            0
        } else {
            command
                .replay_limit
                .unwrap_or(self.shared.config.replay_limit)
                .clamp(1, self.shared.config.replay_limit)
        };
        let catchup_limit = self.shared.config.replay_limit;
        let snapshot = self
            .shared
            .persistence
            .replay(
                actor.clone(),
                command.session_id.clone(),
                requested_after,
                limit,
            )
            .await
            .map_err(RuntimeStoreError::protocol)?;
        let accepted = SubscriptionAccepted {
            session_id: command.session_id.clone(),
            high_water_sequence: snapshot.high_water,
        };
        let (sender, receiver) = mpsc::channel(self.shared.config.subscriber_capacity);
        let persistence = self.shared.persistence.clone();
        let session_id = command.session_id;
        tokio::spawn(async move {
            if let Some(earliest) = (!live_only)
                .then_some(snapshot.earliest_returned.or(snapshot.earliest_available))
                .flatten()
                .filter(|earliest| *earliest > snapshot.requested_after.saturating_add(1))
            {
                if sender
                    .send(replay_gap_event(
                        &session_id,
                        snapshot.requested_after,
                        earliest,
                    ))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let mut previous_replay_sequence = None::<i64>;
            for event in snapshot.events {
                if previous_replay_sequence
                    .is_some_and(|previous| event.sequence > previous.saturating_add(1))
                {
                    let previous = previous_replay_sequence.unwrap_or(snapshot.requested_after);
                    if sender
                        .send(replay_gap_event(&session_id, previous, event.sequence))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                previous_replay_sequence = Some(event.sequence);
                if sender.send(event.envelope()).await.is_err() {
                    return;
                }
            }
            let mut last_sequence = snapshot.high_water.unwrap_or(requested_after);
            loop {
                let received = tokio::select! {
                    _ = sender.closed() => return,
                    received = live.recv() => received,
                };
                match received {
                    Ok(event) => {
                        let Some(sequence) = event.sequence else {
                            if sender.send(event).await.is_err() {
                                return;
                            }
                            continue;
                        };
                        if sequence <= last_sequence {
                            continue;
                        }
                        if sequence > last_sequence.saturating_add(1) {
                            let skipped = sequence.saturating_sub(last_sequence + 1) as u64;
                            if sender
                                .send(lagged_event(&session_id, skipped, Some(last_sequence)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        last_sequence = sequence;
                        if sender.send(event).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        if sender
                            .send(lagged_event(&session_id, skipped, Some(last_sequence)))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        match persistence
                            .replay(
                                actor.clone(),
                                session_id.clone(),
                                last_sequence,
                                catchup_limit,
                            )
                            .await
                        {
                            Ok(catchup) => {
                                for event in catchup.events {
                                    if event.sequence > last_sequence.saturating_add(1)
                                        && sender
                                            .send(replay_gap_event(
                                                &session_id,
                                                last_sequence,
                                                event.sequence,
                                            ))
                                            .await
                                            .is_err()
                                    {
                                        return;
                                    }
                                    last_sequence = last_sequence.max(event.sequence);
                                    if sender.send(event.envelope()).await.is_err() {
                                        return;
                                    }
                                }
                                last_sequence = catchup.high_water.unwrap_or(last_sequence);
                            }
                            Err(error) => {
                                tracing::warn!(error = ?error, "Hive subscription catch-up failed");
                                return;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        Ok(HandlerReply::Subscription {
            accepted,
            events: receiver,
        })
    }
}

fn dispatch(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    command: DispatchCommand,
) -> Result<Mutation, RuntimeStoreError> {
    if command.task.trim().is_empty() || command.working_dir.trim().is_empty() {
        return Err(RuntimeStoreError::Invalid(
            "dispatch requires a task and working directory".into(),
        ));
    }
    let (model, model_key, model_catalog_revision) = normalize_model_identity(
        command.model.clone(),
        command.model_key.clone(),
        command.model_catalog_revision.clone(),
        "dispatch",
    )?;
    let model = model.ok_or_else(|| {
        RuntimeStoreError::Invalid("dispatch requires a frozen model identity".into())
    })?;
    let model_key_json = model_key
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    if command.task.len() > 64 * 1024 {
        return Err(RuntimeStoreError::Invalid(
            "dispatch task exceeds 65536 bytes".into(),
        ));
    }
    if command.working_dir.len() > 4096 || command.working_dir.as_bytes().contains(&0) {
        return Err(RuntimeStoreError::Invalid(
            "dispatch working directory is invalid or too long".into(),
        ));
    }
    if !Path::new(&command.working_dir).is_absolute() {
        return Err(RuntimeStoreError::Invalid(
            "dispatch working directory must be absolute".into(),
        ));
    }
    if command
        .project_dir
        .as_deref()
        .is_some_and(|path| path.len() > 4096 || path.as_bytes().contains(&0))
    {
        return Err(RuntimeStoreError::Invalid(
            "dispatch project directory is invalid or too long".into(),
        ));
    }
    if command
        .project_dir
        .as_deref()
        .is_some_and(|path| !Path::new(path).is_absolute())
    {
        return Err(RuntimeStoreError::Invalid(
            "dispatch project directory must be absolute".into(),
        ));
    }
    if command
        .crew_slug
        .as_deref()
        .is_some_and(|slug| !is_valid_crew_slug(slug))
    {
        return Err(RuntimeStoreError::Invalid("crew slug is invalid".into()));
    }
    if let Some(user_id) = actor.user_id.as_deref() {
        let exists = tx
            .query_row("SELECT 1 FROM users WHERE id = ?1", [user_id], |_| Ok(()))
            .optional()?
            .is_some();
        if !exists {
            return Err(RuntimeStoreError::Ownership);
        }
    }
    let scheduled_for = command
        .start_at_unix_ms
        .map(unix_millis_to_utc)
        .transpose()?
        .unwrap_or_else(Utc::now);
    let session_id = uuid::Uuid::new_v4().to_string();
    let controller_id = uuid::Uuid::new_v4().to_string();
    let run_id = uuid::Uuid::new_v4().to_string();
    let project_dir = command
        .project_dir
        .clone()
        .unwrap_or_else(|| command.working_dir.clone());
    tx.execute(
        "INSERT INTO sessions (
            id, title, created_at, updated_at, model, model_key_json,
            model_catalog_revision, working_dir, project_dir, workspace_mode,
            session_type, user_id, permission_mode
         ) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, 'selected', 'hive', ?9, 'autonomous')",
        params![
            session_id,
            bounded_title(&command.task),
            now,
            model,
            model_key_json,
            model_catalog_revision,
            command.working_dir,
            project_dir,
            actor.user_id,
        ],
    )?;
    insert_canonical_user_message(tx, &session_id, &command.task, now)?;
    tx.execute(
        "INSERT INTO hive_controllers (
            id, scope_key, user_id, session_id, status, timezone,
            max_concurrent_runs, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'active', 'UTC', 1, ?5, ?5)",
        params![
            controller_id,
            format!("session:{session_id}"),
            actor.user_id,
            session_id,
            now
        ],
    )?;
    let controller = ControllerRecord {
        id: controller_id,
        session_id: session_id.clone(),
        status: "active".into(),
        timezone: "UTC".into(),
    };
    let priority = priority_value(command.priority.as_deref().unwrap_or("normal"))?;
    insert_run(
        tx,
        &run_id,
        &controller.id,
        Some(&session_id),
        None,
        None,
        "dispatch",
        &command.task,
        serde_json::json!({
            "working_dir": command.working_dir,
            "project_dir": project_dir,
            "model": model,
            "model_key": model_key,
            "model_catalog_revision": model_catalog_revision,
            "permission_mode": "autonomous",
            "crew_slug": command.crew_slug,
            "retry": RetryPolicy::default(),
        }),
        priority,
        &canonical_timestamp(scheduled_for),
        5,
        now,
    )?;
    tx.execute(
        "INSERT INTO hive_runtime_state (
            session_id, status, current_run_id, crew_slug, priority, updated_at
         ) VALUES (?1, 'idle', ?2, ?3, ?4, ?5)",
        params![
            session_id,
            run_id,
            command.crew_slug,
            priority_name(priority),
            now
        ],
    )?;
    let event = append_event(
        tx,
        &controller,
        "run_queued",
        Some(&run_id),
        None,
        Some(&format!("run:{run_id}:queued")),
        serde_json::json!({"run_id": run_id, "kind": "dispatch"}),
        now,
    )?;
    Ok(Mutation {
        response: ResponsePayload::Dispatch(DispatchResponse {
            session_id: session_id.clone(),
            status: "queued".into(),
        }),
        resource_id: Some(session_id),
        events: vec![event],
    })
}

fn validate_bounded_field(
    value: &str,
    field: &str,
    max_bytes: usize,
) -> Result<(), RuntimeStoreError> {
    if value.trim().is_empty() || value.len() > max_bytes || value.as_bytes().contains(&0) {
        return Err(RuntimeStoreError::Invalid(format!(
            "{field} is invalid or exceeds {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn start_session(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, session_id)?;
    freeze_session_model_into_open_runs(tx, &session, now)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    if let Some((run_id, attempt_count)) = recovery_required_run(tx, &controller.id)? {
        // Never create or wake sibling work while a prior attempt has
        // uncertain side effects. Cancellation is the explicit abandon path;
        // until then the controller remains fenced and the projection stays
        // visibly in error.
        tx.execute(
            "UPDATE hive_controllers SET status = 'paused', updated_at = ?2 WHERE id = ?1",
            params![controller.id, now],
        )?;
        tx.execute(
            "INSERT INTO hive_runtime_state (session_id, status, current_run_id, updated_at)
             VALUES (?1, 'error', ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET status = 'error',
                 current_run_id = excluded.current_run_id,
                 updated_at = excluded.updated_at",
            params![session_id, run_id, now],
        )?;
        let event = append_event(
            tx,
            &controller,
            "start_blocked_recovery_required",
            Some(&run_id),
            None,
            Some(&format!("recovery_block:{run_id}:{attempt_count}:start")),
            serde_json::json!({
                "run_id": run_id,
                "attempt_no": attempt_count,
                "resolution": "cancel the uncertain run to abandon it before starting new work"
            }),
            now,
        )?;
        return Ok(session_mutation(
            session_id,
            "recovery_required",
            Some(run_id),
            vec![event],
        ));
    }
    // Surfacing an uncertain prior attempt above takes precedence; everything
    // past this point wakes or creates work, which needs a claimable workspace.
    require_session_workspace(&session)?;
    tx.execute(
        "UPDATE hive_controllers SET status = 'active', updated_at = ?2 WHERE id = ?1",
        params![controller.id, now],
    )?;
    let existing: Option<(String, String, i64)> = tx
        .query_row(
            "SELECT id, status, attempt_count FROM hive_runs
             WHERE controller_id = ?1
               AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input')
             ORDER BY created_at DESC LIMIT 1",
            [&controller.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (run_id, event_type, dedupe_key) = if let Some((existing, status, attempt_count)) = existing
    {
        if matches!(
            status.as_str(),
            "sleeping" | "retry_wait" | "awaiting_input"
        ) {
            tx.execute(
                "UPDATE hive_runs
                 SET status = 'queued', available_at = ?2, wake_at = NULL,
                     last_stop_reason = NULL, updated_at = ?2
                 WHERE id = ?1 AND status IN ('sleeping', 'retry_wait', 'awaiting_input')",
                params![existing, now],
            )?;
            align_run_projection(tx, &existing, "queued", now)?;
            let dedupe_key = format!("transition:{existing}:{attempt_count}:queued");
            (existing, "run_requeued", Some(dedupe_key))
        } else {
            (existing, "run_start_requested", None)
        }
    } else {
        let run_id = uuid::Uuid::new_v4().to_string();
        let objective = tx
            .query_row(
                "SELECT content FROM messages
                 WHERE session_id = ?1 AND role = 'user' ORDER BY id DESC LIMIT 1",
                [session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|content| canonical_text(&content))
            .unwrap_or_else(|| format!("Continue {}", session.title));
        insert_run(
            tx,
            &run_id,
            &controller.id,
            Some(session_id),
            None,
            None,
            "legacy_resume",
            &objective,
            serde_json::json!({
                "working_dir": session.working_dir,
                "project_dir": session.project_dir,
                "model": session.model,
                "model_key": session.model_key,
                "model_catalog_revision": session.model_catalog_revision,
                "permission_mode": require_frozen_session_permission_mode(&session)?,
                "retry": RetryPolicy::default(),
            }),
            0,
            now,
            5,
            now,
        )?;
        let dedupe_key = format!("run:{run_id}:queued");
        (run_id, "run_queued", Some(dedupe_key))
    };
    let event = append_event(
        tx,
        &controller,
        event_type,
        Some(&run_id),
        None,
        dedupe_key.as_deref(),
        serde_json::json!({"run_id": run_id, "kind": "legacy_resume"}),
        now,
    )?;
    Ok(session_mutation(
        session_id,
        "queued",
        Some(run_id),
        vec![event],
    ))
}

fn schedule_session(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    wake_at_unix_ms: i64,
    reason: &str,
) -> Result<Mutation, RuntimeStoreError> {
    validate_bounded_field(reason, "schedule reason", 8 * 1024)?;
    let session = require_owned_session(tx, actor, session_id)?;
    let _ = require_frozen_session_model(&session)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let wake_at = unix_millis_to_utc(wake_at_unix_ms)?;
    let schedule_id = uuid::Uuid::new_v4().to_string();
    let recurrence = serde_json::to_string(&RecurrenceV1::Once { at: wake_at })
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    let wake_at = canonical_timestamp(wake_at);
    tx.execute(
        "INSERT INTO hive_schedules (
            id, controller_id, title, summary, objective, recurrence_kind,
            recurrence_json, timezone, gap_policy, fold_policy, next_fire_at,
            last_scheduled_for, status, priority, project_dir, model,
            model_key_json, model_catalog_revision, crew_slug,
            misfire_policy, misfire_grace_secs, catch_up_limit, overlap_policy,
            max_attempts, retry_base_secs, retry_max_secs, retry_jitter,
            revision, created_by, created_at, updated_at
         ) SELECT ?1, ?2, ?3, ?3, ?3, 'once', ?4, ?5, 'shift_forward', 'first',
                  ?6, NULL, 'enabled', 0, s.project_dir, s.model,
                  s.model_key_json, s.model_catalog_revision, rs.crew_slug,
                  'fire_once', 300, 1, 'queue_one', 5, 15, 900, 'full', 0,
                  ?7, ?8, ?8
           FROM sessions s
           LEFT JOIN hive_runtime_state rs ON rs.session_id = s.id
          WHERE s.id = ?9",
        params![
            schedule_id,
            controller.id,
            reason,
            recurrence,
            controller.timezone,
            wake_at,
            actor.user_id.as_deref().unwrap_or("local"),
            now,
            session_id,
        ],
    )?;
    let event = append_event(
        tx,
        &controller,
        "schedule_created",
        None,
        Some(&schedule_id),
        Some(&format!("schedule:{schedule_id}:created")),
        serde_json::json!({"schedule_id": schedule_id, "next_fire_at": wake_at}),
        now,
    )?;
    Ok(session_mutation(
        session_id,
        "scheduled",
        Some(schedule_id),
        vec![event],
    ))
}

fn set_controller_status(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    status: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, session_id)?;
    if status == "active" {
        freeze_session_model_into_open_runs(tx, &session, now)?;
    }
    let mut controller = get_or_create_controller(tx, &session, now)?;
    let previous = controller.status.clone();
    if status == "active" {
        if let Some((run_id, attempt_count)) = recovery_required_run(tx, &controller.id)? {
            tx.execute(
                "UPDATE hive_controllers SET status = 'paused', updated_at = ?2 WHERE id = ?1",
                params![controller.id, now],
            )?;
            controller.status = "paused".into();
            tx.execute(
                "INSERT INTO hive_runtime_state (session_id, status, current_run_id, updated_at)
                 VALUES (?1, 'error', ?2, ?3)
                 ON CONFLICT(session_id) DO UPDATE SET status = 'error',
                     current_run_id = excluded.current_run_id,
                     updated_at = excluded.updated_at",
                params![session_id, run_id, now],
            )?;
            let event = append_event(
                tx,
                &controller,
                "resume_blocked_recovery_required",
                Some(&run_id),
                None,
                Some(&format!("recovery_block:{run_id}:{attempt_count}:resume")),
                serde_json::json!({
                    "run_id": run_id,
                    "attempt_no": attempt_count,
                    "previous": previous,
                    "current": "paused",
                    "resolution": "cancel the uncertain run to abandon it before resuming"
                }),
                now,
            )?;
            return Ok(session_mutation(
                session_id,
                "recovery_required",
                Some(run_id),
                vec![event],
            ));
        }
    }
    tx.execute(
        "UPDATE hive_controllers SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![controller.id, status, now],
    )?;
    controller.status = status.to_string();
    let active_runs = if status == "paused" {
        let mut statement = tx.prepare(
            "SELECT id, status, attempt_count, lease_token FROM hive_runs
             WHERE controller_id = ?1 AND status IN ('leased', 'running')",
        )?;
        let active = statement
            .query_map([&controller.id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        active
    } else {
        Vec::new()
    };
    for (run_id, run_status, attempt_no, lease_token) in &active_runs {
        let (target, reason) = if run_status == "leased" {
            ("queued", "paused before execution; safely requeued")
        } else {
            (
                "recovery_required",
                "paused during execution; side effects may be uncertain",
            )
        };
        tx.execute(
            "UPDATE hive_runs
             SET status = ?2, lease_owner = NULL, lease_token = NULL,
                 lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
                 last_stop_reason = 'paused by user', last_error = ?3, updated_at = ?4
             WHERE id = ?1 AND status = ?5",
            params![run_id, target, reason, now, run_status],
        )?;
        if let Some(lease_token) = lease_token {
            tx.execute(
                "UPDATE hive_run_attempts
                 SET finished_at = ?4, outcome = 'abandoned', stop_reason = 'paused by user',
                     error = ?5
                 WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3
                   AND finished_at IS NULL",
                params![run_id, attempt_no, lease_token, now, reason],
            )?;
        }
        if target == "recovery_required" {
            tx.execute(
                "UPDATE hive_control_outbox
                 SET status = 'discarded',
                     last_error = 'run entered recovery before control delivery',
                     updated_at = ?2
                 WHERE run_id = ?1 AND status = 'pending'",
                params![run_id, now],
            )?;
            tx.execute(
                "UPDATE hive_schedule_occurrences SET status = 'failed',
                     decision_reason = ?2, updated_at = ?3
                 WHERE run_id = ?1 AND status = 'running'",
                params![run_id, reason, now],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO hive_runtime_state (session_id, status, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET status = excluded.status, updated_at = excluded.updated_at",
        params![session_id, if status == "paused" { "paused" } else { "idle" }, now],
    )?;
    let event_type = if status == "paused" {
        "controller_paused"
    } else {
        "controller_started"
    };
    let event = append_event(
        tx,
        &controller,
        event_type,
        None,
        None,
        None,
        serde_json::json!({"previous": previous, "current": status}),
        now,
    )?;
    let mut events = vec![event];
    for (run_id, run_status, attempt_count, _) in active_runs {
        let (event_type, reason, target_status) = if run_status == "leased" {
            (
                "run_lease_requeued",
                "paused before execution; safely requeued",
                "queued",
            )
        } else {
            (
                "recovery_required",
                "paused during execution; side effects may be uncertain",
                "recovery_required",
            )
        };
        let dedupe_key = format!("transition:{run_id}:{attempt_count}:{target_status}");
        events.push(append_event(
            tx,
            &controller,
            event_type,
            Some(&run_id),
            None,
            Some(&dedupe_key),
            serde_json::json!({"run_id": run_id, "reason": reason}),
            now,
        )?);
    }
    Ok(session_mutation(session_id, status, None, events))
}

fn cancel_session(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let cancellable = {
        let mut statement = tx.prepare(
            "SELECT id, attempt_count, schedule_id, status, lease_token FROM hive_runs
             WHERE controller_id = ?1
               AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        )?;
        let rows = statement
            .query_map([&controller.id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.execute(
        "UPDATE hive_control_outbox
         SET status = 'discarded', last_error = 'session cancelled', updated_at = ?2
         WHERE controller_id = ?1 AND status = 'pending'",
        params![controller.id, now],
    )?;
    tx.execute(
        "UPDATE hive_schedule_occurrences
         SET status = 'cancelled', decision_reason = 'cancelled by user', updated_at = ?2
         WHERE run_id IN (
             SELECT id FROM hive_runs WHERE controller_id = ?1
               AND status IN ('queued', 'leased', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')
         ) AND status IN ('pending', 'queued', 'running')",
        params![controller.id, now],
    )?;
    tx.execute(
        "UPDATE hive_runs
         SET status = 'cancelled', lease_owner = NULL, lease_token = NULL,
             lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
             wake_at = NULL, last_stop_reason = 'cancelled by user',
             finished_at = ?2, updated_at = ?2
         WHERE controller_id = ?1
           AND status IN ('queued', 'leased', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        params![controller.id, now],
    )?;
    tx.execute(
        "UPDATE hive_schedules SET status = 'cancelled', revision = revision + 1,
                 updated_at = ?2
         WHERE controller_id = ?1 AND status IN ('enabled', 'paused')",
        params![controller.id, now],
    )?;
    tx.execute(
        "UPDATE hive_controllers SET status = 'disabled', updated_at = ?2 WHERE id = ?1",
        params![controller.id, now],
    )?;
    tx.execute(
        "INSERT INTO hive_runtime_state (session_id, status, updated_at)
         VALUES (?1, 'cancelled', ?2)
         ON CONFLICT(session_id) DO UPDATE SET status = 'cancelled', updated_at = excluded.updated_at",
        params![session_id, now],
    )?;
    let mut events = Vec::with_capacity(cancellable.len() + 1);
    for (run_id, attempt_count, schedule_id, status, lease_token) in cancellable {
        if status == "leased" {
            if let Some(lease_token) = lease_token {
                tx.execute(
                    "UPDATE hive_run_attempts
                     SET finished_at = ?4, outcome = 'cancelled',
                         stop_reason = 'cancelled by user'
                     WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3
                       AND finished_at IS NULL",
                    params![run_id, attempt_count, lease_token, now],
                )?;
            }
        }
        if status == "running" {
            events.push(append_event(
                tx,
                &controller,
                "cancellation_requested",
                Some(&run_id),
                schedule_id.as_deref(),
                Some(&format!("cancel_requested:{run_id}:{attempt_count}")),
                serde_json::json!({"run_id": run_id, "attempt": attempt_count}),
                now,
            )?);
        } else {
            let dedupe_key = format!("transition:{run_id}:{attempt_count}:cancelled");
            events.push(append_event(
                tx,
                &controller,
                "run_cancelled",
                Some(&run_id),
                schedule_id.as_deref(),
                Some(&dedupe_key),
                serde_json::json!({"run_id": run_id, "reason": "cancelled by user"}),
                now,
            )?);
        }
    }
    events.push(append_event(
        tx,
        &controller,
        "session_cancelled",
        None,
        None,
        None,
        serde_json::json!({"reason": "cancelled by user"}),
        now,
    )?);
    Ok(session_mutation(session_id, "cancelled", None, events))
}

fn delete_session(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let has_active_run = tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM hive_runs
             WHERE controller_id = ?1 AND status IN ('leased', 'running', 'recovery_required')
         )",
        [&controller.id],
        |row| row.get::<_, bool>(0),
    )?;
    if has_active_run {
        return Err(RuntimeStoreError::StateConflict(
            "session has an active run; cancel it and wait for quiescence before deleting".into(),
        ));
    }
    let event = append_event(
        tx,
        &controller,
        "session_deleted",
        None,
        None,
        None,
        serde_json::json!({"deleted": true}),
        now,
    )?;
    tx.execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
    Ok(Mutation {
        response: ack("session deleted"),
        resource_id: Some(session_id.to_string()),
        events: vec![event],
    })
}

fn send_message(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    message: &str,
    pending_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    if message.trim().is_empty() {
        return Err(RuntimeStoreError::Invalid("message is empty".into()));
    }
    if message.len() > 64 * 1024 {
        return Err(RuntimeStoreError::Invalid(
            "message exceeds 65536 bytes".into(),
        ));
    }
    let session = require_owned_session(tx, actor, session_id)?;
    freeze_session_model_into_open_runs(tx, &session, now)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let active_run_id = tx
        .query_row(
            "SELECT id FROM hive_runs
             WHERE controller_id = ?1 AND status IN ('leased', 'running')
             ORDER BY updated_at DESC LIMIT 1",
            [&controller.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(run_id) = active_run_id {
        insert_pending_user_message(tx, session_id, pending_id, message, now)?;
        let event = append_event(
            tx,
            &controller,
            "message_staged",
            Some(&run_id),
            None,
            Some(&format!("pending_message:{pending_id}")),
            serde_json::json!({
                "run_id": run_id,
                "pending_id": pending_id,
            }),
            now,
        )?;
        return Ok(Mutation {
            response: ack("message staged for the active run"),
            resource_id: Some(session_id.to_string()),
            events: vec![event],
        });
    }
    insert_canonical_user_message(tx, session_id, message, now)?;
    let event = append_event(
        tx,
        &controller,
        "message_received",
        None,
        None,
        None,
        serde_json::json!({
            "message_bytes": message.len(),
            "message_chars": message.chars().count(),
        }),
        now,
    )?;
    let mut events = vec![event];
    let resumed = resume_waiting_run(tx, &controller, now)?;
    if let Some((run_id, previous_status, attempt_count)) = resumed {
        let dedupe_key = format!("transition:{run_id}:{attempt_count}:queued");
        events.push(append_event(
            tx,
            &controller,
            "run_requeued",
            Some(&run_id),
            None,
            Some(&dedupe_key),
            serde_json::json!({
                "run_id": run_id,
                "previous_status": previous_status,
                "reason": "user message received"
            }),
            now,
        )?);
    } else if let Some(event) = queue_message_turn_if_idle(tx, &session, &controller, message, now)?
    {
        events.push(event);
    }
    Ok(Mutation {
        response: ack("message accepted"),
        resource_id: Some(session_id.to_string()),
        events,
    })
}

fn stage_steer(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    pending_id: &str,
    content: Value,
) -> Result<Mutation, RuntimeStoreError> {
    validate_bounded_field(pending_id, "pending id", 256)?;
    let content = serde_json::from_value::<Vec<Content>>(content).map_err(|error| {
        RuntimeStoreError::Invalid(format!("invalid steering content: {error}"))
    })?;
    if content.is_empty() {
        return Err(RuntimeStoreError::Invalid(
            "steering content is empty".into(),
        ));
    }
    let content_json = serde_json::to_string(&content)
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    if content_json.len() > 256 * 1024 {
        return Err(RuntimeStoreError::Invalid(
            "steering content exceeds 262144 bytes".into(),
        ));
    }
    let session = require_owned_session(tx, actor, session_id)?;
    freeze_session_model_into_open_runs(tx, &session, now)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    insert_pending_user_content(tx, session_id, pending_id, &content_json, now)?;
    let active_run_id = tx
        .query_row(
            "SELECT id FROM hive_runs
             WHERE controller_id = ?1 AND status IN ('leased', 'running')
             ORDER BY updated_at DESC LIMIT 1",
            [&controller.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let run_id = active_run_id.as_deref();
    let mut events = vec![append_event(
        tx,
        &controller,
        "steer_staged",
        run_id,
        None,
        Some(&format!("pending_steer:{pending_id}")),
        serde_json::json!({
            "run_id": run_id,
            "pending_id": pending_id,
        }),
        now,
    )?];
    if active_run_id.is_none() {
        if let Some((run_id, previous_status, attempt_count)) =
            resume_waiting_run(tx, &controller, now)?
        {
            let dedupe_key = format!("transition:{run_id}:{attempt_count}:queued");
            events.push(append_event(
                tx,
                &controller,
                "run_requeued",
                Some(&run_id),
                None,
                Some(&dedupe_key),
                serde_json::json!({
                    "run_id": run_id,
                    "previous_status": previous_status,
                    "reason": "durable steering received"
                }),
                now,
            )?);
        } else {
            let objective = steering_objective(&content);
            if let Some(event) =
                queue_message_turn_if_idle(tx, &session, &controller, &objective, now)?
            {
                events.push(event);
            }
        }
    }
    Ok(Mutation {
        response: ack("steering durably staged"),
        resource_id: Some(session_id.to_string()),
        events,
    })
}

fn steering_objective(content: &[Content]) -> String {
    let text = content
        .iter()
        .filter_map(|item| match item {
            Content::Text { text } if !text.trim().is_empty() => Some(text.trim()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.is_empty() {
        "Process the durable user steering attached to this session".into()
    } else {
        text
    }
}

fn stage_tool_approval(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    requested_run_id: &str,
    tool_call_id: &str,
    approved: bool,
) -> Result<Mutation, RuntimeStoreError> {
    validate_bounded_field(requested_run_id, "run id", 256)?;
    validate_bounded_field(tool_call_id, "tool call id", 512)?;
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let run_id = tx
        .query_row(
            "SELECT id FROM hive_runs
             WHERE controller_id = ?1 AND id = ?2 AND status IN ('leased', 'running')",
            params![controller.id, requested_run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            RuntimeStoreError::Invalid("the exact run is no longer accepting tool approvals".into())
        })?;
    let pending_event_type = tx
        .query_row(
            "SELECT json_extract(payload_json, '$.type')
             FROM hive_controller_events
             WHERE controller_id = ?1 AND run_id = ?2 AND event_type = 'agentic_event'
               AND json_extract(payload_json, '$.id') = ?3
               AND json_extract(payload_json, '$.type') IN (
                   'tool_approval_required', 'tool_approved', 'tool_denied', 'tool_result'
               )
             ORDER BY sequence DESC LIMIT 1",
            params![controller.id, run_id, tool_call_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if pending_event_type.as_deref() != Some("tool_approval_required") {
        return Err(RuntimeStoreError::StateConflict(format!(
            "tool call {tool_call_id} is not awaiting approval on run {run_id}"
        )));
    }
    let existing = tx
        .query_row(
            "SELECT payload_json FROM hive_control_outbox
             WHERE controller_id = ?1 AND control_kind = 'tool_approval'
               AND dedupe_key = ?2",
            params![controller.id, format!("{run_id}:{tool_call_id}")],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        let existing = serde_json::from_str::<Value>(&existing)
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        if existing.get("approved").and_then(Value::as_bool) != Some(approved) {
            return Err(RuntimeStoreError::Invalid(format!(
                "tool call {tool_call_id} already has a different approval decision"
            )));
        }
    } else {
        let id = crate::legacy_identity::tool_approval_id(&controller.id, &run_id, tool_call_id);
        let payload = serde_json::to_string(&serde_json::json!({
            "tool_call_id": tool_call_id,
            "approved": approved,
        }))
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        tx.execute(
            "INSERT INTO hive_control_outbox (
                id, controller_id, session_id, run_id, control_kind, dedupe_key,
                payload_json, status, attempt_count, available_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'tool_approval', ?5, ?6, 'pending', 0, ?7, ?7, ?7)",
            params![
                id,
                controller.id,
                session_id,
                run_id,
                format!("{run_id}:{tool_call_id}"),
                payload,
                now
            ],
        )?;
    }
    let event = append_event(
        tx,
        &controller,
        "tool_approval_queued",
        Some(&run_id),
        None,
        Some(&format!("tool_approval:{run_id}:{tool_call_id}")),
        serde_json::json!({
            "run_id": run_id,
            "tool_call_id": tool_call_id,
            "approved": approved,
        }),
        now,
    )?;
    Ok(Mutation {
        response: ack("tool approval durably queued"),
        resource_id: Some(session_id.to_string()),
        events: vec![event],
    })
}

fn user_response(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    requested_run_id: &str,
    tool_call_id: &str,
    response: &str,
    pending_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    validate_bounded_field(requested_run_id, "run id", 256)?;
    validate_bounded_field(tool_call_id, "tool call id", 512)?;
    validate_bounded_field(response, "user response", 64 * 1024)?;
    validate_bounded_field(pending_id, "pending id", 256)?;
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let exact_run_status = tx
        .query_row(
            "SELECT status FROM hive_runs WHERE controller_id = ?1 AND id = ?2
             AND status IN ('leased', 'running', 'awaiting_input')",
            params![controller.id, requested_run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            RuntimeStoreError::StateConflict(
                "the exact run is no longer accepting a user response".into(),
            )
        })?;
    let pending_event_type = tx
        .query_row(
            "SELECT CASE
                 WHEN event_type = 'agentic_event'
                   THEN json_extract(payload_json, '$.type')
                 ELSE event_type
             END
             FROM hive_controller_events
             WHERE controller_id = ?1 AND run_id = ?2
               AND (
                   (event_type = 'agentic_event'
                    AND json_extract(payload_json, '$.type') = 'awaiting_input'
                    AND json_extract(payload_json, '$.tool_call_id') = ?3)
                   OR
                   (event_type IN ('user_response_received', 'user_response_staged')
                    AND json_extract(payload_json, '$.tool_call_id') = ?3)
               )
             ORDER BY sequence DESC LIMIT 1",
            params![controller.id, requested_run_id, tool_call_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if pending_event_type.as_deref() != Some("awaiting_input") {
        return Err(RuntimeStoreError::StateConflict(format!(
            "tool call {tool_call_id} is not awaiting a response on run {requested_run_id}"
        )));
    }
    let durable_response = format!("Response to {tool_call_id}:\n{response}");
    let active_run_id = tx
        .query_row(
            "SELECT id FROM hive_runs
             WHERE controller_id = ?1 AND status IN ('leased', 'running')
             ORDER BY updated_at DESC LIMIT 1",
            [&controller.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if active_run_id
        .as_deref()
        .is_some_and(|run_id| run_id != requested_run_id)
    {
        return Err(RuntimeStoreError::StateConflict(
            "a replacement run is active; refusing to redirect the response".into(),
        ));
    }
    if exact_run_status == "awaiting_input" && active_run_id.is_some() {
        return Err(RuntimeStoreError::StateConflict(
            "persisted run state disagrees with the active execution".into(),
        ));
    }

    // Ask-user completion and the durable run transition are not one atomic
    // operation. If the answer arrives while the run is still active, stage
    // it under an idempotent non-canonical role. A live loop can promote it at
    // its next boundary; otherwise finish_execution observes the staging row,
    // yields immediately, and the replacement run promotes it before loading
    // history. This prevents an answer from being stranded behind a just-
    // committed awaiting_input transition.
    if let Some(run_id) = active_run_id {
        insert_pending_user_message(tx, session_id, pending_id, &durable_response, now)?;
        let event = append_event(
            tx,
            &controller,
            "user_response_staged",
            Some(&run_id),
            None,
            Some(&format!("pending_response:{pending_id}")),
            serde_json::json!({
                "run_id": run_id,
                "pending_id": pending_id,
                "tool_call_id": tool_call_id,
            }),
            now,
        )?;
        return Ok(Mutation {
            response: ack("user response staged for the active run"),
            resource_id: Some(session_id.to_string()),
            events: vec![event],
        });
    }

    insert_canonical_user_message(tx, session_id, &durable_response, now)?;
    let mut events = vec![append_event(
        tx,
        &controller,
        "user_response_received",
        Some(requested_run_id),
        None,
        None,
        serde_json::json!({
            "tool_call_id": tool_call_id,
            "response_bytes": response.len(),
            "response_chars": response.chars().count(),
        }),
        now,
    )?];
    let attempt_count = tx
        .query_row(
            "SELECT attempt_count FROM hive_runs
             WHERE id = ?1 AND controller_id = ?2 AND status = 'awaiting_input'",
            params![requested_run_id, controller.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| {
            RuntimeStoreError::StateConflict(
                "the exact run is no longer awaiting the requested response".into(),
            )
        })?;
    let changed = tx.execute(
        "UPDATE hive_runs
         SET status = 'queued', available_at = ?3, wake_at = NULL, updated_at = ?3
         WHERE id = ?1 AND controller_id = ?2 AND status = 'awaiting_input'",
        params![requested_run_id, controller.id, now],
    )?;
    if changed != 1 {
        return Err(RuntimeStoreError::StateConflict(
            "the exact run changed state before its response could be resumed".into(),
        ));
    }
    align_run_projection(tx, requested_run_id, "queued", now)?;
    let dedupe_key = format!("transition:{requested_run_id}:{attempt_count}:queued");
    events.push(append_event(
        tx,
        &controller,
        "run_requeued",
        Some(requested_run_id),
        None,
        Some(&dedupe_key),
        serde_json::json!({
            "run_id": requested_run_id,
            "previous_status": "awaiting_input",
            "reason": "user response received"
        }),
        now,
    )?);
    Ok(Mutation {
        response: ack("user response accepted"),
        resource_id: Some(session_id.to_string()),
        events,
    })
}

fn resume_waiting_run(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    now: &str,
) -> Result<Option<(String, String, i64)>, RuntimeStoreError> {
    let waiting = {
        let mut statement = tx.prepare(
            "SELECT id, status, attempt_count FROM hive_runs
             WHERE controller_id = ?1 AND status IN ('sleeping', 'awaiting_input')
             ORDER BY updated_at DESC, created_at DESC LIMIT 2",
        )?;
        let waiting = statement
            .query_map([&controller.id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        waiting
    };
    if waiting.len() > 1 {
        return Err(RuntimeStoreError::StateConflict(
            "multiple runs are waiting; an exact run id is required to resume one".into(),
        ));
    }
    let Some((run_id, previous_status, attempt_count)) = waiting.into_iter().next() else {
        return Ok(None);
    };
    let changed = tx.execute(
        "UPDATE hive_runs
         SET status = 'queued', available_at = ?2, wake_at = NULL,
             last_stop_reason = NULL, updated_at = ?2
         WHERE id = ?1 AND status IN ('sleeping', 'awaiting_input')",
        params![run_id, now],
    )?;
    if changed != 1 {
        return Ok(None);
    }
    align_run_projection(tx, &run_id, "queued", now)?;
    Ok(Some((run_id, previous_status, attempt_count)))
}

fn queue_message_turn_if_idle(
    tx: &Transaction<'_>,
    session: &super::persistence::OwnedSession,
    controller: &ControllerRecord,
    message: &str,
    now: &str,
) -> Result<Option<PersistedEvent>, RuntimeStoreError> {
    let _ = require_frozen_session_model(session)?;
    require_session_workspace(session)?;
    let unfinished: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_runs
         WHERE controller_id = ?1
           AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        [&controller.id],
        |row| row.get(0),
    )?;
    if unfinished > 0 {
        return Ok(None);
    }
    let (priority_name, crew_slug) = tx
        .query_row(
            "SELECT priority, crew_slug FROM hive_runtime_state WHERE session_id = ?1",
            [&session.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .unwrap_or_else(|| ("normal".to_string(), None));
    let priority = priority_value(&priority_name).unwrap_or(0);
    let run_id = uuid::Uuid::new_v4().to_string();
    insert_run(
        tx,
        &run_id,
        &controller.id,
        Some(&session.id),
        None,
        None,
        "legacy_resume",
        message,
        serde_json::json!({
            "working_dir": session.working_dir,
            "project_dir": session.project_dir,
            "model": session.model,
            "model_key": session.model_key,
            "model_catalog_revision": session.model_catalog_revision,
            "permission_mode": require_frozen_session_permission_mode(session)?,
            "crew_slug": crew_slug,
            "retry": RetryPolicy::default(),
        }),
        priority,
        now,
        5,
        now,
    )?;
    let runtime_status = if controller.status == "paused" {
        "paused"
    } else {
        "idle"
    };
    tx.execute(
        "INSERT INTO hive_runtime_state (session_id, status, current_run_id, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(session_id) DO UPDATE SET current_run_id = excluded.current_run_id,
             status = CASE WHEN hive_runtime_state.status = 'paused' THEN 'paused' ELSE excluded.status END,
             updated_at = excluded.updated_at",
        params![session.id, runtime_status, run_id, now],
    )?;
    let event = append_event(
        tx,
        controller,
        "run_queued",
        Some(&run_id),
        None,
        Some(&format!("run:{run_id}:queued")),
        serde_json::json!({
            "run_id": run_id,
            "kind": "message_turn",
        }),
        now,
    )?;
    Ok(Some(event))
}

fn normalize_model_identity(
    model: Option<String>,
    model_key: Option<ModelKey>,
    model_catalog_revision: Option<String>,
    context: &str,
) -> Result<(Option<String>, Option<ModelKey>, Option<String>), RuntimeStoreError> {
    let model = model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if model
        .as_deref()
        .is_some_and(|value| value.len() > 512 || value.as_bytes().contains(&0))
    {
        return Err(RuntimeStoreError::Invalid(format!(
            "{context} model is invalid or exceeds 512 bytes"
        )));
    }

    if let Some(key) = model_key.as_ref() {
        for (field, value) in [
            ("provider", key.provider.as_str()),
            ("model_id", key.model_id.as_str()),
            ("api_format", key.api_format.as_str()),
        ] {
            if value.trim().is_empty()
                || value != value.trim()
                || value.len() > 512
                || value.as_bytes().contains(&0)
            {
                return Err(RuntimeStoreError::Invalid(format!(
                    "{context} model_key.{field} is invalid"
                )));
            }
        }
        if key.auth_scope.as_deref().is_some_and(|value| {
            value.trim().is_empty()
                || value != value.trim()
                || value.len() > 128
                || value.as_bytes().contains(&0)
        }) {
            return Err(RuntimeStoreError::Invalid(format!(
                "{context} model_key.auth_scope is invalid"
            )));
        }
        if model.as_deref().is_some_and(|model| model != key.model_id) {
            return Err(RuntimeStoreError::Invalid(format!(
                "{context} model must match model_key.model_id"
            )));
        }
        let serialized =
            serde_json::to_value(key).map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        serde_json::from_value::<CoreModelKey>(serialized).map_err(|error| {
            RuntimeStoreError::Invalid(format!("{context} model_key is unsupported: {error}"))
        })?;
    }

    let model = model.or_else(|| model_key.as_ref().map(|key| key.model_id.clone()));
    let model_catalog_revision = model_catalog_revision
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if model_catalog_revision
        .as_deref()
        .is_some_and(|value| value.len() > 512 || value.as_bytes().contains(&0))
    {
        return Err(RuntimeStoreError::Invalid(format!(
            "{context} model catalog revision is invalid or exceeds 512 bytes"
        )));
    }
    if model_key.is_none() && model_catalog_revision.is_some() {
        return Err(RuntimeStoreError::Invalid(format!(
            "{context} model catalog revision requires model_key"
        )));
    }
    Ok((model, model_key, model_catalog_revision))
}

fn require_frozen_session_model(
    session: &super::persistence::OwnedSession,
) -> Result<&str, RuntimeStoreError> {
    normalize_model_identity(
        session.model.clone(),
        session.model_key.clone(),
        session.model_catalog_revision.clone(),
        "Hive session",
    )?;
    let model = session
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            RuntimeStoreError::StateConflict(
                "Hive session has no frozen model; select a model before starting it".into(),
            )
        })?;
    if session
        .model_key
        .as_ref()
        .is_some_and(|key| key.model_id != model)
    {
        return Err(RuntimeStoreError::StateConflict(
            "Hive session model does not match its frozen model key".into(),
        ));
    }
    if session.model_key.is_none() && session.model_catalog_revision.is_some() {
        return Err(RuntimeStoreError::StateConflict(
            "Hive session catalog revision has no frozen model key".into(),
        ));
    }
    Ok(model)
}

fn require_frozen_session_permission_mode(
    session: &super::persistence::OwnedSession,
) -> Result<&str, RuntimeStoreError> {
    match session.permission_mode.as_str() {
        "supervised" | "autonomous" => Ok(session.permission_mode.as_str()),
        _ => Err(RuntimeStoreError::StateConflict(
            "Hive session has an invalid permission mode".into(),
        )),
    }
}

/// Enqueue-time mirror of the execution host's claim validation, which
/// refuses a daemon-default workspace and requires absolute paths. Sessions
/// predating explicit Hive workspaces would otherwise enqueue runs that are
/// doomed to fail their claim with a redacted, non-actionable error.
fn require_session_workspace(
    session: &super::persistence::OwnedSession,
) -> Result<(), RuntimeStoreError> {
    let paths: Vec<&str> = [
        session.working_dir.as_deref(),
        session.project_dir.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .collect();
    if paths.is_empty() {
        return Err(RuntimeStoreError::StateConflict(
            "Hive session has no working or project directory; set a workspace before starting it"
                .into(),
        ));
    }
    if paths.iter().any(|path| !Path::new(path).is_absolute()) {
        return Err(RuntimeStoreError::StateConflict(
            "Hive session workspace paths must be absolute".into(),
        ));
    }
    Ok(())
}

fn freeze_session_model_into_open_runs(
    tx: &Transaction<'_>,
    session: &super::persistence::OwnedSession,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    let model = require_frozen_session_model(session)?;
    let permission_mode = require_frozen_session_permission_mode(session)?;
    let session_key_value = session
        .model_key
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    let mut statement = tx.prepare(
        "SELECT id, schedule_id, config_json FROM hive_runs
         WHERE session_id = ?1
           AND status IN ('queued', 'sleeping', 'retry_wait', 'awaiting_input')",
    )?;
    let rows = statement
        .query_map([&session.id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    for (run_id, schedule_id, serialized) in rows {
        let mut config = serde_json::from_str::<Value>(&serialized)
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        let object = config.as_object_mut().ok_or_else(|| {
            RuntimeStoreError::StateConflict("Hive run config is not a JSON object".into())
        })?;
        let missing_model = object
            .get("model")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty());
        let missing_permission = object.get("permission_mode").is_none();
        let should_backfill_model = missing_model && schedule_id.is_none();
        let run_model_matches_session = object
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim() == model);
        // Scheduled runs own their selection. Only legacy, non-scheduled runs
        // may inherit provider-aware identity from their parent session.
        let can_inherit_session_identity =
            schedule_id.is_none() && (missing_model || run_model_matches_session);
        let missing_key = object.get("model_key").is_none_or(Value::is_null);
        let should_backfill_key =
            can_inherit_session_identity && session_key_value.is_some() && missing_key;
        let key_matches_session = session_key_value
            .as_ref()
            .is_some_and(|key| object.get("model_key") == Some(key));
        let missing_revision = object
            .get("model_catalog_revision")
            .is_none_or(Value::is_null);
        let should_backfill_revision = can_inherit_session_identity
            && session.model_catalog_revision.is_some()
            && missing_revision
            && (should_backfill_key || key_matches_session);
        if !should_backfill_model
            && !missing_permission
            && !should_backfill_key
            && !should_backfill_revision
        {
            continue;
        }
        if should_backfill_model {
            object.insert("model".into(), Value::String(model.to_string()));
        }
        if should_backfill_key {
            object.insert(
                "model_key".into(),
                session_key_value
                    .clone()
                    .expect("backfill requires a frozen session key"),
            );
        }
        if should_backfill_revision {
            object.insert(
                "model_catalog_revision".into(),
                Value::String(
                    session
                        .model_catalog_revision
                        .clone()
                        .expect("backfill requires a frozen catalog revision"),
                ),
            );
        }
        if missing_permission {
            object.insert(
                "permission_mode".into(),
                Value::String(permission_mode.to_string()),
            );
        }
        tx.execute(
            "UPDATE hive_runs SET config_json = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                run_id,
                serde_json::to_string(&config)
                    .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
                now
            ],
        )?;
    }
    Ok(())
}

fn recovery_required_run(
    tx: &Transaction<'_>,
    controller_id: &str,
) -> Result<Option<(String, i64)>, RuntimeStoreError> {
    tx.query_row(
        "SELECT id, attempt_count FROM hive_runs
         WHERE controller_id = ?1 AND status = 'recovery_required'
         ORDER BY updated_at DESC, id ASC LIMIT 1",
        [controller_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(RuntimeStoreError::from)
}

fn align_run_projection(
    tx: &Transaction<'_>,
    run_id: &str,
    status: &str,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    let (controller_id, session_id, occurrence_id) = tx.query_row(
        "SELECT controller_id, session_id, occurrence_id FROM hive_runs WHERE id = ?1",
        [run_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;
    if status == "recovery_required" {
        tx.execute(
            "UPDATE hive_controllers SET status = 'paused', updated_at = ?2 WHERE id = ?1",
            params![controller_id, now],
        )?;
    }
    if let Some(occurrence_id) = occurrence_id {
        let occurrence_status = match status {
            "queued" | "leased" => "queued",
            "succeeded" => "succeeded",
            "cancelled" => "cancelled",
            "failed" | "dead_letter" | "recovery_required" => "failed",
            _ => "running",
        };
        tx.execute(
            "UPDATE hive_schedule_occurrences
             SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![occurrence_id, occurrence_status, now],
        )?;
    }
    if let Some(session_id) = session_id {
        let recovery_run_id = tx
            .query_row(
                "SELECT id FROM hive_runs
                 WHERE controller_id = ?1 AND status = 'recovery_required'
                 ORDER BY updated_at DESC, id ASC LIMIT 1",
                [&controller_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let controller_status = tx.query_row(
            "SELECT status FROM hive_controllers WHERE id = ?1",
            [&controller_id],
            |row| row.get::<_, String>(0),
        )?;
        let active = tx
            .query_row(
                "SELECT id, status FROM hive_runs
                 WHERE controller_id = ?1
                   AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')
                 ORDER BY CASE status
                     WHEN 'running' THEN 0 WHEN 'leased' THEN 1
                     WHEN 'recovery_required' THEN 2 WHEN 'awaiting_input' THEN 3
                     WHEN 'sleeping' THEN 4 WHEN 'queued' THEN 5 ELSE 6 END,
                     updated_at DESC, id ASC LIMIT 1",
                [&controller_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let (runtime_status, current_run_id) = if let Some(recovery_run_id) = recovery_run_id {
            ("error", recovery_run_id)
        } else if controller_status == "paused" {
            (
                "paused",
                active
                    .map(|(active_run_id, _)| active_run_id)
                    .unwrap_or_else(|| run_id.to_string()),
            )
        } else if let Some((active_run_id, active_status)) = active {
            let runtime_status = match active_status.as_str() {
                "running" | "leased" => "running",
                "recovery_required" => "error",
                "awaiting_input" => "awaiting_input",
                "sleeping" => "sleeping",
                _ => "idle",
            };
            (runtime_status, active_run_id)
        } else {
            let runtime_status = match status {
                "cancelled" => "cancelled",
                "failed" | "dead_letter" | "recovery_required" => "error",
                _ => "idle",
            };
            (runtime_status, run_id.to_string())
        };
        tx.execute(
            "INSERT INTO hive_runtime_state (session_id, status, current_run_id, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET status = excluded.status,
                 current_run_id = excluded.current_run_id,
                 updated_at = excluded.updated_at",
            params![session_id, runtime_status, current_run_id, now],
        )?;
    }
    Ok(())
}

fn set_priority(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    priority: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let value = priority_value(priority)?;
    tx.execute(
        "UPDATE hive_runs SET priority = ?2, updated_at = ?3
         WHERE controller_id = ?1 AND status IN ('queued', 'sleeping', 'retry_wait')",
        params![controller.id, value, now],
    )?;
    tx.execute(
        "INSERT INTO hive_runtime_state (session_id, status, priority, updated_at)
         VALUES (?1, 'idle', ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET priority = excluded.priority, updated_at = excluded.updated_at",
        params![session_id, priority_name(value), now],
    )?;
    let event = append_event(
        tx,
        &controller,
        "priority_changed",
        None,
        None,
        None,
        serde_json::json!({"priority": priority, "value": value}),
        now,
    )?;
    Ok(session_mutation(session_id, "updated", None, vec![event]))
}

fn set_crew(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    crew_slug: Option<&str>,
) -> Result<Mutation, RuntimeStoreError> {
    if let Some(crew_slug) = crew_slug {
        if !is_valid_crew_slug(crew_slug) {
            return Err(RuntimeStoreError::Invalid("crew slug is invalid".into()));
        }
    }
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    tx.execute(
        "UPDATE hive_schedules SET crew_slug = ?2, revision = revision + 1, updated_at = ?3
         WHERE controller_id = ?1 AND status IN ('enabled', 'paused')",
        params![controller.id, crew_slug, now],
    )?;
    tx.execute(
        "UPDATE hive_runs
         SET config_json = json_set(config_json, '$.crew_slug', ?2), updated_at = ?3
         WHERE controller_id = ?1 AND status = 'queued'",
        params![controller.id, crew_slug, now],
    )?;
    tx.execute(
        "INSERT INTO hive_runtime_state (session_id, status, crew_slug, updated_at)
         VALUES (?1, 'idle', ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET crew_slug = excluded.crew_slug, updated_at = excluded.updated_at",
        params![session_id, crew_slug, now],
    )?;
    let event = append_event(
        tx,
        &controller,
        "crew_changed",
        None,
        None,
        None,
        serde_json::json!({"crew_slug": crew_slug}),
        now,
    )?;
    Ok(session_mutation(session_id, "updated", None, vec![event]))
}

fn recover(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: Option<&str>,
) -> Result<Mutation, RuntimeStoreError> {
    let mut controllers = Vec::new();
    if let Some(session_id) = session_id {
        let session = require_owned_session(tx, actor, session_id)?;
        controllers.push(get_or_create_controller(tx, &session, now)?);
    } else {
        let mut statement = tx.prepare(
            "SELECT c.id, c.session_id, c.status, c.timezone
             FROM hive_controllers c JOIN sessions s ON s.id = c.session_id
             WHERE ((?1 IS NULL AND s.user_id IS NULL) OR s.user_id = ?1)",
        )?;
        controllers = statement
            .query_map([actor.user_id.as_deref()], |row| {
                Ok(ControllerRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    status: row.get(2)?,
                    timezone: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    let mut events = Vec::new();
    let mut recovered_count = 0usize;
    for controller in &controllers {
        let mut statement = tx.prepare(
            "SELECT id, status, attempt_count, lease_token FROM hive_runs
             WHERE controller_id = ?1 AND status IN ('leased', 'running')
               AND lease_expires_at <= ?2",
        )?;
        let expired = statement
            .query_map(params![controller.id, now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (run_id, previous_status, attempt_no, lease_token) in expired {
            if previous_status == "leased" {
                let session = require_owned_session(tx, actor, &controller.session_id)?;
                freeze_session_model_into_open_runs(tx, &session, now)?;
            }
            let (target_status, reason, event_type) = if previous_status == "leased" {
                (
                    "queued",
                    "worker lease expired before execution; requeued",
                    "run_lease_requeued",
                )
            } else {
                (
                    "recovery_required",
                    "worker lease expired; side effects may be uncertain",
                    "recovery_required",
                )
            };
            tx.execute(
                "UPDATE hive_runs SET status = ?2, lease_owner = NULL,
                     lease_token = NULL, lease_epoch = NULL, lease_expires_at = NULL,
                     heartbeat_at = NULL, last_error = ?3,
                     updated_at = ?4 WHERE id = ?1 AND status = ?5",
                params![run_id, target_status, reason, now, previous_status],
            )?;
            if target_status == "recovery_required" {
                tx.execute(
                    "UPDATE hive_control_outbox
                     SET status = 'discarded', last_error = ?2, updated_at = ?3
                     WHERE run_id = ?1 AND status = 'pending'",
                    params![run_id, reason, now],
                )?;
            }
            align_run_projection(tx, &run_id, target_status, now)?;
            if let Some(lease_token) = lease_token {
                tx.execute(
                    "UPDATE hive_run_attempts SET finished_at = ?4, outcome = 'abandoned',
                         error = ?5
                     WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3",
                    params![run_id, attempt_no, lease_token, now, reason],
                )?;
            }
            let dedupe_key = format!("transition:{run_id}:{attempt_no}:{target_status}");
            events.push(append_event(
                tx,
                controller,
                event_type,
                Some(&run_id),
                None,
                Some(&dedupe_key),
                serde_json::json!({
                    "run_id": run_id,
                    "reason": reason
                }),
                now,
            )?);
            recovered_count += 1;
        }
    }
    Ok(Mutation {
        response: ResponsePayload::Recover(RecoverResponse { recovered_count }),
        resource_id: session_id.map(ToOwned::to_owned),
        events,
    })
}

struct ParsedScheduleDefinition {
    title: String,
    summary: String,
    objective: String,
    recurrence: RecurrenceV1,
    timezone: String,
    dst_policy: DstPolicy,
    next_fire_at: String,
    priority: i32,
    project_dir: Option<String>,
    model: Option<String>,
    model_key: Option<ModelKey>,
    model_catalog_revision: Option<String>,
    model_was_explicit: bool,
    crew_slug: Option<String>,
    misfire: MisfireConfig,
    overlap_policy: OverlapPolicy,
    retry: RetryPolicy,
}

fn parse_schedule_definition(
    definition: ScheduleDefinition,
    now: &str,
) -> Result<ParsedScheduleDefinition, RuntimeStoreError> {
    fn required(value: String, field: &str, max_bytes: usize) -> Result<String, RuntimeStoreError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(RuntimeStoreError::Invalid(format!(
                "schedule {field} must not be empty"
            )));
        }
        if value.len() > max_bytes {
            return Err(RuntimeStoreError::Invalid(format!(
                "schedule {field} exceeds {max_bytes} bytes"
            )));
        }
        Ok(value.to_string())
    }

    let model_was_explicit = definition
        .model
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || definition.model_key.is_some();
    let title = required(definition.title, "title", 512)?;
    let objective = required(definition.objective, "objective", 64 * 1024)?;
    let summary = definition.summary.trim().to_string();
    if summary.len() > 8 * 1024 {
        return Err(RuntimeStoreError::Invalid(
            "schedule summary exceeds 8192 bytes".into(),
        ));
    }
    let timezone = required(definition.timezone, "timezone", 128)?;
    let timezone_parsed =
        parse_timezone(&timezone).map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?;
    let recurrence: RecurrenceV1 = serde_json::from_value(definition.recurrence)
        .map_err(|error| RuntimeStoreError::Invalid(format!("invalid recurrence: {error}")))?;
    recurrence
        .validate()
        .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?;
    let dst_policy: DstPolicy = serde_json::from_value(definition.dst_policy)
        .map_err(|error| RuntimeStoreError::Invalid(format!("invalid DST policy: {error}")))?;
    let now_instant = mitsuro_core::hive::parse_utc_timestamp(now)
        .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?;
    let next_fire_at = recurrence
        .next_after(timezone_parsed, now_instant, dst_policy)
        .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?
        .ok_or_else(|| {
            RuntimeStoreError::Invalid("schedule has no occurrence after the current time".into())
        })?;
    let misfire: MisfireConfig = serde_json::from_value(definition.misfire)
        .map_err(|error| RuntimeStoreError::Invalid(format!("invalid misfire policy: {error}")))?;
    if misfire.catch_up_limit > 10_000 {
        return Err(RuntimeStoreError::Invalid(
            "misfire catch_up_limit exceeds 10000".into(),
        ));
    }
    let overlap_policy = match definition.overlap_policy.as_str() {
        "skip" => OverlapPolicy::Skip,
        "queue_one" => OverlapPolicy::QueueOne,
        "allow" => OverlapPolicy::Allow,
        _ => {
            return Err(RuntimeStoreError::Invalid(
                "overlap_policy must be skip, queue_one, or allow".into(),
            ));
        }
    };
    let retry: RetryPolicy = serde_json::from_value(definition.retry)
        .map_err(|error| RuntimeStoreError::Invalid(format!("invalid retry policy: {error}")))?;
    if retry.max_attempts == 0
        || retry.max_attempts > MAX_RETRY_ATTEMPTS
        || retry.base_delay_secs == 0
        || retry.max_delay_secs < retry.base_delay_secs
        || retry.max_delay_secs > MAX_RETRY_DELAY_SECS
    {
        return Err(RuntimeStoreError::Invalid(
            format!(
                "retry policy requires 1..={MAX_RETRY_ATTEMPTS} attempts, a nonzero base delay, and base <= max <= {MAX_RETRY_DELAY_SECS} seconds"
            ),
        ));
    }
    let crew_slug = definition
        .crew_slug
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if crew_slug
        .as_deref()
        .is_some_and(|slug| !is_valid_crew_slug(slug))
    {
        return Err(RuntimeStoreError::Invalid("crew slug is invalid".into()));
    }

    let project_dir = definition
        .project_dir
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if project_dir
        .as_deref()
        .is_some_and(|value| value.len() > 4096 || value.as_bytes().contains(&0))
    {
        return Err(RuntimeStoreError::Invalid(
            "schedule project_dir is invalid or too long".into(),
        ));
    }
    if project_dir
        .as_deref()
        .is_some_and(|value| !Path::new(value).is_absolute())
    {
        return Err(RuntimeStoreError::Invalid(
            "schedule project_dir must be absolute".into(),
        ));
    }
    let (model, model_key, model_catalog_revision) = normalize_model_identity(
        definition.model,
        definition.model_key,
        definition.model_catalog_revision,
        "schedule",
    )?;

    Ok(ParsedScheduleDefinition {
        title,
        summary,
        objective,
        recurrence,
        timezone,
        dst_policy,
        next_fire_at: canonical_timestamp(next_fire_at),
        priority: definition.priority,
        project_dir,
        model,
        model_key,
        model_catalog_revision,
        model_was_explicit,
        crew_slug,
        misfire,
        overlap_policy,
        retry,
    })
}

fn create_recurring_schedule(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    command: CreateScheduleCommand,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, &command.session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let mut definition = parse_schedule_definition(command.definition, now)?;
    definition.project_dir = definition
        .project_dir
        .or_else(|| session.project_dir.clone())
        .or_else(|| session.working_dir.clone());
    if !definition.model_was_explicit {
        require_frozen_session_model(&session)?;
        definition.model.clone_from(&session.model);
        definition.model_key.clone_from(&session.model_key);
        definition
            .model_catalog_revision
            .clone_from(&session.model_catalog_revision);
    }
    if definition.crew_slug.is_none() {
        definition.crew_slug = tx
            .query_row(
                "SELECT crew_slug FROM hive_runtime_state WHERE session_id = ?1",
                [&session.id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
    }
    if definition.project_dir.is_none() {
        return Err(RuntimeStoreError::Invalid(
            "schedule requires an explicit or session-owned workspace".into(),
        ));
    }
    if definition
        .project_dir
        .as_deref()
        .is_some_and(|path| !Path::new(path).is_absolute())
    {
        return Err(RuntimeStoreError::Invalid(
            "schedule project_dir must be absolute".into(),
        ));
    }
    if definition.model.is_none() {
        return Err(RuntimeStoreError::Invalid(
            "schedule requires an explicit or session-persisted model".into(),
        ));
    }
    let schedule_id = uuid::Uuid::new_v4().to_string();
    let recurrence_json = serde_json::to_string(&definition.recurrence)
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    let model_key_json = definition
        .model_key
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    tx.execute(
        "INSERT INTO hive_schedules (
            id, controller_id, title, summary, objective, recurrence_kind,
            recurrence_json, timezone, gap_policy, fold_policy, next_fire_at,
            last_scheduled_for, status, priority, project_dir, model,
            model_key_json, model_catalog_revision, crew_slug,
            misfire_policy, misfire_grace_secs, catch_up_limit, overlap_policy,
            max_attempts, retry_base_secs, retry_max_secs, retry_jitter,
            revision, created_by, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL,
            'enabled', ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
            ?21, ?22, ?23, ?24, ?25, 0, ?26, ?27, ?27
         )",
        params![
            schedule_id,
            controller.id,
            definition.title,
            definition.summary,
            definition.objective,
            definition.recurrence.kind_name(),
            recurrence_json,
            definition.timezone,
            definition.dst_policy.gap.as_str(),
            definition.dst_policy.fold.as_str(),
            definition.next_fire_at,
            definition.priority,
            definition.project_dir,
            definition.model,
            model_key_json,
            definition.model_catalog_revision,
            definition.crew_slug,
            definition.misfire.policy.as_str(),
            definition.misfire.grace_secs,
            definition.misfire.catch_up_limit as u64,
            definition.overlap_policy.as_str(),
            definition.retry.max_attempts,
            definition.retry.base_delay_secs,
            definition.retry.max_delay_secs,
            definition.retry.jitter.as_str(),
            actor.user_id.as_deref().unwrap_or("local"),
            now,
        ],
    )?;
    let event = append_event(
        tx,
        &controller,
        "schedule_created",
        None,
        Some(&schedule_id),
        Some(&format!("schedule:{schedule_id}:revision:0")),
        serde_json::json!({"schedule_id": schedule_id, "revision": 0}),
        now,
    )?;
    Ok(Mutation {
        response: ResponsePayload::Schedule(ScheduleResponse {
            schedule_id: schedule_id.clone(),
            revision: 0,
            status: "enabled".into(),
        }),
        resource_id: Some(schedule_id),
        events: vec![event],
    })
}

fn replace_recurring_schedule(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    command: ReplaceScheduleCommand,
) -> Result<Mutation, RuntimeStoreError> {
    validate_bounded_field(&command.schedule_id, "schedule id", 256)?;
    let session = require_owned_session(tx, actor, &command.session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let mut definition = parse_schedule_definition(command.definition, now)?;
    definition.project_dir = definition
        .project_dir
        .or_else(|| session.project_dir.clone())
        .or_else(|| session.working_dir.clone());
    if !definition.model_was_explicit {
        require_frozen_session_model(&session)?;
        definition.model.clone_from(&session.model);
        definition.model_key.clone_from(&session.model_key);
        definition
            .model_catalog_revision
            .clone_from(&session.model_catalog_revision);
    }
    if definition.project_dir.is_none() {
        return Err(RuntimeStoreError::Invalid(
            "schedule requires an explicit or session-owned workspace".into(),
        ));
    }
    if definition
        .project_dir
        .as_deref()
        .is_some_and(|path| !Path::new(path).is_absolute())
    {
        return Err(RuntimeStoreError::Invalid(
            "schedule project_dir must be absolute".into(),
        ));
    }
    if definition.model.is_none() {
        return Err(RuntimeStoreError::Invalid(
            "schedule requires an explicit or session-persisted model".into(),
        ));
    }
    let current = tx
        .query_row(
            "SELECT status, revision FROM hive_schedules
             WHERE id = ?1 AND controller_id = ?2",
            params![command.schedule_id, controller.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((status, revision)) = current else {
        return Err(RuntimeStoreError::NotFound("schedule not found".into()));
    };
    if revision < 0 || revision as u64 != command.expected_revision {
        return Err(RuntimeStoreError::RevisionConflict(format!(
            "schedule revision is {revision}, not {}",
            command.expected_revision
        )));
    }
    if matches!(status.as_str(), "completed" | "cancelled") {
        return Err(RuntimeStoreError::StateConflict(format!(
            "{status} schedules cannot be replaced"
        )));
    }
    let recurrence_json = serde_json::to_string(&definition.recurrence)
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    let model_key_json = definition
        .model_key
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    let changed = tx.execute(
        "UPDATE hive_schedules SET
            title = ?3, summary = ?4, objective = ?5, recurrence_kind = ?6,
            recurrence_json = ?7, timezone = ?8, gap_policy = ?9,
            fold_policy = ?10, next_fire_at = ?11, priority = ?12,
            project_dir = ?13, model = ?14, model_key_json = ?15,
            model_catalog_revision = ?16, crew_slug = ?17,
            misfire_policy = ?18, misfire_grace_secs = ?19,
            catch_up_limit = ?20, overlap_policy = ?21, max_attempts = ?22,
            retry_base_secs = ?23, retry_max_secs = ?24, retry_jitter = ?25,
            revision = revision + 1, updated_at = ?26
         WHERE id = ?1 AND controller_id = ?2 AND revision = ?27",
        params![
            command.schedule_id,
            controller.id,
            definition.title,
            definition.summary,
            definition.objective,
            definition.recurrence.kind_name(),
            recurrence_json,
            definition.timezone,
            definition.dst_policy.gap.as_str(),
            definition.dst_policy.fold.as_str(),
            definition.next_fire_at,
            definition.priority,
            definition.project_dir,
            definition.model,
            model_key_json,
            definition.model_catalog_revision,
            definition.crew_slug,
            definition.misfire.policy.as_str(),
            definition.misfire.grace_secs,
            definition.misfire.catch_up_limit as u64,
            definition.overlap_policy.as_str(),
            definition.retry.max_attempts,
            definition.retry.base_delay_secs,
            definition.retry.max_delay_secs,
            definition.retry.jitter.as_str(),
            now,
            command.expected_revision,
        ],
    )?;
    if changed != 1 {
        return Err(RuntimeStoreError::RevisionConflict(
            "schedule changed concurrently".into(),
        ));
    }
    let revision = command.expected_revision.saturating_add(1);
    let event = append_event(
        tx,
        &controller,
        "schedule_updated",
        None,
        Some(&command.schedule_id),
        Some(&format!(
            "schedule:{}:revision:{revision}",
            command.schedule_id
        )),
        serde_json::json!({"schedule_id": command.schedule_id, "revision": revision}),
        now,
    )?;
    Ok(Mutation {
        response: ResponsePayload::Schedule(ScheduleResponse {
            schedule_id: command.schedule_id.clone(),
            revision,
            status,
        }),
        resource_id: Some(command.schedule_id),
        events: vec![event],
    })
}

fn set_schedule_status(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    command: SetScheduleStatusCommand,
) -> Result<Mutation, RuntimeStoreError> {
    validate_bounded_field(&command.schedule_id, "schedule id", 256)?;
    let session = require_owned_session(tx, actor, &command.session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let current = tx
        .query_row(
            "SELECT status, revision FROM hive_schedules
             WHERE id = ?1 AND controller_id = ?2",
            params![command.schedule_id, controller.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((current_status, revision)) = current else {
        return Err(RuntimeStoreError::NotFound("schedule not found".into()));
    };
    if revision < 0 || revision as u64 != command.expected_revision {
        return Err(RuntimeStoreError::RevisionConflict(format!(
            "schedule revision is {revision}, not {}",
            command.expected_revision
        )));
    }
    let allowed = current_status == command.status
        || matches!(
            (current_status.as_str(), command.status.as_str()),
            ("enabled", "paused" | "completed" | "cancelled") | ("paused", "enabled" | "cancelled")
        );
    if !allowed {
        return Err(RuntimeStoreError::StateConflict(format!(
            "illegal schedule transition from {current_status} to {}",
            command.status
        )));
    }
    if !matches!(
        command.status.as_str(),
        "enabled" | "paused" | "completed" | "cancelled"
    ) {
        return Err(RuntimeStoreError::Invalid("invalid schedule status".into()));
    }
    let changed = tx.execute(
        "UPDATE hive_schedules SET status = ?3, revision = revision + 1,
             updated_at = ?4
         WHERE id = ?1 AND controller_id = ?2 AND revision = ?5 AND status = ?6",
        params![
            command.schedule_id,
            controller.id,
            command.status,
            now,
            command.expected_revision,
            current_status,
        ],
    )?;
    if changed != 1 {
        return Err(RuntimeStoreError::RevisionConflict(
            "schedule changed concurrently".into(),
        ));
    }
    let revision = command.expected_revision.saturating_add(1);
    let event = append_event(
        tx,
        &controller,
        "schedule_status_changed",
        None,
        Some(&command.schedule_id),
        Some(&format!(
            "schedule:{}:revision:{revision}",
            command.schedule_id
        )),
        serde_json::json!({
            "schedule_id": command.schedule_id,
            "previous": current_status,
            "current": command.status,
            "revision": revision,
        }),
        now,
    )?;
    Ok(Mutation {
        response: ResponsePayload::Schedule(ScheduleResponse {
            schedule_id: command.schedule_id.clone(),
            revision,
            status: command.status,
        }),
        resource_id: Some(command.schedule_id),
        events: vec![event],
    })
}

fn extension(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    command: mitsuro_hive_protocol::ExtensionCommand,
) -> Result<Mutation, RuntimeStoreError> {
    validate_bounded_field(&command.name, "extension name", 256)?;
    let payload_bytes = serde_json::to_vec(&command.payload).map_err(|error| {
        RuntimeStoreError::Invalid(format!("invalid extension payload: {error}"))
    })?;
    if payload_bytes.len() > 256 * 1024 {
        return Err(RuntimeStoreError::Invalid(
            "extension payload exceeds 262144 bytes".into(),
        ));
    }
    let session_id = command
        .payload
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeStoreError::Invalid("extension requires session_id".into()))?
        .to_string();
    let session = require_owned_session(tx, actor, &session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let event = append_event(
        tx,
        &controller,
        "extension_received",
        None,
        None,
        None,
        serde_json::json!({
            "name": &command.name,
            "payload_kind": json_value_kind(&command.payload),
            "payload_bytes": payload_bytes.len(),
            "top_level_fields": command.payload.as_object().map_or(0, serde_json::Map::len),
        }),
        now,
    )?;
    Ok(Mutation {
        response: ResponsePayload::Extension(ExtensionResponse {
            name: command.name,
            payload: serde_json::json!({"accepted": true}),
        }),
        resource_id: Some(session_id),
        events: vec![event],
    })
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_run(
    tx: &Transaction<'_>,
    id: &str,
    controller_id: &str,
    session_id: Option<&str>,
    schedule_id: Option<&str>,
    occurrence_id: Option<&str>,
    kind: &str,
    objective: &str,
    config: Value,
    priority: i32,
    available_at: &str,
    max_attempts: u32,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    let config_json = serde_json::to_string(&config)
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    tx.execute(
        "INSERT INTO hive_runs (
            id, controller_id, session_id, schedule_id, occurrence_id, kind,
            objective, config_json, status, priority, concurrency_key,
            scheduled_for, available_at, wake_at, attempt_count, max_attempts,
            lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
            last_stop_reason, last_error, outcome_json, created_at, started_at,
            finished_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', ?9, NULL,
            ?10, ?10, NULL, 0, ?11, NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, ?12, NULL, NULL, ?12
         )",
        params![
            id,
            controller_id,
            session_id,
            schedule_id,
            occurrence_id,
            kind,
            objective,
            config_json,
            priority,
            available_at,
            max_attempts,
            now,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_canonical_user_message(
    tx: &Transaction<'_>,
    session_id: &str,
    message: &str,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    let content = serde_json::to_string(&vec![Content::Text {
        text: message.to_string(),
    }])
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    tx.execute(
        "INSERT INTO messages (session_id, role, content, created_at)
         VALUES (?1, 'user', ?2, ?3)",
        params![session_id, content, now],
    )?;
    let message_id = tx.last_insert_rowid();
    if let Some(body) = canonical_text(&content) {
        let mut hash_material = Vec::with_capacity("user".len() + 1 + body.len());
        hash_material.extend_from_slice(b"user");
        hash_material.push(0);
        hash_material.extend_from_slice(body.as_bytes());
        tx.execute(
            "INSERT INTO conversation_episodes (
                session_id, source_message_id, role, body, content_hash, occurred_at
             ) VALUES (?1, ?2, 'user', ?3, ?4, ?5)
             ON CONFLICT(session_id, source_message_id) DO UPDATE SET
                role = excluded.role, body = excluded.body,
                content_hash = excluded.content_hash, occurred_at = excluded.occurred_at",
            params![
                session_id,
                message_id,
                body,
                hash_request_bytes(hash_material),
                now
            ],
        )?;
    }
    tx.execute(
        "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
        params![session_id, now],
    )?;
    Ok(())
}

fn insert_pending_user_message(
    tx: &Transaction<'_>,
    session_id: &str,
    pending_id: &str,
    message: &str,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    let content = serde_json::to_string(&vec![Content::Text {
        text: message.to_string(),
    }])
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    insert_pending_user_content(tx, session_id, pending_id, &content, now)
}

fn insert_pending_user_content(
    tx: &Transaction<'_>,
    session_id: &str,
    pending_id: &str,
    content_json: &str,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    let role = format!("pending_user:{pending_id}");
    tx.execute(
        "INSERT INTO messages (session_id, role, content, created_at)
         SELECT ?1, ?2, ?3, ?4
         WHERE NOT EXISTS (
             SELECT 1 FROM messages WHERE session_id = ?1 AND role = ?2
         )",
        params![session_id, role, content_json, now],
    )?;
    tx.execute(
        "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
        params![session_id, now],
    )?;
    Ok(())
}

fn canonical_text(content_json: &str) -> Option<String> {
    let joined = serde_json::from_str::<Vec<Content>>(content_json)
        .ok()?
        .into_iter()
        .filter_map(|content| match content {
            Content::Text { text } => Some(text),
            _ => None,
        })
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if joined.is_empty() {
        return None;
    }
    const MAX_EPISODE_BYTES: usize = 16 * 1024;
    if joined.len() <= MAX_EPISODE_BYTES {
        return Some(joined);
    }
    let mut boundary = MAX_EPISODE_BYTES;
    while !joined.is_char_boundary(boundary) {
        boundary -= 1;
    }
    Some(joined[..boundary].to_string())
}

fn session_mutation(
    session_id: &str,
    status: &str,
    resource_id: Option<String>,
    events: Vec<PersistedEvent>,
) -> Mutation {
    Mutation {
        response: ResponsePayload::Session(SessionResponse {
            session_id: session_id.to_string(),
            state: serde_json::json!({"status": status}),
        }),
        resource_id: resource_id.or_else(|| Some(session_id.to_string())),
        events,
    }
}

pub(super) fn ack(message: &str) -> ResponsePayload {
    ResponsePayload::Ack(AckResponse {
        accepted: true,
        message: Some(message.to_string()),
    })
}

fn priority_value(priority: &str) -> Result<i32, RuntimeStoreError> {
    match priority {
        "low" => Ok(-10),
        "normal" => Ok(0),
        "high" => Ok(50),
        "critical" => Ok(100),
        value => Err(RuntimeStoreError::Invalid(format!(
            "unsupported priority: {value}"
        ))),
    }
}

fn priority_name(priority: i32) -> &'static str {
    if priority < 0 {
        "low"
    } else if priority >= 50 {
        "high"
    } else {
        "normal"
    }
}

fn bounded_title(task: &str) -> String {
    task.chars().take(80).collect()
}

fn pending_message_id(actor: &Actor, session_id: &str, idempotency_key: &str) -> String {
    crate::legacy_identity::pending_message_id(
        actor.user_id.as_deref().unwrap_or("local"),
        &actor.client_kind,
        session_id,
        idempotency_key,
    )
}

fn steer_pending_id(
    actor: &Actor,
    session_id: &str,
    idempotency_key: &str,
    requested: Option<&str>,
) -> String {
    requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| pending_message_id(actor, session_id, idempotency_key))
}

fn control_after_commit(
    command: &Command,
    actor: &Actor,
    idempotency_key: &str,
) -> Option<(String, ExecutionControl)> {
    match command {
        Command::StartSession(command) | Command::ResumeSession(command) => {
            Some((command.session_id.clone(), ExecutionControl::Start))
        }
        Command::PauseSession(command) => Some((
            command.session_id.clone(),
            ExecutionControl::Cancel {
                reason: "paused by user".into(),
            },
        )),
        Command::SendMessage(command) => Some((
            command.session_id.clone(),
            ExecutionControl::Steer {
                pending_id: Some(pending_message_id(
                    actor,
                    &command.session_id,
                    idempotency_key,
                )),
                content: serde_json::json!([{
                    "type": "text",
                    "text": command.message,
                }]),
            },
        )),
        Command::Steer(command) => Some((
            command.session_id.clone(),
            ExecutionControl::Steer {
                pending_id: Some(steer_pending_id(
                    actor,
                    &command.session_id,
                    idempotency_key,
                    command.pending_id.as_deref(),
                )),
                content: command.content.clone(),
            },
        )),
        // Tool approvals are scheduler-delivered from hive_control_outbox.
        // Returning success means the decision is durable, even if the host
        // input channel is between registrations at commit time.
        Command::ToolApproval(_) => None,
        Command::UserResponse(command) => Some((
            command.session_id.clone(),
            ExecutionControl::UserResponse {
                run_id: command.run_id.clone(),
                pending_id: pending_message_id(actor, &command.session_id, idempotency_key),
                tool_call_id: command.tool_call_id.clone(),
                response: command.response.clone(),
            },
        )),
        Command::CancelSession(command) => Some((
            command.session_id.clone(),
            ExecutionControl::Cancel {
                reason: "cancelled by user".into(),
            },
        )),
        // Delete is accepted only after active workers are quiescent, so no
        // best-effort post-commit cancellation is necessary or safe.
        Command::DeleteSession(_) => None,
        _ => None,
    }
}

fn replay_gap_event(session_id: &str, requested_after: i64, earliest: i64) -> EventEnvelope {
    EventEnvelope {
        version: ProtocolVersion::CURRENT,
        session_id: Some(session_id.to_string()),
        run_id: None,
        sequence: None,
        emitted_at_unix_ms: unix_time_millis(),
        event: HiveEvent::ReplayGap(ReplayGapEvent {
            requested_after,
            earliest_available: earliest,
        }),
    }
}

fn lagged_event(
    session_id: &str,
    skipped: u64,
    resume_after_sequence: Option<i64>,
) -> EventEnvelope {
    EventEnvelope {
        version: ProtocolVersion::CURRENT,
        session_id: Some(session_id.to_string()),
        run_id: None,
        sequence: None,
        emitted_at_unix_ms: unix_time_millis(),
        event: HiveEvent::Lagged(LaggedEvent {
            skipped,
            resume_after_sequence,
        }),
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use mitsuro_hive_protocol::UserResponseCommand;

    #[test]
    fn user_response_control_preserves_exact_run_and_pending_identity() {
        let actor = Actor::local("test");
        let command = Command::UserResponse(UserResponseCommand {
            session_id: "session-1".into(),
            run_id: "run-a".into(),
            tool_call_id: "question-1".into(),
            response: "continue".into(),
        });

        let (session_id, control) =
            control_after_commit(&command, &actor, "response-key").expect("control");
        assert_eq!(session_id, "session-1");
        assert!(matches!(
            control,
            ExecutionControl::UserResponse {
                run_id,
                pending_id,
                tool_call_id,
                response,
            } if run_id == "run-a"
                && !pending_id.trim().is_empty()
                && tool_call_id == "question-1"
                && response == "continue"
        ));
    }
}
