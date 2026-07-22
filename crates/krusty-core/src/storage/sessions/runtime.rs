use anyhow::Result;

use crate::storage::agent_state::{AgentState, AgentStateStore};

use super::super::{RuntimeTraceEvent, RuntimeTraceStore, RuntimeTraceSummary};
use super::{SessionInfo, SessionManager};

const LIST_ACTIVE_SESSIONS_SQL_ALL: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode, model_key_json, model_catalog_revision,
            agent_state, agent_started_at, agent_last_event_at
             FROM sessions
             WHERE agent_state != 'idle'
             ORDER BY id";
const LIST_ACTIVE_SESSIONS_SQL_BY_USER: &str =
    "SELECT id, title, updated_at, token_count, parent_session_id, working_dir, user_id, work_mode, model, target_branch, project_dir, workspace_mode, session_type, permission_mode, model_key_json, model_catalog_revision,
            agent_state, agent_started_at, agent_last_event_at
             FROM sessions
             WHERE agent_state != 'idle' AND user_id = ?1
             ORDER BY id";

impl SessionManager {
    pub fn set_agent_state(&self, session_id: &str, state: &str) -> Result<()> {
        AgentStateStore::new(&self.db).set_agent_state(session_id, state)
    }

    /// Get the agent state for a session
    pub fn get_agent_state(&self, session_id: &str) -> Option<AgentState> {
        self.try_get_agent_state(session_id).ok().flatten()
    }

    /// Get the agent state for a session without swallowing storage failures.
    pub fn try_get_agent_state(&self, session_id: &str) -> Result<Option<AgentState>> {
        AgentStateStore::new(&self.db).try_get_agent_state(session_id)
    }

    /// Update agent last_event_at timestamp (for keeping session "alive")
    pub fn touch_agent_event(&self, session_id: &str) -> Result<()> {
        AgentStateStore::new(&self.db).touch_agent_event(session_id)
    }

    /// List sessions with active agents (not idle)
    pub fn list_active_sessions(&self) -> Result<Vec<(String, AgentState)>> {
        AgentStateStore::new(&self.db).list_active_sessions()
    }

    /// List active sessions with session metadata and optional ownership filtering.
    pub fn list_active_session_details_for_user(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<(SessionInfo, AgentState)>> {
        let mut stmt = if user_id.is_some() {
            self.db.conn().prepare(LIST_ACTIVE_SESSIONS_SQL_BY_USER)?
        } else {
            self.db.conn().prepare(LIST_ACTIVE_SESSIONS_SQL_ALL)?
        };

        let rows = if let Some(user_id) = user_id {
            stmt.query_map([user_id], Self::map_active_session_row)?
        } else {
            stmt.query_map([], Self::map_active_session_row)?
        };

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Load compact runtime trace events for a session.
    pub fn load_runtime_trace_events(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<RuntimeTraceEvent>> {
        RuntimeTraceStore::new(&self.db).list_events(session_id, limit)
    }

    /// Load compact runtime trace events after a known sequence.
    pub fn load_runtime_trace_events_after(
        &self,
        session_id: &str,
        after_sequence: i64,
        limit: Option<usize>,
    ) -> Result<Vec<RuntimeTraceEvent>> {
        RuntimeTraceStore::new(&self.db).list_events_after(session_id, after_sequence, limit)
    }

    /// Load replay-friendly runtime trace summary for a session.
    pub fn load_runtime_trace_summary(&self, session_id: &str) -> Result<RuntimeTraceSummary> {
        RuntimeTraceStore::new(&self.db).summarize_session(session_id)
    }

    /// Load the most recent persisted runtime trace sequence for a session.
    pub fn load_runtime_trace_latest_sequence(&self, session_id: &str) -> Result<Option<i64>> {
        RuntimeTraceStore::new(&self.db).latest_sequence(session_id)
    }
}
