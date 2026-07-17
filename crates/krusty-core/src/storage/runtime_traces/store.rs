use anyhow::Result;
use rusqlite::params;
use serde_json::json;

use super::model::{RuntimeTraceEvent, TraceFailureCategory};
use super::summary::RuntimeTraceSummary;
use crate::agent::loop_events::LoopStopReason;
use crate::storage::database::Database;

/// Storage access for runtime traces.
pub struct RuntimeTraceStore<'a> {
    db: &'a Database,
}

impl<'a> RuntimeTraceStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn next_sequence(&self, session_id: &str) -> Result<i64> {
        let next = self.db.conn().query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1
             FROM runtime_traces
             WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(next)
    }

    pub fn append_event(&self, session_id: &str, event: &RuntimeTraceEvent) -> Result<()> {
        let payload_json = serde_json::to_string(&event.payload)?;
        let failure_category = event
            .failure_category
            .as_ref()
            .map(|category| category.as_str().to_string());
        let stop_reason = event.stop_reason.as_ref().map(|reason| match reason {
            LoopStopReason::Completed => "completed",
            LoopStopReason::AwaitingInput => "awaiting_input",
            LoopStopReason::Sleeping => "sleeping",
            LoopStopReason::BudgetExhausted => "budget_exhausted",
            LoopStopReason::ProviderError => "provider_error",
            LoopStopReason::LoopGuardTriggered => "loop_guard_triggered",
            LoopStopReason::StreamIdleTimeout => "stream_idle_timeout",
            LoopStopReason::UserAbort => "user_abort",
            LoopStopReason::Pinched => "pinched",
            LoopStopReason::PinchFailed => "pinch_failed",
        });

        self.db.conn().execute(
            "INSERT INTO runtime_traces (
                session_id,
                run_id,
                sequence,
                turn,
                event_type,
                call_kind,
                operation,
                payload_json,
                failure_category,
                stop_reason,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                session_id,
                &event.run_id,
                event.sequence,
                event.turn as i64,
                &event.event_type,
                &event.call_kind,
                &event.operation,
                payload_json,
                failure_category,
                stop_reason,
                &event.created_at
            ],
        )?;
        Ok(())
    }

    /// Append an event while allocating its session sequence inside the same
    /// SQLite statement.
    ///
    /// A session can briefly have two trace forwarders during a fast follow-up
    /// turn (for example while first-turn title generation drains). Computing
    /// `MAX(sequence) + 1` on each connection before inserting races in that
    /// case. Keeping allocation and insertion in one write statement lets
    /// SQLite serialize the writers and guarantees a unique monotonic value.
    pub fn append_event_with_next_sequence(
        &self,
        session_id: &str,
        event: &RuntimeTraceEvent,
    ) -> Result<i64> {
        let payload_json = serde_json::to_string(&event.payload)?;
        let failure_category = event
            .failure_category
            .as_ref()
            .map(|category| category.as_str().to_string());
        let stop_reason = event.stop_reason.as_ref().map(|reason| match reason {
            LoopStopReason::Completed => "completed",
            LoopStopReason::AwaitingInput => "awaiting_input",
            LoopStopReason::Sleeping => "sleeping",
            LoopStopReason::BudgetExhausted => "budget_exhausted",
            LoopStopReason::ProviderError => "provider_error",
            LoopStopReason::LoopGuardTriggered => "loop_guard_triggered",
            LoopStopReason::StreamIdleTimeout => "stream_idle_timeout",
            LoopStopReason::UserAbort => "user_abort",
            LoopStopReason::Pinched => "pinched",
            LoopStopReason::PinchFailed => "pinch_failed",
        });

        let sequence = self.db.conn().query_row(
            "INSERT INTO runtime_traces (
                session_id,
                run_id,
                sequence,
                turn,
                event_type,
                call_kind,
                operation,
                payload_json,
                failure_category,
                stop_reason,
                created_at
            )
            SELECT
                ?1,
                ?2,
                COALESCE(MAX(sequence), 0) + 1,
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                ?8,
                ?9,
                ?10
            FROM runtime_traces
            WHERE session_id = ?1
            RETURNING sequence",
            params![
                session_id,
                &event.run_id,
                event.turn as i64,
                &event.event_type,
                &event.call_kind,
                &event.operation,
                payload_json,
                failure_category,
                stop_reason,
                &event.created_at
            ],
            |row| row.get(0),
        )?;
        Ok(sequence)
    }

    /// Append a compact batch while preserving the session-global sequence.
    ///
    /// Every insert derives its sequence in the write statement itself. The
    /// first insert acquires SQLite's writer lock, so concurrent forwarders
    /// cannot allocate the same `MAX(sequence) + 1` value between a separate
    /// read and write.
    pub fn append_events_with_next_sequences(
        &self,
        session_id: &str,
        events: &[RuntimeTraceEvent],
    ) -> Result<Vec<i64>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }

        let transaction = self.db.conn().unchecked_transaction()?;
        let mut sequences = Vec::with_capacity(events.len());
        for event in events {
            let payload_json = serde_json::to_string(&event.payload)?;
            let failure_category = event
                .failure_category
                .as_ref()
                .map(|category| category.as_str().to_string());
            let stop_reason = event.stop_reason.as_ref().map(stop_reason_name);
            let sequence = transaction.query_row(
                "INSERT INTO runtime_traces (
                    session_id,
                    run_id,
                    sequence,
                    turn,
                    event_type,
                    call_kind,
                    operation,
                    payload_json,
                    failure_category,
                    stop_reason,
                    created_at
                )
                SELECT
                    ?1,
                    ?2,
                    COALESCE(MAX(sequence), 0) + 1,
                    ?3,
                    ?4,
                    ?5,
                    ?6,
                    ?7,
                    ?8,
                    ?9,
                    ?10
                FROM runtime_traces
                WHERE session_id = ?1
                RETURNING sequence",
                params![
                    session_id,
                    &event.run_id,
                    event.turn as i64,
                    &event.event_type,
                    &event.call_kind,
                    &event.operation,
                    payload_json,
                    failure_category,
                    stop_reason,
                    &event.created_at
                ],
                |row| row.get(0),
            )?;
            sequences.push(sequence);
        }
        transaction.commit()?;
        Ok(sequences)
    }

    /// Retain only the newest trace rows for a session. Sequence values are
    /// intentionally not renumbered, so incremental consumers can detect that
    /// an old cursor has fallen outside the retained window.
    pub fn prune_session_to_latest(&self, session_id: &str, keep: usize) -> Result<usize> {
        if keep == 0 {
            return Ok(self.db.conn().execute(
                "DELETE FROM runtime_traces WHERE session_id = ?1",
                [session_id],
            )?);
        }

        let deleted = self.db.conn().execute(
            "DELETE FROM runtime_traces
             WHERE session_id = ?1
               AND sequence < (
                   SELECT COALESCE(MAX(sequence), 0) - ?2 + 1
                   FROM runtime_traces
                   WHERE session_id = ?1
               )",
            params![session_id, keep as i64],
        )?;
        Ok(deleted)
    }

    pub fn list_events(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<RuntimeTraceEvent>> {
        let mut events = if let Some(limit) = limit {
            let sql = "SELECT run_id, sequence, turn, event_type, payload_json, failure_category, stop_reason, created_at, call_kind, operation
                 FROM runtime_traces
                 WHERE session_id = ?1
                 ORDER BY sequence DESC
                 LIMIT ?2";
            let mut stmt = self.db.conn().prepare(sql)?;
            let rows = stmt.query_map(params![session_id, limit as i64], map_trace_row)?;
            let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            events.reverse();
            events
        } else {
            let sql = "SELECT run_id, sequence, turn, event_type, payload_json, failure_category, stop_reason, created_at, call_kind, operation
                 FROM runtime_traces
                 WHERE session_id = ?1
                 ORDER BY sequence ASC";
            let mut stmt = self.db.conn().prepare(sql)?;
            let rows = stmt.query_map([session_id], map_trace_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        if events.is_empty() {
            return Ok(events);
        }

        events.shrink_to_fit();
        Ok(events)
    }

    pub fn list_events_after(
        &self,
        session_id: &str,
        after_sequence: i64,
        limit: Option<usize>,
    ) -> Result<Vec<RuntimeTraceEvent>> {
        let mut sql = String::from(
            "SELECT run_id, sequence, turn, event_type, payload_json, failure_category, stop_reason, created_at, call_kind, operation
             FROM runtime_traces
             WHERE session_id = ?1
               AND sequence > ?2
             ORDER BY sequence ASC",
        );

        if limit.is_some() {
            sql.push_str(" LIMIT ?3");
        }

        let mut stmt = self.db.conn().prepare(&sql)?;
        let rows = if let Some(limit) = limit {
            stmt.query_map(
                params![session_id, after_sequence, limit as i64],
                map_trace_row,
            )?
        } else {
            stmt.query_map(params![session_id, after_sequence], map_trace_row)?
        };
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        events.shrink_to_fit();
        Ok(events)
    }

    pub fn latest_sequence(&self, session_id: &str) -> Result<Option<i64>> {
        let latest = self.db.conn().query_row(
            "SELECT MAX(sequence)
             FROM runtime_traces
             WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(latest)
    }

    pub fn summarize_session(&self, session_id: &str) -> Result<RuntimeTraceSummary> {
        let events = self.list_events(session_id, None)?;
        Ok(RuntimeTraceSummary::from_events(&events))
    }

    pub fn summarize_latest_run(&self, session_id: &str) -> Result<RuntimeTraceSummary> {
        let events = self.list_events(session_id, None)?;
        let Some(run_id) = events.last().map(|event| event.run_id.clone()) else {
            return Ok(RuntimeTraceSummary::default());
        };
        let latest_events = events
            .into_iter()
            .filter(|event| event.run_id == run_id)
            .collect::<Vec<_>>();
        Ok(RuntimeTraceSummary::from_events(&latest_events))
    }
}

fn stop_reason_name(reason: &LoopStopReason) -> &'static str {
    match reason {
        LoopStopReason::Completed => "completed",
        LoopStopReason::AwaitingInput => "awaiting_input",
        LoopStopReason::Sleeping => "sleeping",
        LoopStopReason::BudgetExhausted => "budget_exhausted",
        LoopStopReason::ProviderError => "provider_error",
        LoopStopReason::LoopGuardTriggered => "loop_guard_triggered",
        LoopStopReason::StreamIdleTimeout => "stream_idle_timeout",
        LoopStopReason::UserAbort => "user_abort",
        LoopStopReason::Pinched => "pinched",
        LoopStopReason::PinchFailed => "pinch_failed",
    }
}

fn map_trace_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeTraceEvent> {
    let payload_json: String = row.get(4)?;
    let failure_category = row
        .get::<_, Option<String>>(5)?
        .and_then(|raw| TraceFailureCategory::from_str(&raw));
    let stop_reason = row
        .get::<_, Option<String>>(6)?
        .and_then(|raw| match raw.as_str() {
            "completed" => Some(LoopStopReason::Completed),
            "awaiting_input" => Some(LoopStopReason::AwaitingInput),
            "sleeping" => Some(LoopStopReason::Sleeping),
            "budget_exhausted" => Some(LoopStopReason::BudgetExhausted),
            "provider_error" => Some(LoopStopReason::ProviderError),
            "loop_guard_triggered" => Some(LoopStopReason::LoopGuardTriggered),
            "stream_idle_timeout" => Some(LoopStopReason::StreamIdleTimeout),
            "user_abort" => Some(LoopStopReason::UserAbort),
            "pinched" => Some(LoopStopReason::Pinched),
            "pinch_failed" | "context_compaction_failed" => Some(LoopStopReason::PinchFailed),
            _ => None,
        });

    Ok(RuntimeTraceEvent {
        run_id: row.get(0)?,
        sequence: row.get(1)?,
        turn: row.get::<_, i64>(2)? as usize,
        event_type: row.get(3)?,
        call_kind: row.get(8)?,
        operation: row.get(9)?,
        payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
        failure_category,
        stop_reason,
        created_at: row.get(7)?,
    })
}
