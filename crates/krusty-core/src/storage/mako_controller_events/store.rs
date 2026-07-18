use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::mako::normalize_timestamp;
use crate::storage::Database;

use super::{MakoControllerEvent, NewMakoControllerEvent};

const COLUMNS: &str = "id, controller_id, sequence, event_type, run_id, schedule_id, dedupe_key, payload_json, created_at";

pub struct MakoControllerEventStore {
    db: Database,
}

impl MakoControllerEventStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Appends a controller-local ordered event. A duplicate dedupe key returns the first event.
    pub fn append(&self, new_event: &NewMakoControllerEvent) -> Result<MakoControllerEvent> {
        anyhow::ensure!(
            !new_event.controller_id.trim().is_empty(),
            "controller event has an empty controller id"
        );
        anyhow::ensure!(
            new_event
                .dedupe_key
                .as_deref()
                .is_none_or(|key| !key.trim().is_empty()),
            "controller event has an empty dedupe key"
        );
        let payload_json = serde_json::to_string(&new_event.payload)?;
        let created_at = normalize_timestamp(&new_event.created_at)?;
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;

        if let Some(dedupe_key) = new_event.dedupe_key.as_deref() {
            if let Some(existing) = get_by_dedupe(&tx, &new_event.controller_id, dedupe_key)? {
                tx.commit()?;
                return Ok(existing);
            }
        }

        let previous = tx.query_row(
            "SELECT COALESCE(MAX(sequence), 0)
             FROM mako_controller_events WHERE controller_id = ?1",
            [&new_event.controller_id],
            |row| row.get::<_, i64>(0),
        )?;
        anyhow::ensure!(previous >= 0, "negative controller event sequence");
        let sequence = previous
            .checked_add(1)
            .context("controller event sequence exhausted")?;
        tx.execute(
            "INSERT INTO mako_controller_events (
                controller_id, sequence, event_type, run_id, schedule_id,
                dedupe_key, payload_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                new_event.controller_id,
                sequence,
                new_event.event_type.to_string(),
                new_event.run_id,
                new_event.schedule_id,
                new_event.dedupe_key,
                payload_json,
                created_at,
            ],
        )?;
        let id = tx.last_insert_rowid();
        let sql = format!("SELECT {COLUMNS} FROM mako_controller_events WHERE id = ?1");
        let event = tx.query_row(&sql, [id], map_event)?;
        tx.commit()?;
        Ok(event)
    }

    pub fn list_after(
        &self,
        controller_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<MakoControllerEvent>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let after_sequence = i64::try_from(after_sequence).context("sequence is too large")?;
        let sql = format!(
            "SELECT {COLUMNS} FROM mako_controller_events
             WHERE controller_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC LIMIT ?3"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let events = statement
            .query_map(
                params![controller_id, after_sequence, limit as i64],
                map_event,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Mako controller events")?;
        Ok(events)
    }

    /// Returns exact durable approval requests whose latest state for the
    /// same `(run_id, tool_call_id)` remains pending and whose run can still
    /// accept an approval. Trace-run identifiers are deliberately excluded.
    pub fn list_pending_tool_approvals(
        &self,
        controller_id: &str,
    ) -> Result<Vec<MakoControllerEvent>> {
        let sql = format!(
            "WITH interactions AS (
                SELECT e.id, e.controller_id, e.sequence, e.event_type, e.run_id,
                       e.schedule_id, e.dedupe_key, e.payload_json, e.created_at,
                       r.status AS run_status,
                       CASE
                         WHEN e.event_type = 'agentic_event'
                           THEN json_extract(e.payload_json, '$.type')
                         ELSE e.event_type
                       END AS interaction_state,
                       CASE
                         WHEN e.event_type = 'agentic_event'
                           THEN json_extract(e.payload_json, '$.id')
                         ELSE json_extract(e.payload_json, '$.tool_call_id')
                       END AS tool_call_id
                  FROM mako_controller_events e
                  JOIN mako_runs r ON r.id = e.run_id
                 WHERE e.controller_id = ?1
                   AND e.run_id IS NOT NULL
                   AND (
                     (e.event_type = 'agentic_event'
                       AND json_extract(e.payload_json, '$.type') IN (
                         'tool_approval_required', 'tool_approved', 'tool_denied', 'tool_result'
                       ))
                     OR e.event_type IN ('tool_approval_queued', 'tool_approval_delivered')
                   )
            ), ranked AS (
                SELECT *, ROW_NUMBER() OVER (
                    PARTITION BY run_id, tool_call_id ORDER BY sequence DESC
                ) AS state_rank
                  FROM interactions
                 WHERE tool_call_id IS NOT NULL
            )
            SELECT {COLUMNS} FROM ranked
             WHERE state_rank = 1
               AND interaction_state = 'tool_approval_required'
               AND run_status IN ('leased', 'running')
             ORDER BY sequence ASC"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let events = statement
            .query_map([controller_id], map_event)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing pending durable Mako tool approvals")?;
        Ok(events)
    }

    /// Returns durable run ids whose latest state for this exact question is
    /// still awaiting a response.
    pub fn list_pending_user_response_runs(
        &self,
        controller_id: &str,
        tool_call_id: &str,
    ) -> Result<Vec<String>> {
        let mut statement = self.db.conn().prepare(
            "WITH interactions AS (
                SELECT e.run_id, e.sequence, r.status AS run_status,
                       CASE
                         WHEN e.event_type = 'agentic_event'
                           THEN json_extract(e.payload_json, '$.type')
                         ELSE e.event_type
                       END AS interaction_state
                  FROM mako_controller_events e
                  JOIN mako_runs r ON r.id = e.run_id
                 WHERE e.controller_id = ?1
                   AND e.run_id IS NOT NULL
                   AND (
                     (e.event_type = 'agentic_event'
                       AND json_extract(e.payload_json, '$.type') = 'awaiting_input'
                       AND json_extract(e.payload_json, '$.tool_call_id') = ?2)
                     OR
                     (e.event_type IN ('user_response_received', 'user_response_staged')
                       AND json_extract(e.payload_json, '$.tool_call_id') = ?2)
                   )
            ), ranked AS (
                SELECT *, ROW_NUMBER() OVER (
                    PARTITION BY run_id ORDER BY sequence DESC
                ) AS state_rank
                  FROM interactions
            )
            SELECT run_id FROM ranked
             WHERE state_rank = 1
               AND interaction_state = 'awaiting_input'
               AND run_status IN ('leased', 'running', 'awaiting_input')
             ORDER BY sequence ASC",
        )?;
        let run_ids = statement
            .query_map(params![controller_id, tool_call_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing pending durable Mako user responses")?;
        Ok(run_ids)
    }

    #[cfg(test)]
    pub(super) fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }
}

fn get_by_dedupe(
    tx: &Transaction<'_>,
    controller_id: &str,
    dedupe_key: &str,
) -> Result<Option<MakoControllerEvent>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM mako_controller_events
         WHERE controller_id = ?1 AND dedupe_key = ?2"
    );
    tx.query_row(&sql, params![controller_id, dedupe_key], map_event)
        .optional()
        .context("reading deduplicated controller event")
}

fn map_event(row: &Row<'_>) -> rusqlite::Result<MakoControllerEvent> {
    let sequence = nonnegative_i64(row, 2)? as u64;
    let event_type = row.get::<_, String>(3)?;
    let payload_json = row.get::<_, String>(7)?;
    let payload = serde_json::from_str(&payload_json)
        .map_err(|error| conversion_error(7, format!("invalid event payload JSON: {error}")))?;
    Ok(MakoControllerEvent {
        id: row.get(0)?,
        controller_id: row.get(1)?,
        sequence,
        event_type,
        run_id: row.get(4)?,
        schedule_id: row.get(5)?,
        dedupe_key: row.get(6)?,
        payload,
        created_at: row.get(8)?,
    })
}

fn nonnegative_i64(row: &Row<'_>, index: usize) -> rusqlite::Result<i64> {
    let value = row.get::<_, i64>(index)?;
    if value < 0 {
        Err(conversion_error(index, "negative unsigned value"))
    } else {
        Ok(value)
    }
}

fn conversion_error(index: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(IoError::new(ErrorKind::InvalidData, message.into())),
    )
}
