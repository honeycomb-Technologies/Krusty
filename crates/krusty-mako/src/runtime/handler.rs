use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use krusty_core::mako::{canonical_timestamp, RecurrenceV1, RetryPolicy};
use krusty_core::storage::{hash_request_bytes, is_valid_crew_slug};
use krusty_core::Content;
use krusty_mako_protocol::{
    unix_time_millis, AckResponse, Actor, Command, DispatchCommand, DispatchResponse,
    EventEnvelope, ExtensionResponse, LaggedEvent, MakoEvent, ProtocolErrorPayload,
    ProtocolVersion, RecoverResponse, ReplayGapEvent, ResponsePayload, SessionResponse,
    SubscribeCommand, SubscriptionAccepted,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio::task::JoinHandle;

use crate::{CommandContext, CommandHandler, HandlerReply, HandlerResult};

use super::backend::{ExecutionBackend, ExecutionControl};
use super::config::MakoRuntimeConfig;
use super::events::EventHub;
use super::persistence::{
    append_event, get_or_create_controller, request_hash, require_owned_session,
    unix_millis_to_utc, ControllerRecord, Mutation, MutationOutcome, PersistedEvent,
    RuntimePersistence, RuntimeStoreError,
};
use super::pump;

pub(crate) struct RuntimeShared {
    pub(crate) config: MakoRuntimeConfig,
    pub(crate) instance_id: String,
    pub(crate) persistence: RuntimePersistence,
    pub(crate) backend: Arc<dyn ExecutionBackend>,
    pub(crate) events: EventHub,
    pub(crate) mutation_gate: Mutex<()>,
}

pub struct DurableMakoCommandHandler {
    pub(crate) shared: Arc<RuntimeShared>,
}

pub struct MakoRuntimeHandle {
    handler: Arc<DurableMakoCommandHandler>,
    shutdown_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl MakoRuntimeHandle {
    pub fn handler(&self) -> Arc<DurableMakoCommandHandler> {
        Arc::clone(&self.handler)
    }

    pub async fn shutdown(mut self) {
        self.shutdown_tx.send_replace(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for MakoRuntimeHandle {
    fn drop(&mut self) {
        self.shutdown_tx.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub async fn start_runtime(
    config: MakoRuntimeConfig,
    daemon_instance_id: impl Into<String>,
    backend: Arc<dyn ExecutionBackend>,
) -> anyhow::Result<MakoRuntimeHandle> {
    config.validate()?;
    let persistence = RuntimePersistence::new(config.database_path.clone(), config.idempotency_ttl);
    persistence
        .initialize()
        .await
        .map_err(|error| anyhow::anyhow!(error.protocol().message))?;
    let instance_label = daemon_instance_id.into();
    let shared = Arc::new(RuntimeShared {
        instance_id: format!("{instance_label}:boot:{}", uuid::Uuid::new_v4()),
        events: EventHub::new(config.live_event_capacity),
        persistence,
        backend,
        mutation_gate: Mutex::new(()),
        config,
    });
    let handler = Arc::new(DurableMakoCommandHandler {
        shared: Arc::clone(&shared),
    });
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(pump::run(shared, shutdown_rx));
    Ok(MakoRuntimeHandle {
        handler,
        shutdown_tx,
        task: Some(task),
    })
}

#[async_trait]
impl CommandHandler for DurableMakoCommandHandler {
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
        let _gate = self.shared.mutation_gate.lock().await;
        let mut outcome = self
            .mutate(actor, idempotency_key, operation, hash, command)
            .await
            .map_err(RuntimeStoreError::protocol)?;
        outcome.events.sort_by_key(|event| event.sequence);
        for event in &outcome.events {
            self.shared.events.publish(event.envelope());
        }
        drop(_gate);

        if !outcome.replayed {
            if let Some((session_id, control)) = control {
                if let Err(error) = self.shared.backend.control(&session_id, control).await {
                    tracing::warn!(session_id, error = %error, "Mako backend control delivery failed after durable acceptance");
                }
            }
        }
        Ok(HandlerReply::Response(outcome.response))
    }

    async fn runtime_stats(&self) -> Value {
        self.shared
            .persistence
            .stats()
            .await
            .unwrap_or_else(|error| serde_json::json!({"error": error.protocol().code}))
    }
}

impl DurableMakoCommandHandler {
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
                    Command::Steer(command) => control_event(
                        tx,
                        actor,
                        now,
                        &command.session_id,
                        "steer_received",
                        serde_json::json!({
                            "pending_id": command.pending_id,
                            "content": command.content,
                        }),
                        false,
                    ),
                    Command::ToolApproval(command) => control_event(
                        tx,
                        actor,
                        now,
                        &command.session_id,
                        "tool_approval_received",
                        serde_json::json!({
                            "tool_call_id": command.tool_call_id,
                            "approved": command.approved,
                        }),
                        false,
                    ),
                    Command::UserResponse(command) => control_event(
                        tx,
                        actor,
                        now,
                        &command.session_id,
                        "user_response_received",
                        serde_json::json!({
                            "tool_call_id": command.tool_call_id,
                            "response": command.response,
                        }),
                        true,
                    ),
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
            for event in snapshot.events {
                if sender.send(event.envelope()).await.is_err() {
                    return;
                }
            }
            let mut last_sequence = snapshot.high_water.unwrap_or(requested_after);
            loop {
                match live.recv().await {
                    Ok(event) => {
                        let Some(sequence) = event.sequence else {
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
                                    last_sequence = last_sequence.max(event.sequence);
                                    if sender.send(event.envelope()).await.is_err() {
                                        return;
                                    }
                                }
                                last_sequence = catchup.high_water.unwrap_or(last_sequence);
                            }
                            Err(error) => {
                                tracing::warn!(error = ?error, "Mako subscription catch-up failed");
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
            id, title, created_at, updated_at, model, working_dir, project_dir,
            workspace_mode, session_type, user_id, permission_mode
         ) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 'selected', 'mako', ?7, 'autonomous')",
        params![
            session_id,
            bounded_title(&command.task),
            now,
            command.model,
            command.working_dir,
            project_dir,
            actor.user_id,
        ],
    )?;
    insert_canonical_user_message(tx, &session_id, &command.task, now)?;
    tx.execute(
        "INSERT INTO mako_controllers (
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
            "model": command.model,
            "crew_slug": command.crew_slug,
            "retry": RetryPolicy::default(),
        }),
        priority,
        &canonical_timestamp(scheduled_for),
        5,
        now,
    )?;
    tx.execute(
        "INSERT INTO mako_runtime_state (
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

fn start_session(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    tx.execute(
        "UPDATE mako_controllers SET status = 'active', updated_at = ?2 WHERE id = ?1",
        params![controller.id, now],
    )?;
    let existing: Option<(String, String, i64)> = tx
        .query_row(
            "SELECT id, status, attempt_count FROM mako_runs
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
                "UPDATE mako_runs
                 SET status = 'queued', available_at = ?2, wake_at = NULL,
                     last_stop_reason = NULL, updated_at = ?2
                 WHERE id = ?1 AND status IN ('sleeping', 'retry_wait', 'awaiting_input')",
                params![existing, now],
            )?;
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
    if reason.trim().is_empty() {
        return Err(RuntimeStoreError::Invalid(
            "schedule reason is empty".into(),
        ));
    }
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let wake_at = unix_millis_to_utc(wake_at_unix_ms)?;
    let schedule_id = uuid::Uuid::new_v4().to_string();
    let recurrence = serde_json::to_string(&RecurrenceV1::Once { at: wake_at })
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    let wake_at = canonical_timestamp(wake_at);
    tx.execute(
        "INSERT INTO mako_schedules (
            id, controller_id, title, summary, objective, recurrence_kind,
            recurrence_json, timezone, gap_policy, fold_policy, next_fire_at,
            last_scheduled_for, status, priority, project_dir, model, crew_slug,
            misfire_policy, misfire_grace_secs, catch_up_limit, overlap_policy,
            max_attempts, retry_base_secs, retry_max_secs, retry_jitter,
            revision, created_by, created_at, updated_at
         ) SELECT ?1, ?2, ?3, ?3, ?3, 'once', ?4, ?5, 'shift_forward', 'first',
                  ?6, NULL, 'enabled', 0, s.project_dir, s.model, rs.crew_slug,
                  'fire_once', 300, 1, 'queue_one', 5, 15, 900, 'full', 0,
                  ?7, ?8, ?8
           FROM sessions s
           LEFT JOIN mako_runtime_state rs ON rs.session_id = s.id
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
    let mut controller = get_or_create_controller(tx, &session, now)?;
    let previous = controller.status.clone();
    tx.execute(
        "UPDATE mako_controllers SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![controller.id, status, now],
    )?;
    controller.status = status.to_string();
    let active_runs = if status == "paused" {
        let mut statement = tx.prepare(
            "SELECT id, status, attempt_count, lease_token FROM mako_runs
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
            "UPDATE mako_runs
             SET status = ?2, lease_owner = NULL, lease_token = NULL,
                 lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
                 last_stop_reason = 'paused by user', last_error = ?3, updated_at = ?4
             WHERE id = ?1 AND status = ?5",
            params![run_id, target, reason, now, run_status],
        )?;
        if let Some(lease_token) = lease_token {
            tx.execute(
                "UPDATE mako_run_attempts
                 SET finished_at = ?4, outcome = 'abandoned', stop_reason = 'paused by user',
                     error = ?5
                 WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3
                   AND finished_at IS NULL",
                params![run_id, attempt_no, lease_token, now, reason],
            )?;
        }
        if target == "recovery_required" {
            tx.execute(
                "UPDATE mako_schedule_occurrences SET status = 'failed',
                     decision_reason = ?2, updated_at = ?3
                 WHERE run_id = ?1 AND status = 'running'",
                params![run_id, reason, now],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO mako_runtime_state (session_id, status, updated_at)
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
            "SELECT id, attempt_count, schedule_id FROM mako_runs
             WHERE controller_id = ?1
               AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        )?;
        let rows = statement
            .query_map([&controller.id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.execute(
        "UPDATE mako_run_attempts
         SET finished_at = ?2, outcome = 'cancelled', stop_reason = 'cancelled by user'
         WHERE run_id IN (
             SELECT id FROM mako_runs WHERE controller_id = ?1
               AND status IN ('leased', 'running')
         ) AND finished_at IS NULL",
        params![controller.id, now],
    )?;
    tx.execute(
        "UPDATE mako_runs
         SET status = 'cancelled', lease_owner = NULL, lease_token = NULL,
             lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
             wake_at = NULL, last_stop_reason = 'cancelled by user',
             finished_at = ?2, updated_at = ?2
         WHERE controller_id = ?1
           AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        params![controller.id, now],
    )?;
    tx.execute(
        "UPDATE mako_schedules SET status = 'cancelled', revision = revision + 1,
                 updated_at = ?2
         WHERE controller_id = ?1 AND status IN ('enabled', 'paused')",
        params![controller.id, now],
    )?;
    tx.execute(
        "UPDATE mako_controllers SET status = 'paused', updated_at = ?2 WHERE id = ?1",
        params![controller.id, now],
    )?;
    tx.execute(
        "INSERT INTO mako_runtime_state (session_id, status, updated_at)
         VALUES (?1, 'cancelled', ?2)
         ON CONFLICT(session_id) DO UPDATE SET status = 'cancelled', updated_at = excluded.updated_at",
        params![session_id, now],
    )?;
    let mut events = Vec::with_capacity(cancellable.len() + 1);
    for (run_id, attempt_count, schedule_id) in cancellable {
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
    let event = append_event(
        tx,
        &controller,
        "session_deleted",
        None,
        None,
        None,
        serde_json::json!({"session_id": session_id}),
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
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let active_run_id = tx
        .query_row(
            "SELECT id FROM mako_runs
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
        serde_json::json!({"message": message}),
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

fn control_event(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    session_id: &str,
    event_type: &str,
    payload: Value,
    resume_waiting: bool,
) -> Result<Mutation, RuntimeStoreError> {
    let session = require_owned_session(tx, actor, session_id)?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let event = append_event(tx, &controller, event_type, None, None, None, payload, now)?;
    let mut events = vec![event];
    if resume_waiting {
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
                    "reason": "user response received"
                }),
                now,
            )?);
        }
    }
    Ok(Mutation {
        response: ack("control accepted"),
        resource_id: Some(session_id.to_string()),
        events,
    })
}

fn resume_waiting_run(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    now: &str,
) -> Result<Option<(String, String, i64)>, RuntimeStoreError> {
    let waiting = tx
        .query_row(
            "SELECT id, status, attempt_count FROM mako_runs
             WHERE controller_id = ?1 AND status IN ('sleeping', 'awaiting_input')
             ORDER BY updated_at DESC, created_at DESC LIMIT 1",
            [&controller.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((run_id, previous_status, attempt_count)) = waiting else {
        return Ok(None);
    };
    let changed = tx.execute(
        "UPDATE mako_runs
         SET status = 'queued', available_at = ?2, wake_at = NULL,
             last_stop_reason = NULL, updated_at = ?2
         WHERE id = ?1 AND status IN ('sleeping', 'awaiting_input')",
        params![run_id, now],
    )?;
    if changed != 1 {
        return Ok(None);
    }
    tx.execute(
        "UPDATE mako_schedule_occurrences SET status = 'queued', updated_at = ?2
         WHERE run_id = ?1 AND status = 'running'",
        params![run_id, now],
    )?;
    tx.execute(
        "INSERT INTO mako_runtime_state (session_id, status, current_run_id, updated_at)
         VALUES (?1, 'idle', ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET status = 'idle',
             current_run_id = excluded.current_run_id, updated_at = excluded.updated_at",
        params![controller.session_id, run_id, now],
    )?;
    Ok(Some((run_id, previous_status, attempt_count)))
}

fn queue_message_turn_if_idle(
    tx: &Transaction<'_>,
    session: &super::persistence::OwnedSession,
    controller: &ControllerRecord,
    message: &str,
    now: &str,
) -> Result<Option<PersistedEvent>, RuntimeStoreError> {
    let unfinished: i64 = tx.query_row(
        "SELECT COUNT(*) FROM mako_runs
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
            "SELECT priority, crew_slug FROM mako_runtime_state WHERE session_id = ?1",
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
        "INSERT INTO mako_runtime_state (session_id, status, current_run_id, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(session_id) DO UPDATE SET current_run_id = excluded.current_run_id,
             status = CASE WHEN mako_runtime_state.status = 'paused' THEN 'paused' ELSE excluded.status END,
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
        "UPDATE mako_runs SET priority = ?2, updated_at = ?3
         WHERE controller_id = ?1 AND status IN ('queued', 'sleeping', 'retry_wait')",
        params![controller.id, value, now],
    )?;
    tx.execute(
        "INSERT INTO mako_runtime_state (session_id, status, priority, updated_at)
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
        "UPDATE mako_schedules SET crew_slug = ?2, revision = revision + 1, updated_at = ?3
         WHERE controller_id = ?1 AND status IN ('enabled', 'paused')",
        params![controller.id, crew_slug, now],
    )?;
    tx.execute(
        "UPDATE mako_runs
         SET config_json = json_set(config_json, '$.crew_slug', ?2), updated_at = ?3
         WHERE controller_id = ?1 AND status = 'queued'",
        params![controller.id, crew_slug, now],
    )?;
    tx.execute(
        "INSERT INTO mako_runtime_state (session_id, status, crew_slug, updated_at)
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
             FROM mako_controllers c JOIN sessions s ON s.id = c.session_id
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
            "SELECT id, status, attempt_count, lease_token FROM mako_runs
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
                "UPDATE mako_runs SET status = ?2, lease_owner = NULL,
                     lease_token = NULL, lease_epoch = NULL, lease_expires_at = NULL,
                     heartbeat_at = NULL, last_error = ?3,
                     updated_at = ?4 WHERE id = ?1 AND status = ?5",
                params![run_id, target_status, reason, now, previous_status],
            )?;
            if let Some(lease_token) = lease_token {
                tx.execute(
                    "UPDATE mako_run_attempts SET finished_at = ?4, outcome = 'abandoned',
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

fn extension(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    command: krusty_mako_protocol::ExtensionCommand,
) -> Result<Mutation, RuntimeStoreError> {
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
        serde_json::json!({"name": &command.name, "payload": &command.payload}),
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
        "INSERT INTO mako_runs (
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

fn insert_canonical_user_message(
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
    let role = format!("pending_user:{pending_id}");
    let content = serde_json::to_string(&vec![Content::Text {
        text: message.to_string(),
    }])
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    tx.execute(
        "INSERT INTO messages (session_id, role, content, created_at)
         SELECT ?1, ?2, ?3, ?4
         WHERE NOT EXISTS (
             SELECT 1 FROM messages WHERE session_id = ?1 AND role = ?2
         )",
        params![session_id, role, content, now],
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

fn ack(message: &str) -> ResponsePayload {
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
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!(
            "krusty:mako:pending-message:{}:{}:{}:{}",
            actor.user_id.as_deref().unwrap_or("local"),
            actor.client_kind,
            session_id,
            idempotency_key,
        )
        .as_bytes(),
    )
    .to_string()
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
                pending_id: command.pending_id.clone(),
                content: command.content.clone(),
            },
        )),
        Command::ToolApproval(command) => Some((
            command.session_id.clone(),
            ExecutionControl::ToolApproval {
                tool_call_id: command.tool_call_id.clone(),
                approved: command.approved,
            },
        )),
        Command::UserResponse(command) => Some((
            command.session_id.clone(),
            ExecutionControl::UserResponse {
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
        event: MakoEvent::ReplayGap(ReplayGapEvent {
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
        event: MakoEvent::Lagged(LaggedEvent {
            skipped,
            resume_after_sequence,
        }),
    }
}
