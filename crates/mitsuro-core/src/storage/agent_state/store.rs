use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use super::model::AgentState;
use crate::storage::database::Database;

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

    /// Get the agent state for a session without swallowing storage failures.
    pub fn try_get_agent_state(&self, session_id: &str) -> Result<Option<AgentState>> {
        self.db
            .conn()
            .query_row(
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
            )
            .optional()
            .map_err(Into::into)
    }

    /// Update agent last_event_at timestamp (for keeping session alive)
    pub fn touch_agent_event(&self, session_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE sessions SET agent_last_event_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }

    /// List sessions with active agents (not idle)
    #[cfg(test)]
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
