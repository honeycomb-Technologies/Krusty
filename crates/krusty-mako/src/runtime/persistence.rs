use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use krusty_core::mako::{canonical_timestamp, parse_utc_timestamp};
use krusty_core::storage::{hash_request_bytes, Database};
use krusty_mako_protocol::{
    unix_time_millis, Actor, EventEnvelope, ExtensionEvent, MakoEvent, ProtocolErrorPayload,
    ProtocolVersion, ResponsePayload, RuntimeEvent,
};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;

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
            let session = require_owned_session(db.conn(), &actor, &session_id)?;
            let controller =
                get_or_create_controller(db.conn(), &session, &canonical_timestamp(Utc::now()))?;
            let (earliest_available, high_water) = db.conn().query_row(
                "SELECT MIN(sequence), MAX(sequence)
                 FROM mako_controller_events WHERE controller_id = ?1",
                [&controller.id],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )?;
            let events = if limit == 0 {
                Vec::new()
            } else {
                let mut statement = db.conn().prepare(
                    "SELECT id, controller_id, sequence, event_type, run_id, schedule_id,
                            payload_json, created_at
                     FROM mako_controller_events
                     WHERE controller_id = ?1 AND sequence > ?2
                     ORDER BY sequence DESC LIMIT ?3",
                )?;
                let mut events = statement
                    .query_map(
                        params![controller.id, requested_after, limit as i64],
                        |row| map_persisted_event(row, &session.id),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                events.reverse();
                events
            };
            let earliest_returned = events.first().map(|event| event.sequence);
            Ok(ReplaySnapshot {
                events,
                requested_after,
                earliest_returned,
                earliest_available,
                high_water,
            })
        })
        .await
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?
    }

    pub(crate) async fn stats(&self) -> Result<Value, RuntimeStoreError> {
        let path = self.database_path.clone();
        tokio::task::spawn_blocking(move || {
            let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
            let controllers: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_controllers WHERE status = 'active'",
                [],
                |row| row.get(0),
            )?;
            let active_runs: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_runs WHERE status IN ('leased', 'running')",
                [],
                |row| row.get(0),
            )?;
            let queued_runs: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_runs WHERE status = 'queued'",
                [],
                |row| row.get(0),
            )?;
            let recovery_required: i64 = db.conn().query_row(
                "SELECT COUNT(*) FROM mako_runs WHERE status = 'recovery_required'",
                [],
                |row| row.get(0),
            )?;
            Ok(serde_json::json!({
                "active_controllers": controllers,
                "active_runs": active_runs,
                "queued_runs": queued_runs,
                "recovery_required": recovery_required,
            }))
        })
        .await
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?
    }
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
    let session = connection
        .query_row(
            "SELECT id, user_id, title, working_dir, project_dir, model
             FROM sessions WHERE id = ?1",
            [session_id],
            |row| {
                Ok(OwnedSession {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    title: row.get(2)?,
                    working_dir: row.get(3)?,
                    project_dir: row.get(4)?,
                    model: row.get(5)?,
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
            return Ok(existing);
        }
    }
    let sequence: i64 = tx.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1
         FROM mako_controller_events WHERE controller_id = ?1",
        [&controller.id],
        |row| row.get(0),
    )?;
    let payload_json = serde_json::to_string(&payload)
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
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
