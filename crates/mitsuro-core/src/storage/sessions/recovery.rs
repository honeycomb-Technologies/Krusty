use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

use super::super::{
    ContinuationClaimSnapshot, PendingInteractionSnapshot, RecoveryStatus, SessionRecoveryState,
};
use super::SessionManager;

const CONTINUATION_RESUMING_AGENT_STATE: &str = "resuming_input";

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

    /// Atomically claim an exact human-input continuation.
    ///
    /// The persisted recovery row is the authorization source of truth. Agent
    /// runtime state is used only as a transient execution lease: a server
    /// restart may reset it to `idle` while durable human input remains
    /// actionable. The `IMMEDIATE` transaction serializes competing server
    /// processes. The accepted response is written into the recovery snapshot
    /// in the same transaction that fences that lease, so a crash before run
    /// start cannot consume or forget the response.
    ///
    /// Awaiting-input recovery is deliberately a one-pending-interaction
    /// contract. Accepting one item from a multi-item snapshot would either
    /// discard unanswered prompts when the run resumes or make one response
    /// replayable. Such snapshots fail closed and remain untouched.
    pub fn claim_awaiting_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        accepted_response: &str,
    ) -> Result<Option<(SessionRecoveryState, PendingInteractionSnapshot)>> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT recovery_json, agent_state FROM sessions WHERE id = ?1",
                [session_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;

        let Some((Some(recovery_json), agent_state)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        let mut recovery: SessionRecoveryState = serde_json::from_str(&recovery_json)?;
        if recovery.status != RecoveryStatus::AwaitingInput {
            tx.commit()?;
            return Ok(None);
        }
        if recovery.pending_interactions.len() != 1 {
            bail!(
                "invalid awaiting-input recovery for session {session_id}: expected exactly one pending interaction, found {}",
                recovery.pending_interactions.len()
            );
        }

        let pending = recovery.pending_interactions[0].clone();
        let matches_exact_interaction = matches!(
            &pending,
            PendingInteractionSnapshot::AskUserQuestion { tool_call_id, .. }
                | PendingInteractionSnapshot::PlanConfirm { tool_call_id, .. }
                if tool_call_id == interaction_id
        );
        if !matches_exact_interaction {
            tx.commit()?;
            return Ok(None);
        }

        let now = Utc::now().to_rfc3339();
        if let Some(existing_claim) = recovery.continuation_claim.as_ref() {
            let same_accepted_response = existing_claim.interaction_id == interaction_id
                && existing_claim.accepted_response == accepted_response;
            // A durable accepted response may be reclaimed only after startup
            // repair (or an ordinary failed start) has yielded the transient
            // lease back to idle. A live resumer, or a different answer, loses
            // the race without changing any state.
            if !same_accepted_response || agent_state != "idle" {
                tx.commit()?;
                return Ok(None);
            }
            let changed = tx.execute(
                "UPDATE sessions
                 SET agent_state = ?1, agent_started_at = ?2,
                     agent_last_event_at = ?2, updated_at = ?2
                 WHERE id = ?3 AND recovery_json = ?4 AND agent_state = 'idle'",
                params![
                    CONTINUATION_RESUMING_AGENT_STATE,
                    now,
                    session_id,
                    recovery_json
                ],
            )?;
            if changed != 1 {
                tx.commit()?;
                return Ok(None);
            }
            tx.commit()?;
            return Ok(Some((recovery, pending)));
        }

        if !matches!(agent_state.as_str(), "awaiting_input" | "idle") {
            tx.commit()?;
            return Ok(None);
        }
        recovery.continuation_claim = Some(ContinuationClaimSnapshot {
            interaction_id: interaction_id.to_string(),
            accepted_response: accepted_response.to_string(),
            claimed_at: now.clone(),
        });
        let claimed_recovery_json = serde_json::to_string(&recovery)?;
        let changed = tx.execute(
            "UPDATE sessions
             SET recovery_json = ?1, agent_state = ?2,
                 agent_started_at = ?3, agent_last_event_at = ?3, updated_at = ?3
             WHERE id = ?4 AND recovery_json = ?5 AND agent_state = ?6",
            params![
                claimed_recovery_json,
                CONTINUATION_RESUMING_AGENT_STATE,
                now,
                session_id,
                recovery_json,
                agent_state
            ],
        )?;
        if changed != 1 {
            tx.commit()?;
            return Ok(None);
        }
        tx.commit()?;

        Ok(Some((recovery, pending)))
    }

    /// Yield only the transient execution lease after a claimed continuation
    /// fails before the orchestrator starts. The accepted response remains
    /// durable and can be reclaimed with the exact same value after restart or
    /// retry; a different answer cannot replace an already accepted response.
    pub fn yield_awaiting_interaction_claim(
        &self,
        session_id: &str,
        interaction_id: &str,
        accepted_response: &str,
    ) -> Result<bool> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT recovery_json, agent_state FROM sessions WHERE id = ?1",
                [session_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((Some(recovery_json), agent_state)) = row else {
            tx.commit()?;
            return Ok(false);
        };
        let recovery: SessionRecoveryState = serde_json::from_str(&recovery_json)?;
        let matching_claim = recovery.continuation_claim.as_ref().is_some_and(|claim| {
            claim.interaction_id == interaction_id
                && claim.accepted_response == accepted_response
                && recovery.status == RecoveryStatus::AwaitingInput
        });
        if !matching_claim || agent_state != CONTINUATION_RESUMING_AGENT_STATE {
            tx.commit()?;
            return Ok(false);
        }

        let now = Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE sessions
             SET agent_state = 'idle', agent_started_at = NULL,
                 agent_last_event_at = NULL, updated_at = ?1
             WHERE id = ?2 AND recovery_json = ?3 AND agent_state = ?4",
            params![
                now,
                session_id,
                recovery_json,
                CONTINUATION_RESUMING_AGENT_STATE
            ],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Reset non-idle HTTP-owned agent execution state after an unclean
    /// shutdown. Hive session recovery belongs to the standalone daemon and
    /// must not be rewritten by an HTTP-server restart.
    pub fn reset_transient_agent_states(&self) -> Result<usize> {
        let repaired = self.db.conn().execute(
            "UPDATE sessions
             SET agent_state = 'idle',
                 agent_started_at = NULL,
                 agent_last_event_at = NULL
             WHERE agent_state != 'idle'
               AND session_type != 'hive'",
            [],
        )?;
        Ok(repaired)
    }

    /// Clear persisted non-resumable recovery snapshots that should not survive
    /// a fresh server start. Actionable pending human interactions are preserved
    /// so reload/restart can surface them without resuming tool execution. Hive
    /// snapshots are daemon-owned and are never cleared by this HTTP repair.
    pub fn clear_stale_transient_recovery_states(&self) -> Result<usize> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, recovery_json
             FROM sessions
             WHERE recovery_json IS NOT NULL
               AND session_type != 'hive'",
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
