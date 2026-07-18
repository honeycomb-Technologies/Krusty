use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use krusty_core::mako::{canonical_timestamp, parse_utc_timestamp};
use krusty_core::storage::{hash_request_bytes, Database};
use krusty_mako_protocol::{
    unix_time_millis, Actor, DaemonRuntimeStats, EventEnvelope, ExtensionEvent, MakoEvent,
    ProtocolErrorPayload, ProtocolVersion, ResponsePayload, RuntimeEvent,
};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::Serialize;
use serde_json::{Map, Value};

const MAX_CONTROLLER_EVENT_ROWS: i64 = 2_048;
const MAX_CONTROLLER_EVENT_BYTES: i64 = 2 * 1024 * 1024;
const MAX_DURABLE_EVENT_PAYLOAD_BYTES: usize = 8 * 1024;
const MAX_PENDING_INTERACTIONS: i64 = 32;
const RESERVED_RESOLUTION_EVENT_ROWS: i64 = MAX_PENDING_INTERACTIONS;
const RESERVED_RESOLUTION_EVENT_BYTES: i64 = MAX_PENDING_INTERACTIONS * 12 * 1024;
const EVENT_RETENTION_DELETE_BATCH: i64 = 1_024;
const MAINTENANCE_DELETE_BATCH: i64 = 256;
const TERMINAL_OUTBOX_RETENTION_DAYS: i64 = 7;

#[derive(Debug, Clone)]
pub(crate) struct RuntimePersistence {
    database_path: PathBuf,
    idempotency_ttl: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct OwnedSession {
    pub(crate) id: String,
    pub(crate) user_id: Option<String>,
    pub(crate) title: String,
    pub(crate) working_dir: Option<String>,
    pub(crate) project_dir: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) permission_mode: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ControllerRecord {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) status: String,
    pub(crate) timezone: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PersistedEvent {
    pub(crate) controller_id: String,
    pub(crate) session_id: String,
    pub(crate) run_id: Option<String>,
    pub(crate) schedule_id: Option<String>,
    pub(crate) sequence: i64,
    pub(crate) event_type: String,
    pub(crate) payload: Value,
    pub(crate) created_at: String,
}

impl PersistedEvent {
    pub(crate) fn envelope(&self) -> EventEnvelope {
        EventEnvelope {
            version: ProtocolVersion::CURRENT,
            session_id: Some(self.session_id.clone()),
            run_id: self.run_id.clone(),
            sequence: Some(self.sequence),
            emitted_at_unix_ms: parse_utc_timestamp(&self.created_at)
                .map(|value| value.timestamp_millis())
                .unwrap_or_else(|_| unix_time_millis()),
            event: if self.event_type == "agentic_event" {
                MakoEvent::Extension(ExtensionEvent {
                    name: "agentic_event".to_string(),
                    payload: self.payload.clone(),
                })
            } else {
                MakoEvent::Runtime(RuntimeEvent {
                    event_type: self.event_type.clone(),
                    payload: self.payload.clone(),
                })
            },
        }
    }
}

#[derive(Debug)]
pub(crate) struct Mutation {
    pub(crate) response: ResponsePayload,
    pub(crate) resource_id: Option<String>,
    pub(crate) events: Vec<PersistedEvent>,
}

#[derive(Debug)]
pub(crate) struct MutationOutcome {
    pub(crate) response: ResponsePayload,
    pub(crate) events: Vec<PersistedEvent>,
    pub(crate) replayed: bool,
}

#[derive(Debug)]
pub(crate) struct ReplaySnapshot {
    pub(crate) events: Vec<PersistedEvent>,
    pub(crate) requested_after: i64,
    pub(crate) earliest_returned: Option<i64>,
    pub(crate) earliest_available: Option<i64>,
    pub(crate) high_water: Option<i64>,
}

#[derive(Debug)]
pub(crate) enum RuntimeStoreError {
    Ownership,
    NotFound(String),
    Conflict(String),
    RevisionConflict(String),
    StateConflict(String),
    ResourceExhausted(String),
    InProgress,
    Invalid(String),
    Internal(anyhow::Error),
}

impl RuntimeStoreError {
    pub(crate) fn protocol(self) -> ProtocolErrorPayload {
        match self {
            Self::Ownership => ProtocolErrorPayload::new(
                "ownership_denied",
                "session does not belong to the exact authenticated actor",
                false,
            ),
            Self::NotFound(message) => ProtocolErrorPayload::new("not_found", message, false),
            Self::Conflict(message) => {
                ProtocolErrorPayload::new("idempotency_conflict", message, false)
            }
            Self::RevisionConflict(message) => {
                ProtocolErrorPayload::new("revision_conflict", message, false)
            }
            Self::StateConflict(message) => {
                ProtocolErrorPayload::new("state_conflict", message, false)
            }
            Self::ResourceExhausted(message) => {
                ProtocolErrorPayload::new("resource_exhausted", message, false)
            }
            Self::InProgress => ProtocolErrorPayload::new(
                "request_in_progress",
                "an equivalent idempotent request is still in progress",
                true,
            ),
            Self::Invalid(message) => ProtocolErrorPayload::new("invalid_command", message, false),
            Self::Internal(error) => {
                tracing::error!(error = %error, "durable Mako runtime storage failure");
                ProtocolErrorPayload::new(
                    "runtime_storage_error",
                    "durable Mako state could not be updated",
                    true,
                )
            }
        }
    }
}

impl From<anyhow::Error> for RuntimeStoreError {
    fn from(value: anyhow::Error) -> Self {
        Self::Internal(value)
    }
}

impl From<rusqlite::Error> for RuntimeStoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Internal(value.into())
    }
}

