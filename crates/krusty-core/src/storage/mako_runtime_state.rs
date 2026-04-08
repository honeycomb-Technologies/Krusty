//! Persistent Mako runtime state for daemon-owned autonomous sessions.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::database::Database;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MakoRuntimeStateStatus {
    Idle,
    Running,
    Sleeping,
    AwaitingInput,
    Paused,
    Error,
    Cancelled,
}

impl MakoRuntimeStateStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Sleeping => "sleeping",
            Self::AwaitingInput => "awaiting_input",
            Self::Paused => "paused",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "idle" => Some(Self::Idle),
            "running" => Some(Self::Running),
            "sleeping" => Some(Self::Sleeping),
            "awaiting_input" => Some(Self::AwaitingInput),
            "paused" => Some(Self::Paused),
            "error" => Some(Self::Error),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

impl std::fmt::Display for MakoRuntimeStateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MakoRunPriority {
    Low,
    Normal,
    High,
}

impl MakoRunPriority {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

impl Default for MakoRunPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl std::fmt::Display for MakoRunPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MakoRuntimeState {
    pub session_id: String,
    pub status: MakoRuntimeStateStatus,
    pub next_wake_at: Option<String>,
    pub sleep_reason: Option<String>,
    pub last_error: Option<String>,
    pub current_run_id: Option<String>,
    pub last_wake_reason: Option<String>,
    pub priority: MakoRunPriority,
    pub updated_at: String,
}

pub struct MakoRuntimeStateStore {
    db: Database,
}

impl MakoRuntimeStateStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn get_state(&self, session_id: &str) -> Result<Option<MakoRuntimeState>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT session_id, status, next_wake_at, sleep_reason, last_error,
                    current_run_id, last_wake_reason, priority, updated_at
             FROM mako_runtime_state
             WHERE session_id = ?1",
        )?;

        stmt.query_row(params![session_id], map_state_row)
            .optional()
            .context("fetching mako runtime state")
    }

    pub fn list_recoverable_states(&self) -> Result<Vec<MakoRuntimeState>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT session_id, status, next_wake_at, sleep_reason, last_error,
                    current_run_id, last_wake_reason, priority, updated_at
             FROM mako_runtime_state
             WHERE status IN ('running', 'sleeping')
             ORDER BY updated_at ASC",
        )?;

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
            "SELECT session_id, status, next_wake_at, sleep_reason, last_error,
                    current_run_id, last_wake_reason, priority, updated_at
             FROM mako_runtime_state
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
                current_run_id, last_wake_reason, priority, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                next_wake_at = excluded.next_wake_at,
                sleep_reason = excluded.sleep_reason,
                last_error = excluded.last_error,
                current_run_id = excluded.current_run_id,
                last_wake_reason = excluded.last_wake_reason,
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
        let state = MakoRuntimeState {
            session_id: session_id.to_string(),
            status,
            next_wake_at: next_wake_at.map(ToOwned::to_owned),
            sleep_reason: sleep_reason.map(ToOwned::to_owned),
            last_error: last_error.map(ToOwned::to_owned),
            current_run_id: current_run_id.map(ToOwned::to_owned),
            last_wake_reason: last_wake_reason.map(ToOwned::to_owned),
            priority,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        self.upsert_state(&state)
    }

    pub fn set_priority(&self, session_id: &str, priority: MakoRunPriority) -> Result<()> {
        let existing = self.get_state(session_id)?;
        let state = existing.unwrap_or(MakoRuntimeState {
            session_id: session_id.to_string(),
            status: MakoRuntimeStateStatus::Idle,
            next_wake_at: None,
            sleep_reason: None,
            last_error: None,
            current_run_id: None,
            last_wake_reason: None,
            priority: MakoRunPriority::Normal,
            updated_at: chrono::Utc::now().to_rfc3339(),
        });

        self.upsert_state(&MakoRuntimeState {
            priority,
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
}

fn map_state_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MakoRuntimeState> {
    let status_raw: String = row.get(1)?;
    let priority_raw: String = row.get(7)?;
    Ok(MakoRuntimeState {
        session_id: row.get(0)?,
        status: MakoRuntimeStateStatus::parse(&status_raw).unwrap_or(MakoRuntimeStateStatus::Idle),
        next_wake_at: row.get(2)?,
        sleep_reason: row.get(3)?,
        last_error: row.get(4)?,
        current_run_id: row.get(5)?,
        last_wake_reason: row.get(6)?,
        priority: MakoRunPriority::parse(&priority_raw).unwrap_or_default(),
        updated_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::storage::Database;

    fn create_store() -> (MakoRuntimeStateStore, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let db = Database::new(&tmp.path().join("mako.db")).expect("db");
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["sess-1", "Mako Test", now, now],
            )
            .expect("seed session");
        (MakoRuntimeStateStore::new(db), tmp)
    }

    #[test]
    fn set_and_get_state_round_trip() {
        let (store, _tmp) = create_store();
        store
            .set_state(
                "sess-1",
                MakoRuntimeStateStatus::Running,
                None,
                None,
                None,
                Some("run-1"),
                Some("dispatch"),
                MakoRunPriority::Normal,
            )
            .expect("state write");

        let state = store
            .get_state("sess-1")
            .expect("state read")
            .expect("state present");
        assert_eq!(state.status, MakoRuntimeStateStatus::Running);
        assert_eq!(state.current_run_id.as_deref(), Some("run-1"));
        assert_eq!(state.last_wake_reason.as_deref(), Some("dispatch"));
        assert_eq!(state.priority, MakoRunPriority::Normal);
    }

    #[test]
    fn list_recoverable_states_only_returns_running_and_sleeping() {
        let (store, _tmp) = create_store();
        let now = chrono::Utc::now().to_rfc3339();
        store
            .db
            .conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["sess-2", "Mako Sleep", now, now],
            )
            .expect("seed second session");
        store
            .set_state(
                "sess-1",
                MakoRuntimeStateStatus::Running,
                None,
                None,
                None,
                None,
                None,
                MakoRunPriority::Normal,
            )
            .expect("write running state");
        store
            .set_state(
                "sess-2",
                MakoRuntimeStateStatus::Sleeping,
                Some("2026-01-01T00:00:00Z"),
                Some("waiting"),
                None,
                None,
                Some("sleep"),
                MakoRunPriority::High,
            )
            .expect("write sleeping state");
        store
            .db
            .conn()
            .execute(
                "UPDATE mako_runtime_state SET status = 'paused' WHERE session_id = ?1",
                rusqlite::params!["sess-1"],
            )
            .expect("rewrite sess-1 paused");
        store
            .set_state(
                "sess-1",
                MakoRuntimeStateStatus::Running,
                None,
                None,
                None,
                None,
                None,
                MakoRunPriority::Normal,
            )
            .expect("rewrite sess-1 running");

        let states = store.list_recoverable_states().expect("recoverable states");
        assert_eq!(states.len(), 2);
        assert!(states
            .iter()
            .any(|state| state.session_id == "sess-1"
                && state.status == MakoRuntimeStateStatus::Running));
        assert!(states.iter().any(|state| state.session_id == "sess-2"
            && state.status == MakoRuntimeStateStatus::Sleeping));
    }

    #[test]
    fn list_states_for_sessions_returns_requested_rows_only() {
        let (store, _tmp) = create_store();
        let now = chrono::Utc::now().to_rfc3339();
        store
            .db
            .conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["sess-2", "Mako Sleep", now, now],
            )
            .expect("seed second session");
        store
            .db
            .conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["sess-3", "Mako Idle", now, now],
            )
            .expect("seed third session");
        store
            .set_state(
                "sess-1",
                MakoRuntimeStateStatus::Running,
                None,
                None,
                None,
                Some("run-1"),
                Some("dispatch"),
                MakoRunPriority::Low,
            )
            .expect("write sess-1 state");
        store
            .set_state(
                "sess-2",
                MakoRuntimeStateStatus::Sleeping,
                Some("2026-01-01T00:00:00Z"),
                Some("waiting"),
                None,
                None,
                Some("sleep"),
                MakoRunPriority::High,
            )
            .expect("write sess-2 state");

        let states = store
            .list_states_for_sessions(&["sess-1".to_string(), "sess-3".to_string()])
            .expect("batch state lookup");

        assert_eq!(states.len(), 1);
        assert_eq!(
            states.get("sess-1").map(|state| state.status),
            Some(MakoRuntimeStateStatus::Running)
        );
        assert_eq!(
            states.get("sess-1").map(|state| state.priority),
            Some(MakoRunPriority::Low)
        );
        assert!(!states.contains_key("sess-3"));
    }

    #[test]
    fn set_priority_creates_or_updates_runtime_state() {
        let (store, _tmp) = create_store();
        store
            .set_priority("sess-1", MakoRunPriority::High)
            .expect("priority write");

        let state = store
            .get_state("sess-1")
            .expect("state read")
            .expect("state present");
        assert_eq!(state.status, MakoRuntimeStateStatus::Idle);
        assert_eq!(state.priority, MakoRunPriority::High);
    }
}
