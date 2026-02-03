//! Agent state tracking storage
//!
//! Handles agent execution state for sessions (for background execution).

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::database::Database;

/// Agent execution state
#[derive(Debug, Clone)]
pub struct AgentState {
    /// Current state: "idle", "streaming", "tool_executing", "awaiting_input", "error"
    pub state: String,
    /// When the agent started processing
    pub started_at: Option<String>,
    /// Last event timestamp
    pub last_event_at: Option<String>,
}

/// Agent state store
pub struct AgentStateStore<'a> {
    db: &'a Database,
}

impl<'a> AgentStateStore<'a> {
    /// Create a new agent state store with database reference
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Set the agent execution state for a session
    ///
    /// Valid states: "idle", "streaming", "tool_executing", "awaiting_input", "error"
    pub fn set_agent_state(&self, session_id: &str, state: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        // Update state and last_event_at
        // Set agent_started_at only when transitioning from idle
        self.db.conn().execute(
            "UPDATE sessions SET
                agent_state = ?1,
                agent_last_event_at = ?2,
                agent_started_at = CASE
                    WHEN agent_state = 'idle' AND ?1 != 'idle' THEN ?2
                    WHEN ?1 = 'idle' THEN NULL
                    ELSE agent_started_at
                END
             WHERE id = ?3",
            params![state, now, session_id],
        )?;
        Ok(())
    }

    /// Get the agent state for a session
    pub fn get_agent_state(&self, session_id: &str) -> Option<AgentState> {
        let result = self.db.conn().query_row(
            "SELECT agent_state, agent_started_at, agent_last_event_at
             FROM sessions WHERE id = ?1",
            [session_id],
            |row| {
                Ok(AgentState {
                    state: row.get::<_, String>(0)?,
                    started_at: row.get::<_, Option<String>>(1)?,
                    last_event_at: row.get::<_, Option<String>>(2)?,
                })
            },
        );

        result.ok()
    }

    /// Update agent last_event_at timestamp (for keeping session "alive")
    pub fn touch_agent_event(&self, session_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE sessions SET agent_last_event_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }

    /// List sessions with active agents (not idle)
    pub fn list_active_sessions(&self) -> Result<Vec<(String, AgentState)>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, agent_state, agent_started_at, agent_last_event_at
             FROM sessions WHERE agent_state != 'idle'",
        )?;

        let sessions = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                AgentState {
                    state: row.get::<_, String>(1)?,
                    started_at: row.get::<_, Option<String>>(2)?,
                    last_event_at: row.get::<_, Option<String>>(3)?,
                },
            ))
        })?;

        sessions.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::storage::Database;

    use super::AgentStateStore;

    /// Helper to create a temporary database for testing
    fn create_test_db() -> (Database, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("Failed to create database");
        (db, temp_dir)
    }

    #[test]
    fn test_set_and_get_agent_state() {
        let (db, _temp) = create_test_db();
        let store = AgentStateStore::new(&db);

        // Create a session first
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "Test", now, now],
            )
            .expect("Failed to create session");

        // Set agent state
        store
            .set_agent_state(&session_id, "streaming")
            .expect("Failed to set agent state");

        // Get agent state
        let state = store.get_agent_state(&session_id);

        assert!(state.is_some());
        assert_eq!(state.unwrap().state, "streaming");
    }

    #[test]
    fn test_list_active_sessions() {
        let (db, _temp) = create_test_db();
        let store = AgentStateStore::new(&db);

        // Create two sessions
        let session1 = uuid::Uuid::new_v4().to_string();
        let session2 = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        for session_id in &[&session1, &session2] {
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, title, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![session_id, "Test", now, now],
                )
                .expect("Failed to create session");
        }

        // Set one session to streaming, other to idle
        store
            .set_agent_state(&session1, "streaming")
            .expect("Failed to set agent state");
        store
            .set_agent_state(&session2, "idle")
            .expect("Failed to set agent state");

        // List active sessions
        let active = store
            .list_active_sessions()
            .expect("Failed to list active sessions");

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0, session1);
        assert_eq!(active[0].1.state, "streaming");
    }
}