impl RuntimePersistence {
    pub(crate) fn new(database_path: PathBuf, idempotency_ttl: Duration) -> Self {
        Self {
            database_path,
            idempotency_ttl,
        }
    }

    pub(crate) async fn initialize(&self) -> Result<(), RuntimeStoreError> {
        let path = self.database_path.clone();
        tokio::task::spawn_blocking(move || Database::new(&path).map(|_| ()))
            .await
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?
            .map_err(RuntimeStoreError::Internal)
    }

    pub(crate) async fn mutate<F>(
        &self,
        actor: Actor,
        idempotency_key: String,
        operation: &'static str,
        request_hash: String,
        mutation: F,
    ) -> Result<MutationOutcome, RuntimeStoreError>
    where
        F: FnOnce(&Transaction<'_>, &Actor, &str) -> Result<Mutation, RuntimeStoreError>
            + Send
            + 'static,
    {
        let path = self.database_path.clone();
        let ttl = self.idempotency_ttl;
        tokio::task::spawn_blocking(move || {
            let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
            let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
            let scope = actor_scope(&actor);
            let now = Utc::now();
            let now_text = canonical_timestamp(now);
            cleanup_expired_idempotency(&tx, &now_text)?;
            let expires_at = now
                .checked_add_signed(
                    chrono::Duration::from_std(ttl)
                        .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?,
                )
                .ok_or_else(|| RuntimeStoreError::Invalid("idempotency expiry overflow".into()))?;
            let expires_at = canonical_timestamp(expires_at);

            let existing = tx
                .query_row(
                    "SELECT request_hash, response_json, expires_at
                     FROM mako_idempotency_keys
                     WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3",
                    params![scope, operation, idempotency_key],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((existing_hash, response_json, existing_expiry)) = existing {
                if existing_expiry > now_text {
                    if existing_hash != request_hash {
                        return Err(RuntimeStoreError::Conflict(
                            "the idempotency key was already used for different arguments".into(),
                        ));
                    }
                    let Some(response_json) = response_json else {
                        return Err(RuntimeStoreError::InProgress);
                    };
                    let response = serde_json::from_str(&response_json).map_err(|error| {
                        RuntimeStoreError::Internal(
                            anyhow::anyhow!(error).context("decoding replayed response"),
                        )
                    })?;
                    tx.commit()?;
                    return Ok(MutationOutcome {
                        response,
                        events: Vec::new(),
                        replayed: true,
                    });
                }
                tx.execute(
                    "DELETE FROM mako_idempotency_keys
                     WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3",
                    params![scope, operation, idempotency_key],
                )?;
            }

            tx.execute(
                "INSERT INTO mako_idempotency_keys (
                    scope_key, operation, idempotency_key, request_hash,
                    resource_id, response_json, created_at, expires_at
                 ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6)",
                params![
                    scope,
                    operation,
                    idempotency_key,
                    request_hash,
                    now_text,
                    expires_at
                ],
            )?;

            let result = mutation(&tx, &actor, &now_text)?;
            let response_json = serde_json::to_string(&result.response).map_err(|error| {
                RuntimeStoreError::Internal(anyhow::anyhow!(error).context("encoding response"))
            })?;
            tx.execute(
                "UPDATE mako_idempotency_keys
                 SET resource_id = ?4, response_json = ?5
                 WHERE scope_key = ?1 AND operation = ?2 AND idempotency_key = ?3",
                params![
                    scope,
                    operation,
                    idempotency_key,
                    result.resource_id,
                    response_json
                ],
            )?;
            tx.commit()?;
            Ok(MutationOutcome {
                response: result.response,
                events: result.events,
                replayed: false,
            })
        })
        .await
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?
    }

    pub(crate) async fn replay(
        &self,
        actor: Actor,
        session_id: String,
        requested_after: i64,
        limit: usize,
    ) -> Result<ReplaySnapshot, RuntimeStoreError> {
        let path = self.database_path.clone();
        tokio::task::spawn_blocking(move || {
            let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
            // Bind ownership, controller resolution, high-water, and replay
            // rows to one SQLite snapshot. Without this transaction, H+1 can
            // commit between MAX(sequence) and the row query, causing H+1 to
            // be emitted once as replay and again from the pre-registered live
            // receiver.
            let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
            let session = require_owned_session(&tx, &actor, &session_id)?;
            let controller =
                get_or_create_controller(&tx, &session, &canonical_timestamp(Utc::now()))?;
            let (earliest_available, high_water) = tx.query_row(
                "SELECT MIN(sequence), MAX(sequence)
                 FROM mako_controller_events WHERE controller_id = ?1",
                [&controller.id],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )?;
            let events = if limit == 0 {
                Vec::new()
            } else {
                let mut statement = tx.prepare(
                    "SELECT id, controller_id, sequence, event_type, run_id, schedule_id,
                            payload_json, created_at
                     FROM mako_controller_events
                     WHERE controller_id = ?1 AND sequence > ?2 AND sequence <= ?3
                     ORDER BY sequence DESC LIMIT ?4",
                )?;
                let mut events = statement
                    .query_map(
                        params![
                            controller.id,
                            requested_after,
                            high_water.unwrap_or(0),
                            limit as i64
                        ],
                        |row| map_persisted_event(row, &session.id),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                events.reverse();
                events
            };
            let earliest_returned = events.first().map(|event| event.sequence);
            let snapshot = ReplaySnapshot {
                events,
                requested_after,
                earliest_returned,
                earliest_available,
                high_water,
            };
            tx.commit()?;
            Ok(snapshot)
        })
        .await
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?
    }

    pub(crate) async fn stats(
        &self,
        actor: &Actor,
    ) -> Result<DaemonRuntimeStats, RuntimeStoreError> {
        let path = self.database_path.clone();
        let user_id = actor.user_id.clone();
        tokio::task::spawn_blocking(move || {
            let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
            let controllers: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_controllers
                 WHERE status = 'active' AND user_id IS ?1",
                params![user_id.as_deref()],
                |row| row.get(0),
            )?;
            let active_runs: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_runs r
                 JOIN mako_controllers c ON c.id = r.controller_id
                 WHERE r.status IN ('leased', 'running') AND c.user_id IS ?1",
                params![user_id.as_deref()],
                |row| row.get(0),
            )?;
            let queued_runs: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_runs r
                 JOIN mako_controllers c ON c.id = r.controller_id
                 WHERE r.status = 'queued' AND c.user_id IS ?1",
                params![user_id.as_deref()],
                |row| row.get(0),
            )?;
            let recovery_required: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_runs r
                 JOIN mako_controllers c ON c.id = r.controller_id
                 WHERE r.status = 'recovery_required' AND c.user_id IS ?1",
                params![user_id.as_deref()],
                |row| row.get(0),
            )?;
            Ok(DaemonRuntimeStats {
                active_controllers: runtime_count(controllers, "active controller")?,
                active_runs: runtime_count(active_runs, "active run")?,
                queued_runs: runtime_count(queued_runs, "queued run")?,
                recovery_required: runtime_count(recovery_required, "recovery-required run")?,
                ..DaemonRuntimeStats::default()
            })
        })
        .await
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?
    }

    pub(crate) async fn daemon_lease_is_current(
        &self,
        lease_name: &str,
        owner_id: &str,
    ) -> Result<bool, RuntimeStoreError> {
        let path = self.database_path.clone();
        let lease_name = lease_name.to_string();
        let owner_id = owner_id.to_string();
        tokio::task::spawn_blocking(move || {
            let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
            let now = canonical_timestamp(Utc::now());
            db.conn()
                .query_row(
                    "SELECT EXISTS (
                         SELECT 1 FROM mako_daemon_leases
                         WHERE lease_name = ?1 AND owner_id = ?2 AND expires_at > ?3
                     )",
                    params![lease_name, owner_id, now],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(RuntimeStoreError::from)
        })
        .await
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?
    }
}

