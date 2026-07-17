use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::mako::normalize_timestamp;
use crate::storage::Database;

use super::{MakoControllerEvent, MakoControllerEventType, NewMakoControllerEvent};

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
    let raw_event_type = row.get::<_, String>(3)?;
    let event_type = MakoControllerEventType::parse(&raw_event_type).ok_or_else(|| {
        conversion_error(
            3,
            format!("invalid controller event type: {raw_event_type}"),
        )
    })?;
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
