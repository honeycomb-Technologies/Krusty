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
                payload_json,
                failure_category,
                stop_reason,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                &event.run_id,
                event.sequence,
                event.turn as i64,
                &event.event_type,
                payload_json,
                failure_category,
                stop_reason,
                &event.created_at
            ],
        )?;
        Ok(())
    }

    pub fn list_events(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<RuntimeTraceEvent>> {
        let mut events = if let Some(limit) = limit {
            let sql = "SELECT run_id, sequence, turn, event_type, payload_json, failure_category, stop_reason, created_at
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
            let sql = "SELECT run_id, sequence, turn, event_type, payload_json, failure_category, stop_reason, created_at
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
            "SELECT run_id, sequence, turn, event_type, payload_json, failure_category, stop_reason, created_at
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
        payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
        failure_category,
        stop_reason,
        created_at: row.get(7)?,
    })
}