fn runtime_count(value: i64, label: &str) -> Result<usize, RuntimeStoreError> {
    usize::try_from(value).map_err(|_| {
        RuntimeStoreError::Internal(anyhow::anyhow!("{label} count was outside usize range"))
    })
}

pub(crate) fn request_hash(actor: &Actor, command: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(&(actor, command)).unwrap_or_default();
    hash_request_bytes(bytes)
}

pub(crate) fn actor_scope(actor: &Actor) -> String {
    actor
        .user_id
        .as_ref()
        .map(|user_id| format!("user:{user_id}"))
        .unwrap_or_else(|| "local".to_string())
}

pub(crate) fn require_owned_session(
    connection: &rusqlite::Connection,
    actor: &Actor,
    session_id: &str,
) -> Result<OwnedSession, RuntimeStoreError> {
    if session_id.trim().is_empty() || session_id.len() > 256 || session_id.as_bytes().contains(&0)
    {
        return Err(RuntimeStoreError::Invalid(
            "session id is invalid or exceeds 256 bytes".into(),
        ));
    }
    let session = connection
        .query_row(
            "SELECT id, user_id, title, working_dir, project_dir, model, permission_mode
             FROM sessions WHERE id = ?1 AND session_type = 'mako'",
            [session_id],
            |row| {
                Ok(OwnedSession {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    title: row.get(2)?,
                    working_dir: row.get(3)?,
                    project_dir: row.get(4)?,
                    model: row.get(5)?,
                    permission_mode: row.get(6)?,
                })
            },
        )
        .optional()?;
    let Some(session) = session else {
        return Err(RuntimeStoreError::Ownership);
    };
    if session.user_id != actor.user_id {
        return Err(RuntimeStoreError::Ownership);
    }
    Ok(session)
}

