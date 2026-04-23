use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};

use crate::storage::database::Database;

use super::model::{MakoRunPriority, MakoRuntimeState, MakoRuntimeStateStatus};

const SELECT_RUNTIME_STATE_COLUMNS: &str = r#"
    SELECT session_id, status, next_wake_at, sleep_reason, last_error,
           current_run_id, last_wake_reason, crew_slug, priority, updated_at
    FROM mako_runtime_state
"#;

pub struct MakoRuntimeStateStore {
    db: Database,
}

impl MakoRuntimeStateStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn get_state(&self, session_id: &str) -> Result<Option<MakoRuntimeState>> {
        let sql = format!("{SELECT_RUNTIME_STATE_COLUMNS} WHERE session_id = ?1");
        let mut stmt = self.db.conn().prepare(&sql)?;

        stmt.query_row(params![session_id], map_state_row)
            .optional()
            .context("fetching mako runtime state")
    }

    pub fn list_recoverable_states(&self) -> Result<Vec<MakoRuntimeState>> {
        let sql = format!(
            "{SELECT_RUNTIME_STATE_COLUMNS}
             WHERE status IN ('running', 'sleeping')
             ORDER BY updated_at ASC"
        );
        let mut stmt = self.db.conn().prepare(&sql)?;

        let rows = stmt
            .query_map([], map_state_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_states_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<HashMap<String, MakoRuntimeState>> {
        if session_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let placeholders = vec!["?"; session_ids.len()].join(", ");
        let sql = format!(
            "{SELECT_RUNTIME_STATE_COLUMNS}
             WHERE session_id IN ({placeholders})"
        );
        let mut stmt = self.db.conn().prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(session_ids.iter()), map_state_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows
            .into_iter()
            .map(|state| (state.session_id.clone(), state))
            .collect())
    }

    pub fn upsert_state(&self, state: &MakoRuntimeState) -> Result<()> {
        self.db.conn().execute(
            "INSERT INTO mako_runtime_state (
                session_id, status, next_wake_at, sleep_reason, last_error,
                current_run_id, last_wake_reason, crew_slug, priority, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                next_wake_at = excluded.next_wake_at,
                sleep_reason = excluded.sleep_reason,
                last_error = excluded.last_error,
                current_run_id = excluded.current_run_id,
                last_wake_reason = excluded.last_wake_reason,
                crew_slug = excluded.crew_slug,
                priority = excluded.priority,
                updated_at = excluded.updated_at",
            params![
                state.session_id,
                state.status.to_string(),
                state.next_wake_at,
                state.sleep_reason,
                state.last_error,
                state.current_run_id,
                state.last_wake_reason,
                state.crew_slug,
                state.priority.to_string(),
                state.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn set_state(
        &self,
        session_id: &str,
        status: MakoRuntimeStateStatus,
        next_wake_at: Option<&str>,
        sleep_reason: Option<&str>,
        last_error: Option<&str>,
        current_run_id: Option<&str>,
        last_wake_reason: Option<&str>,
        priority: MakoRunPriority,
    ) -> Result<()> {
        let existing = self.get_state(session_id)?;
        let state = MakoRuntimeState {
            session_id: session_id.to_string(),
            status,
            next_wake_at: next_wake_at.map(ToOwned::to_owned),
            sleep_reason: sleep_reason.map(ToOwned::to_owned),
            last_error: last_error.map(ToOwned::to_owned),
            current_run_id: current_run_id.map(ToOwned::to_owned),
            last_wake_reason: last_wake_reason.map(ToOwned::to_owned),
            crew_slug: existing.and_then(|state| state.crew_slug),
            priority,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        self.upsert_state(&state)
    }

    pub fn set_priority(&self, session_id: &str, priority: MakoRunPriority) -> Result<()> {
        let state = self
            .get_state(session_id)?
            .unwrap_or_else(|| MakoRuntimeState::new_empty(session_id));

        self.upsert_state(&MakoRuntimeState {
            priority,
            updated_at: chrono::Utc::now().to_rfc3339(),
            ..state
        })
    }

    pub fn set_crew_slug(&self, session_id: &str, crew_slug: Option<&str>) -> Result<()> {
        let state = self
            .get_state(session_id)?
            .unwrap_or_else(|| MakoRuntimeState::new_empty(session_id));

        self.upsert_state(&MakoRuntimeState {
            crew_slug: crew_slug.map(ToOwned::to_owned),
            updated_at: chrono::Utc::now().to_rfc3339(),
            ..state
        })
    }

    pub fn delete_state(&self, session_id: &str) -> Result<()> {
        self.db.conn().execute(
            "DELETE FROM mako_runtime_state WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }
}

fn map_state_row(row: &Row<'_>) -> rusqlite::Result<MakoRuntimeState> {
    let status_raw: String = row.get(1)?;
    let priority_raw: String = row.get(8)?;
    Ok(MakoRuntimeState {
        session_id: row.get(0)?,
        status: MakoRuntimeStateStatus::parse(&status_raw).unwrap_or(MakoRuntimeStateStatus::Idle),
        next_wake_at: row.get(2)?,
        sleep_reason: row.get(3)?,
        last_error: row.get(4)?,
        current_run_id: row.get(5)?,
        last_wake_reason: row.get(6)?,
        crew_slug: row.get(7)?,
        priority: MakoRunPriority::parse(&priority_raw).unwrap_or_default(),
        updated_at: row.get(9)?,
    })
}
