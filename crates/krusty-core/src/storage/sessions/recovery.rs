use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::super::SessionRecoveryState;
use super::SessionManager;

impl SessionManager {
    /// Persist context-ledger and continuation contracts for deterministic resume.
    pub fn update_context_continuation_state(
        &self,
        session_id: &str,
        context_ledger_json: &str,
        continuation_json: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE sessions
             SET context_ledger_json = ?1, continuation_json = ?2, updated_at = ?3
             WHERE id = ?4",
            params![context_ledger_json, continuation_json, now, session_id],
        )?;
        Ok(())
    }

    /// Load persisted continuation contracts.
    pub fn load_context_continuation_state(
        &self,
        session_id: &str,
    ) -> Result<Option<(String, String)>> {
        let row = self.db.conn().query_row(
            "SELECT context_ledger_json, continuation_json
             FROM sessions
             WHERE id = ?1",
            [session_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )?;

        Ok(match row {
            (Some(ledger), Some(continuation)) => Some((ledger, continuation)),
            _ => None,
        })
    }

    /// Persist explicit interrupted-turn recovery state separately from conversation history.
    pub fn update_recovery_state(
        &self,
        session_id: &str,
        recovery_state: &SessionRecoveryState,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let recovery_json = serde_json::to_string(recovery_state)?;
        self.db.conn().execute(
            "UPDATE sessions
             SET recovery_json = ?1, updated_at = ?2
             WHERE id = ?3",
            params![recovery_json, now, session_id],
        )?;
        Ok(())
    }

    /// Load persisted interrupted-turn recovery state.
    pub fn load_recovery_state(&self, session_id: &str) -> Result<Option<SessionRecoveryState>> {
        let recovery_json = self.db.conn().query_row(
            "SELECT recovery_json
             FROM sessions
             WHERE id = ?1",
            [session_id],
            |row| row.get::<_, Option<String>>(0),
        )?;

        recovery_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(Into::into)
    }

    /// Reset non-idle HTTP-owned agent execution state after an unclean
    /// shutdown. Mako session recovery belongs to the standalone daemon and
    /// must not be rewritten by an HTTP-server restart.
    pub fn reset_transient_agent_states(&self) -> Result<usize> {
        let repaired = self.db.conn().execute(
            "UPDATE sessions
             SET agent_state = 'idle',
                 agent_started_at = NULL,
                 agent_last_event_at = NULL
             WHERE agent_state != 'idle'
               AND session_type != 'mako'",
            [],
        )?;
        Ok(repaired)
    }

    /// Clear persisted non-resumable recovery snapshots that should not survive
    /// a fresh server start. Actionable pending human interactions are preserved
    /// so reload/restart can surface them without resuming tool execution. Mako
    /// snapshots are daemon-owned and are never cleared by this HTTP repair.
    pub fn clear_stale_transient_recovery_states(&self) -> Result<usize> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, recovery_json
             FROM sessions
             WHERE recovery_json IS NOT NULL
               AND session_type != 'mako'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;

        let stale_ids = rows
            .filter_map(|row| row.ok())
            .filter_map(|(session_id, recovery_json)| {
                let recovery_json = recovery_json?;
                let state: SessionRecoveryState = serde_json::from_str(&recovery_json).ok()?;
                if state.is_resumable() || state.has_pending_interactions() {
                    None
                } else {
                    Some(session_id)
                }
            })
            .collect::<Vec<_>>();

        if stale_ids.is_empty() {
            return Ok(0);
        }

        let tx = self.db.conn().unchecked_transaction()?;
        for session_id in &stale_ids {
            tx.execute(
                "UPDATE sessions
                 SET recovery_json = NULL
                 WHERE id = ?1",
                params![session_id],
            )?;
        }
        tx.commit()?;
        Ok(stale_ids.len())
    }

    /// Clear persisted recovery state once the interrupted turn has been finalized or superseded.
    pub fn clear_recovery_state(&self, session_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE sessions
             SET recovery_json = NULL, updated_at = ?1
             WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }
}