pub(crate) fn require_controller(
    connection: &rusqlite::Connection,
    session_id: &str,
) -> Result<ControllerRecord, RuntimeStoreError> {
    connection
        .query_row(
            "SELECT id, session_id, status, timezone
             FROM mako_controllers WHERE session_id = ?1",
            [session_id],
            |row| {
                Ok(ControllerRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    status: row.get(2)?,
                    timezone: row.get(3)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| RuntimeStoreError::NotFound("Mako controller was not found".into()))
}

/// Legacy and server-created sessions predate the durable controller tables.
/// Materialize their one-to-one controller lazily after exact ownership has
/// already been established. The UUID and scope key are deterministic so a
/// retry or competing process converges on the same row.
pub(crate) fn get_or_create_controller(
    connection: &rusqlite::Connection,
    session: &OwnedSession,
    now: &str,
) -> Result<ControllerRecord, RuntimeStoreError> {
    match require_controller(connection, &session.id) {
        Ok(controller) => return Ok(controller),
        Err(RuntimeStoreError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    let controller_id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("krusty:mako:controller:{}", session.id).as_bytes(),
    )
    .to_string();
    connection.execute(
        "INSERT INTO mako_controllers (
            id, scope_key, user_id, session_id, status, timezone,
            max_concurrent_runs, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'active', 'UTC', 1, ?5, ?5)
         ON CONFLICT(session_id) DO NOTHING",
        params![
            controller_id,
            format!("session:{}", session.id),
            session.user_id,
            session.id,
            now
        ],
    )?;
    require_controller(connection, &session.id)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_event(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    event_type: &str,
    run_id: Option<&str>,
    schedule_id: Option<&str>,
    dedupe_key: Option<&str>,
    payload: Value,
    created_at: &str,
) -> Result<PersistedEvent, RuntimeStoreError> {
    if let Some(dedupe_key) = dedupe_key {
        if let Some(existing) = tx
            .query_row(
                "SELECT id, controller_id, sequence, event_type, run_id, schedule_id,
                        payload_json, created_at
                 FROM mako_controller_events
                 WHERE controller_id = ?1 AND dedupe_key = ?2",
                params![controller.id, dedupe_key],
                |row| map_persisted_event(row, &controller.session_id),
            )
            .optional()?
        {
            prune_controller_event_prefix(tx, &controller.id, false)?;
            cleanup_terminal_control_outbox(tx, created_at)?;
            return Ok(existing);
        }
    }
    let sequence: i64 = tx.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1
         FROM mako_controller_events WHERE controller_id = ?1",
        [&controller.id],
        |row| row.get(0),
    )?;
    // Treat this function as the durable privacy boundary. Callers may retain
    // richer payloads for an authenticated live stream, but replay storage is
    // allow-listed here so a new producer cannot accidentally journal model
    // reasoning, tool arguments/output, web bodies, user responses, or raw
    // error text.
    let payload = sanitize_event_payload(event_type, payload);
    let payload_json = serde_json::to_string(&payload)
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    if payload_json.len() > MAX_DURABLE_EVENT_PAYLOAD_BYTES {
        return Err(RuntimeStoreError::ResourceExhausted(format!(
            "durable controller event summary exceeds {MAX_DURABLE_EVENT_PAYLOAD_BYTES} bytes"
        )));
    }
    let reserve_resolution_capacity = !is_interaction_resolution(event_type);
    tx.execute(
        "INSERT INTO mako_controller_events (
            controller_id, sequence, event_type, run_id, schedule_id,
            dedupe_key, payload_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            controller.id,
            sequence,
            event_type,
            run_id,
            schedule_id,
            dedupe_key,
            payload_json,
            created_at
        ],
    )?;
    prune_controller_event_prefix(tx, &controller.id, reserve_resolution_capacity)?;
    cleanup_terminal_control_outbox(tx, created_at)?;
    Ok(PersistedEvent {
        controller_id: controller.id.clone(),
        session_id: controller.session_id.clone(),
        run_id: run_id.map(ToOwned::to_owned),
        schedule_id: schedule_id.map(ToOwned::to_owned),
        sequence,
        event_type: event_type.to_string(),
        payload,
        created_at: created_at.to_string(),
    })
}

fn sanitize_event_payload(event_type: &str, payload: Value) -> Value {
    let Some(source) = payload.as_object() else {
        return serde_json::json!({
            "payload_kind": json_value_kind(&payload),
            "redacted": true,
        });
    };

    if event_type == "agentic_event" {
        return sanitize_agentic_payload(source);
    }

    let mut durable = Map::new();
    let mut removed = false;
    for (key, value) in source {
        let safe_identifier_or_state = matches!(
            key.as_str(),
            "run_id"
                | "session_id"
                | "controller_id"
                | "schedule_id"
                | "occurrence_id"
                | "tool_call_id"
                | "pending_id"
                | "objective_message_id"
                | "kind"
                | "control_kind"
                | "status"
                | "previous_status"
                | "previous"
                | "current"
                | "priority"
                | "crew_slug"
                | "payload_kind"
                | "next_fire_at"
                | "wake_at"
                | "available_at"
        );
        let safe_counter = key.ends_with("_bytes")
            || key.ends_with("_chars")
            || key.ends_with("_count")
            || key.ends_with("_fields")
            || key.ends_with("_tokens")
            || key.ends_with("_secs")
            || matches!(
                key.as_str(),
                "revision"
                    | "attempt"
                    | "attempt_no"
                    | "value"
                    | "approved"
                    | "deleted"
                    | "redacted"
            );
        if (safe_identifier_or_state && value.is_string())
            || (safe_counter && (value.is_number() || value.is_boolean()))
        {
            durable.insert(key.clone(), value.clone());
        } else {
            removed = true;
            if let Some(text) = value.as_str() {
                durable.insert(
                    format!("{key}_chars"),
                    Value::from(text.chars().count() as u64),
                );
            }
        }
    }
    if removed {
        durable.insert("redacted".into(), Value::Bool(true));
    }
    Value::Object(durable)
}

fn sanitize_agentic_payload(source: &Map<String, Value>) -> Value {
    let Some(event_type) = source.get("type").and_then(Value::as_str) else {
        return serde_json::json!({"type": "redacted", "redacted": true});
    };
    let mut durable = Map::new();
    durable.insert("type".into(), Value::String(event_type.to_string()));

    for key in [
        "id",
        "name",
        "tool_call_id",
        "tool_name",
        "tool_use_id",
        "error_code",
        "mode",
        "session_id",
        "source_session_id",
        "new_session_id",
        "checkpoint_id",
        "delegated_run_id",
        "kind",
        "stage",
        "parent_session_id",
        "task_id",
        "agent_name",
        "agent_type",
        "status",
        "level",
        "decision",
        "stop_reason",
    ] {
        if let Some(Value::String(value)) = source.get(key) {
            durable.insert(key.into(), Value::String(value.clone()));
        }
    }
    for key in [
        "is_error",
        "arguments_redacted",
        "output_redacted",
        "error_redacted",
        "success",
        "has_more",
        "turn",
        "tick_number",
        "prompt_tokens",
        "input_tokens",
        "completion_tokens",
        "reasoning_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "total_tokens",
        "task_count",
        "duration_secs",
        "estimated_tokens_before",
        "estimated_tokens_after",
        "replaced_messages",
        "compaction_count",
        "tool_count",
        "tokens",
        "lines_added",
        "lines_removed",
        "message_chars",
        "output_chars",
        "reason_chars",
        "title_chars",
        "description_chars",
        "summary_chars",
        "result_chars",
        "error_chars",
    ] {
        if let Some(value) = source
            .get(key)
            .filter(|value| value.is_number() || value.is_boolean())
        {
            durable.insert(key.into(), value.clone());
        }
    }
    if let Some(arguments) = source.get("arguments") {
        durable.insert("arguments".into(), summarize_durable_shape(arguments));
        durable.insert("arguments_redacted".into(), Value::Bool(true));
    }
    durable.insert("redacted".into(), Value::Bool(true));
    Value::Object(durable)
}

fn summarize_durable_shape(value: &Value) -> Value {
    // A producer may already have supplied a redacted shape. Retain only its
    // fixed vocabulary and numeric cardinality, never object field names.
    if let Some(shape) = value.as_object() {
        if let Some(kind) = shape.get("type").and_then(Value::as_str) {
            let mut summary = Map::new();
            summary.insert("type".into(), Value::String(kind.to_string()));
            for key in ["field_count", "len"] {
                if let Some(number) = shape.get(key).filter(|value| value.is_number()) {
                    summary.insert(key.into(), number.clone());
                }
            }
            return Value::Object(summary);
        }
    }
    match value {
        Value::Object(map) => serde_json::json!({"type": "object", "field_count": map.len()}),
        Value::Array(items) => serde_json::json!({"type": "array", "len": items.len()}),
        Value::String(_) => serde_json::json!({"type": "string"}),
        Value::Number(_) => serde_json::json!({"type": "number"}),
        Value::Bool(_) => serde_json::json!({"type": "bool"}),
        Value::Null => serde_json::json!({"type": "null"}),
    }
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

fn is_interaction_resolution(event_type: &str) -> bool {
    matches!(
        event_type,
        "tool_approval_queued"
            | "tool_approval_delivered"
            | "user_response_received"
            | "user_response_staged"
    )
}

fn prune_controller_event_prefix(
    tx: &Transaction<'_>,
    controller_id: &str,
    reserve_resolution_capacity: bool,
) -> Result<(), RuntimeStoreError> {
    let (mut row_count, mut stored_bytes, max_sequence) = tx.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(
                    length(CAST(payload_json AS BLOB))
                    + length(CAST(event_type AS BLOB))
                    + COALESCE(length(CAST(dedupe_key AS BLOB)), 0)
                ), 0),
                MAX(sequence)
           FROM mako_controller_events
          WHERE controller_id = ?1",
        [controller_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        },
    )?;
    let Some(max_sequence) = max_sequence else {
        return Ok(());
    };

    // Retention is prefix-only. Canonical run state lives in `mako_runs`, so
    // replay retention protects only unresolved human/tool interactions. The
    // current maximum is always retained, preserving monotonic allocation.
    let (protected_min, pending_interactions) = tx.query_row(
        r#"
        WITH unresolved AS (
          SELECT e.sequence
            FROM mako_controller_events e
            JOIN mako_runs r ON r.id = e.run_id
           WHERE e.controller_id = ?1
             AND (
               (
               e.event_type = 'agentic_event'
               AND json_extract(e.payload_json, '$.type') = 'tool_approval_required'
               AND r.status IN ('leased', 'running')
               AND json_extract(e.payload_json, '$.id') IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1
                   FROM mako_controller_events later
                  WHERE later.controller_id = e.controller_id
                    AND later.run_id = e.run_id
                    AND later.sequence > e.sequence
                    AND (
                      (
                        later.event_type = 'agentic_event'
                        AND json_extract(later.payload_json, '$.type') IN (
                          'tool_approved', 'tool_denied', 'tool_result'
                        )
                        AND json_extract(later.payload_json, '$.id') =
                            json_extract(e.payload_json, '$.id')
                      )
                      OR (
                        later.event_type IN (
                          'tool_approval_queued', 'tool_approval_delivered'
                        )
                        AND json_extract(later.payload_json, '$.tool_call_id') =
                            json_extract(e.payload_json, '$.id')
                      )
                    )
               )
               )
               OR (
               e.event_type = 'agentic_event'
               AND json_extract(e.payload_json, '$.type') = 'awaiting_input'
               AND r.status IN ('leased', 'running', 'awaiting_input')
               AND json_extract(e.payload_json, '$.tool_call_id') IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1
                   FROM mako_controller_events later
                  WHERE later.controller_id = e.controller_id
                    AND later.run_id = e.run_id
                    AND later.sequence > e.sequence
                    AND later.event_type IN (
                      'user_response_received', 'user_response_staged'
                    )
                    AND json_extract(later.payload_json, '$.tool_call_id') =
                        json_extract(e.payload_json, '$.tool_call_id')
               )
               )
             )
        )
        SELECT MIN(sequence), COUNT(*) FROM unresolved
        "#,
        [controller_id],
        |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?)),
    )?;
    if pending_interactions > MAX_PENDING_INTERACTIONS {
        return Err(RuntimeStoreError::ResourceExhausted(format!(
            "controller has more than {MAX_PENDING_INTERACTIONS} unresolved interactions"
        )));
    }
    let reserve_for_pending = reserve_resolution_capacity && pending_interactions > 0;
    let row_limit = if reserve_for_pending {
        MAX_CONTROLLER_EVENT_ROWS - RESERVED_RESOLUTION_EVENT_ROWS
    } else {
        MAX_CONTROLLER_EVENT_ROWS
    };
    let byte_limit = if reserve_for_pending {
        MAX_CONTROLLER_EVENT_BYTES - RESERVED_RESOLUTION_EVENT_BYTES
    } else {
        MAX_CONTROLLER_EVENT_BYTES
    };
    if row_count <= row_limit && stored_bytes <= byte_limit {
        return Ok(());
    }
    let keep_from = protected_min.unwrap_or(max_sequence).min(max_sequence);

    while row_count > row_limit || stored_bytes > byte_limit {
        let candidates = {
            let mut statement = tx.prepare(
                "SELECT sequence,
                        length(CAST(payload_json AS BLOB))
                        + length(CAST(event_type AS BLOB))
                        + COALESCE(length(CAST(dedupe_key AS BLOB)), 0)
                   FROM mako_controller_events
                  WHERE controller_id = ?1 AND sequence < ?2
                  ORDER BY sequence ASC
                  LIMIT ?3",
            )?;
            let rows = statement
                .query_map(
                    params![controller_id, keep_from, EVENT_RETENTION_DELETE_BATCH],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        if candidates.is_empty() {
            break;
        }
        let mut cutoff = None;
        for (sequence, bytes) in candidates {
            cutoff = Some(sequence);
            row_count = row_count.saturating_sub(1);
            stored_bytes = stored_bytes.saturating_sub(bytes);
            if row_count <= row_limit && stored_bytes <= byte_limit {
                break;
            }
        }
        let Some(cutoff) = cutoff else {
            break;
        };
        tx.execute(
            "DELETE FROM mako_controller_events
              WHERE controller_id = ?1 AND sequence <= ?2",
            params![controller_id, cutoff],
        )?;
    }
    let (remaining_rows, remaining_bytes): (i64, i64) = tx.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(
                    length(CAST(payload_json AS BLOB))
                    + length(CAST(event_type AS BLOB))
                    + COALESCE(length(CAST(dedupe_key AS BLOB)), 0)
                ), 0)
           FROM mako_controller_events
          WHERE controller_id = ?1",
        [controller_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if remaining_rows > row_limit || remaining_bytes > byte_limit {
        return Err(RuntimeStoreError::ResourceExhausted(format!(
            "controller event journal reached its retention ceiling while unresolved interactions remain (rows={remaining_rows}/{row_limit}, bytes={remaining_bytes}/{byte_limit})"
        )));
    }
    Ok(())
}

fn cleanup_expired_idempotency(tx: &Transaction<'_>, now: &str) -> Result<(), RuntimeStoreError> {
    tx.execute(
        "DELETE FROM mako_idempotency_keys
          WHERE rowid IN (
            SELECT rowid FROM mako_idempotency_keys
             WHERE expires_at <= ?1
             ORDER BY expires_at ASC
             LIMIT ?2
          )",
        params![now, MAINTENANCE_DELETE_BATCH],
    )?;
    Ok(())
}

fn cleanup_terminal_control_outbox(
    tx: &Transaction<'_>,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    let cutoff = parse_utc_timestamp(now)
        .unwrap_or_else(|_| Utc::now())
        .checked_sub_signed(chrono::Duration::days(TERMINAL_OUTBOX_RETENTION_DAYS))
        .map(canonical_timestamp)
        .unwrap_or_else(|| now.to_string());
    tx.execute(
        "DELETE FROM mako_control_outbox
          WHERE rowid IN (
            SELECT rowid FROM mako_control_outbox
             WHERE status IN ('delivered', 'discarded')
               AND updated_at < ?1
             ORDER BY updated_at ASC
             LIMIT ?2
          )",
        params![cutoff, MAINTENANCE_DELETE_BATCH],
    )?;
    Ok(())
}

fn map_persisted_event(row: &Row<'_>, session_id: &str) -> rusqlite::Result<PersistedEvent> {
    let payload_json = row.get::<_, String>(6)?;
    let payload = serde_json::from_str(&payload_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        )
    })?;
    Ok(PersistedEvent {
        controller_id: row.get(1)?,
        session_id: session_id.to_string(),
        run_id: row.get(4)?,
        schedule_id: row.get(5)?,
        sequence: row.get(2)?,
        event_type: row.get(3)?,
        payload,
        created_at: row.get(7)?,
    })
}

pub(crate) fn unix_millis_to_utc(value: i64) -> Result<DateTime<Utc>, RuntimeStoreError> {
    DateTime::from_timestamp_millis(value).ok_or_else(|| {
        RuntimeStoreError::Invalid("timestamp is outside the supported range".into())
    })
}
