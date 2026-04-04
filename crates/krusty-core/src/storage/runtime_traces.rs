//! Structured runtime traces for replay, diagnostics, and regression gating.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::database::Database;
use crate::agent::loop_events::{LoopEvent, LoopStopReason};

/// Canonical failure taxonomy for agent runtime traces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceFailureCategory {
    AgentError,
    ProviderError,
    BudgetExhausted,
    LoopGuardTriggered,
    StreamIdleTimeout,
    ContextCompactionFailed,
    UserAbort,
    ToolExecutionError,
    ServerToolError,
    ToolDenied,
}

impl TraceFailureCategory {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AgentError => "agent_error",
            Self::ProviderError => "provider_error",
            Self::BudgetExhausted => "budget_exhausted",
            Self::LoopGuardTriggered => "loop_guard_triggered",
            Self::StreamIdleTimeout => "stream_idle_timeout",
            Self::ContextCompactionFailed => "context_compaction_failed",
            Self::UserAbort => "user_abort",
            Self::ToolExecutionError => "tool_execution_error",
            Self::ServerToolError => "server_tool_error",
            Self::ToolDenied => "tool_denied",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "agent_error" => Some(Self::AgentError),
            "provider_error" => Some(Self::ProviderError),
            "budget_exhausted" => Some(Self::BudgetExhausted),
            "loop_guard_triggered" => Some(Self::LoopGuardTriggered),
            "stream_idle_timeout" => Some(Self::StreamIdleTimeout),
            "context_compaction_failed" => Some(Self::ContextCompactionFailed),
            "user_abort" => Some(Self::UserAbort),
            "tool_execution_error" => Some(Self::ToolExecutionError),
            "server_tool_error" => Some(Self::ServerToolError),
            "tool_denied" => Some(Self::ToolDenied),
            _ => None,
        }
    }
}

/// Compact persisted trace event derived from a canonical `LoopEvent`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTraceEvent {
    pub run_id: String,
    pub sequence: i64,
    pub turn: usize,
    pub event_type: String,
    pub payload: Value,
    pub failure_category: Option<TraceFailureCategory>,
    pub stop_reason: Option<LoopStopReason>,
    pub created_at: String,
}

impl RuntimeTraceEvent {
    pub fn from_loop_event(
        run_id: impl Into<String>,
        sequence: i64,
        turn: usize,
        event: &LoopEvent,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            sequence,
            turn,
            event_type: loop_event_type(event).to_string(),
            payload: summarize_loop_event(event),
            failure_category: failure_category_for_event(event),
            stop_reason: stop_reason_for_event(event),
            created_at: Utc::now().to_rfc3339(),
        }
    }
}

/// Stable count summary per event type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceEventCount {
    pub event_type: String,
    pub count: usize,
}

/// Session-level replay summary built from runtime traces.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTraceSummary {
    pub total_events: usize,
    pub total_runs: usize,
    pub total_turns: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub server_tool_errors: usize,
    pub agent_errors: usize,
    pub provider_failures: usize,
    pub approval_denials: usize,
    pub awaiting_input_events: usize,
    pub context_compactions: usize,
    pub last_stop_reason: Option<LoopStopReason>,
    pub failure_categories: Vec<TraceFailureCategory>,
    pub event_counts: Vec<TraceEventCount>,
}

impl RuntimeTraceSummary {
    pub fn from_events(events: &[RuntimeTraceEvent]) -> Self {
        let mut run_turns: BTreeMap<&str, usize> = BTreeMap::new();
        let mut run_ids = BTreeSet::new();
        let mut event_counts: BTreeMap<&str, usize> = BTreeMap::new();
        let mut failure_categories = BTreeSet::new();
        let mut summary = Self {
            total_events: events.len(),
            ..Self::default()
        };

        for event in events {
            run_ids.insert(event.run_id.as_str());
            run_turns
                .entry(event.run_id.as_str())
                .and_modify(|max_turn| *max_turn = (*max_turn).max(event.turn))
                .or_insert(event.turn);
            *event_counts.entry(event.event_type.as_str()).or_insert(0) += 1;

            if let Some(category) = event.failure_category.clone() {
                failure_categories.insert(category.clone());
                match category {
                    TraceFailureCategory::AgentError => summary.agent_errors += 1,
                    TraceFailureCategory::ProviderError => summary.provider_failures += 1,
                    TraceFailureCategory::ToolExecutionError => summary.tool_errors += 1,
                    TraceFailureCategory::ServerToolError => summary.server_tool_errors += 1,
                    TraceFailureCategory::ToolDenied => summary.approval_denials += 1,
                    TraceFailureCategory::BudgetExhausted
                    | TraceFailureCategory::LoopGuardTriggered
                    | TraceFailureCategory::StreamIdleTimeout
                    | TraceFailureCategory::ContextCompactionFailed
                    | TraceFailureCategory::UserAbort => {}
                }
            }

            if event.event_type == "tool_call_complete" {
                summary.tool_calls += 1;
            }
            if event.event_type == "awaiting_input" {
                summary.awaiting_input_events += 1;
            }
            if event.event_type == "context_compacted" {
                summary.context_compactions += 1;
            }
            if let Some(stop_reason) = event.stop_reason.clone() {
                summary.last_stop_reason = Some(stop_reason);
            }
        }

        summary.total_runs = run_ids.len();
        summary.total_turns = run_turns.values().copied().sum();
        summary.failure_categories = failure_categories.into_iter().collect();
        summary.event_counts = event_counts
            .into_iter()
            .map(|(event_type, count)| TraceEventCount {
                event_type: event_type.to_string(),
                count,
            })
            .collect();
        summary
    }

    pub fn event_count(&self, event_type: &str) -> usize {
        self.event_counts
            .iter()
            .find(|entry| entry.event_type == event_type)
            .map(|entry| entry.count)
            .unwrap_or(0)
    }
}

/// Expectations for replay-backed regression gating.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayExpectations {
    pub allowed_terminal_reasons: Vec<LoopStopReason>,
    pub max_agent_errors: usize,
    pub max_provider_failures: usize,
    pub max_tool_errors: usize,
    pub max_server_tool_errors: usize,
    pub min_total_runs: usize,
    pub min_total_turns: usize,
    pub min_context_compactions: usize,
    pub min_awaiting_input_events: usize,
    pub required_event_types: Vec<String>,
}

impl ReplayExpectations {
    pub fn strict() -> Self {
        Self {
            allowed_terminal_reasons: vec![
                LoopStopReason::Completed,
                LoopStopReason::AwaitingInput,
            ],
            max_agent_errors: 0,
            max_provider_failures: 0,
            max_tool_errors: 0,
            max_server_tool_errors: 0,
            min_total_runs: 1,
            min_total_turns: 1,
            min_context_compactions: 0,
            min_awaiting_input_events: 0,
            required_event_types: Vec::new(),
        }
    }

    pub fn evaluate(&self, summary: &RuntimeTraceSummary) -> ReplayGateResult {
        let mut violations = Vec::new();

        if summary.total_events == 0 {
            violations.push("no runtime traces recorded".to_string());
        }

        if !self.allowed_terminal_reasons.is_empty() {
            match summary.last_stop_reason.as_ref() {
                Some(stop_reason) if self.allowed_terminal_reasons.contains(stop_reason) => {}
                Some(stop_reason) => violations.push(format!(
                    "terminal reason {:?} is outside the allowed replay gate",
                    stop_reason
                )),
                None => {
                    violations.push("runtime trace is missing a terminal stop reason".to_string())
                }
            }
        }

        if summary.agent_errors > self.max_agent_errors {
            violations.push(format!(
                "agent errors {} exceeded limit {}",
                summary.agent_errors, self.max_agent_errors
            ));
        }
        if summary.provider_failures > self.max_provider_failures {
            violations.push(format!(
                "provider failures {} exceeded limit {}",
                summary.provider_failures, self.max_provider_failures
            ));
        }
        if summary.tool_errors > self.max_tool_errors {
            violations.push(format!(
                "tool errors {} exceeded limit {}",
                summary.tool_errors, self.max_tool_errors
            ));
        }
        if summary.server_tool_errors > self.max_server_tool_errors {
            violations.push(format!(
                "server tool errors {} exceeded limit {}",
                summary.server_tool_errors, self.max_server_tool_errors
            ));
        }
        if summary.total_runs < self.min_total_runs {
            violations.push(format!(
                "total runs {} fell below minimum {}",
                summary.total_runs, self.min_total_runs
            ));
        }
        if summary.total_turns < self.min_total_turns {
            violations.push(format!(
                "total turns {} fell below minimum {}",
                summary.total_turns, self.min_total_turns
            ));
        }
        if summary.context_compactions < self.min_context_compactions {
            violations.push(format!(
                "context compactions {} fell below minimum {}",
                summary.context_compactions, self.min_context_compactions
            ));
        }
        if summary.awaiting_input_events < self.min_awaiting_input_events {
            violations.push(format!(
                "awaiting input events {} fell below minimum {}",
                summary.awaiting_input_events, self.min_awaiting_input_events
            ));
        }
        for event_type in &self.required_event_types {
            if summary.event_count(event_type) == 0 {
                violations.push(format!("required event type {event_type} was not recorded"));
            }
        }

        ReplayGateResult {
            passed: violations.is_empty(),
            violations,
        }
    }
}

/// Result of evaluating a replay summary against expectations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayGateResult {
    pub passed: bool,
    pub violations: Vec<String>,
}

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
            LoopStopReason::ContextCompactionFailed => "context_compaction_failed",
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
            "context_compaction_failed" => Some(LoopStopReason::ContextCompactionFailed),
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

fn loop_event_type(event: &LoopEvent) -> &'static str {
    match event {
        LoopEvent::TextDelta { .. } => "text_delta",
        LoopEvent::TextDeltaWithCitations { .. } => "text_delta_with_citations",
        LoopEvent::ThinkingDelta { .. } => "thinking_delta",
        LoopEvent::ThinkingComplete { .. } => "thinking_complete",
        LoopEvent::ToolCallStart { .. } => "tool_call_start",
        LoopEvent::ToolCallComplete { .. } => "tool_call_complete",
        LoopEvent::ToolExecuting { .. } => "tool_executing",
        LoopEvent::ToolOutputDelta { .. } => "tool_output_delta",
        LoopEvent::ToolResult { .. } => "tool_result",
        LoopEvent::AwaitingInput { .. } => "awaiting_input",
        LoopEvent::ToolApprovalRequired { .. } => "tool_approval_required",
        LoopEvent::ToolApproved { .. } => "tool_approved",
        LoopEvent::ToolDenied { .. } => "tool_denied",
        LoopEvent::ServerToolStart { .. } => "server_tool_start",
        LoopEvent::ServerToolComplete { .. } => "server_tool_complete",
        LoopEvent::WebSearchResults { .. } => "web_search_results",
        LoopEvent::WebFetchResult { .. } => "web_fetch_result",
        LoopEvent::ServerToolError { .. } => "server_tool_error",
        LoopEvent::ModeChange { .. } => "mode_change",
        LoopEvent::PlanUpdate { .. } => "plan_update",
        LoopEvent::PlanComplete { .. } => "plan_complete",
        LoopEvent::AgentSleeping { .. } => "agent_sleeping",
        LoopEvent::TurnComplete { .. } => "turn_complete",
        LoopEvent::TickInjected { .. } => "tick_injected",
        LoopEvent::Usage { .. } => "usage",
        LoopEvent::ContextCompacted { .. } => "context_compacted",
        LoopEvent::TitleGenerated { .. } => "title_generated",
        LoopEvent::Finished { .. } => "finished",
        LoopEvent::Error { .. } => "error",
        LoopEvent::AgentBackgroundStarted { .. } => "agent_background_started",
        LoopEvent::AgentBackgroundCompleted { .. } => "agent_background_completed",
        LoopEvent::UserMessage { .. } => "user_message",
        LoopEvent::ClassifierDecision { .. } => "classifier_decision",
        LoopEvent::TeammateSpawned { .. } => "teammate_spawned",
        LoopEvent::TeammateTaskCompleted { .. } => "teammate_task_completed",
        LoopEvent::TeammateTaskFailed { .. } => "teammate_task_failed",
        LoopEvent::TeammateCancelled { .. } => "teammate_cancelled",
    }
}

fn summarize_loop_event(event: &LoopEvent) -> Value {
    match event {
        LoopEvent::TextDelta { delta } => json!({ "chars": delta.chars().count() }),
        LoopEvent::TextDeltaWithCitations { delta, citations } => {
            json!({ "chars": delta.chars().count(), "citations": citations.len() })
        }
        LoopEvent::ThinkingDelta { thinking } => json!({ "chars": thinking.chars().count() }),
        LoopEvent::ThinkingComplete { thinking, .. } => {
            json!({ "chars": thinking.chars().count() })
        }
        LoopEvent::ToolCallStart { id, name } => json!({ "id": id, "name": name }),
        LoopEvent::ToolCallComplete {
            id,
            name,
            arguments,
        } => json!({
            "id": id,
            "name": name,
            "arguments": summarize_json_shape(arguments),
        }),
        LoopEvent::ToolExecuting { id, name } => json!({ "id": id, "name": name }),
        LoopEvent::ToolOutputDelta { id, delta } => {
            json!({ "id": id, "chars": delta.chars().count() })
        }
        LoopEvent::ToolResult {
            id,
            output,
            is_error,
        } => json!({
            "id": id,
            "is_error": is_error,
            "output_chars": output.chars().count(),
        }),
        LoopEvent::AwaitingInput {
            tool_call_id,
            tool_name,
        } => json!({ "tool_call_id": tool_call_id, "tool_name": tool_name }),
        LoopEvent::ToolApprovalRequired {
            id,
            name,
            arguments,
        } => json!({
            "id": id,
            "name": name,
            "arguments": summarize_json_shape(arguments),
        }),
        LoopEvent::ToolApproved { id } => json!({ "id": id }),
        LoopEvent::ToolDenied { id } => json!({ "id": id }),
        LoopEvent::ServerToolStart { id, name } => json!({ "id": id, "name": name }),
        LoopEvent::ServerToolComplete { id, name } => json!({ "id": id, "name": name }),
        LoopEvent::WebSearchResults {
            tool_use_id,
            results,
        } => {
            json!({ "tool_use_id": tool_use_id, "result_count": results.len() })
        }
        LoopEvent::WebFetchResult {
            tool_use_id,
            content,
        } => json!({
            "tool_use_id": tool_use_id,
            "url": content.url,
            "media_type": content.media_type,
            "content_chars": content.content.chars().count(),
        }),
        LoopEvent::ServerToolError {
            tool_use_id,
            error_code,
        } => json!({ "tool_use_id": tool_use_id, "error_code": error_code }),
        LoopEvent::ModeChange { mode, reason } => json!({ "mode": mode, "reason": reason }),
        LoopEvent::PlanUpdate { tasks } => json!({
            "task_count": tasks.len(),
            "completed_count": tasks.iter().filter(|task| task.completed).count(),
        }),
        LoopEvent::PlanComplete {
            tool_call_id,
            title,
            task_count,
        } => json!({
            "tool_call_id": tool_call_id,
            "title": title,
            "task_count": task_count,
        }),
        LoopEvent::AgentSleeping {
            duration_secs,
            reason,
        } => json!({
            "duration_secs": duration_secs,
            "reason": reason,
        }),
        LoopEvent::TurnComplete { turn, has_more } => {
            json!({ "turn": turn, "has_more": has_more })
        }
        LoopEvent::TickInjected { tick_number } => json!({ "tick_number": tick_number }),
        LoopEvent::Usage {
            prompt_tokens,
            completion_tokens,
        } => json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
        }),
        LoopEvent::ContextCompacted {
            reason,
            estimated_tokens_before,
            estimated_tokens_after,
            replaced_messages,
        } => json!({
            "reason": reason,
            "estimated_tokens_before": estimated_tokens_before,
            "estimated_tokens_after": estimated_tokens_after,
            "replaced_messages": replaced_messages,
        }),
        LoopEvent::TitleGenerated { title } => json!({ "title": title }),
        LoopEvent::Finished {
            session_id,
            stop_reason,
        } => json!({ "session_id": session_id, "stop_reason": stop_reason }),
        LoopEvent::Error { error } => json!({ "error": error }),
        LoopEvent::AgentBackgroundStarted {
            delegated_run_id,
            agent_type,
            description,
        } => json!({
            "delegated_run_id": delegated_run_id,
            "agent_type": agent_type,
            "description": description,
        }),
        LoopEvent::AgentBackgroundCompleted {
            delegated_run_id,
            agent_type,
            success,
            summary,
        } => json!({
            "delegated_run_id": delegated_run_id,
            "agent_type": agent_type,
            "success": success,
            "summary": summary,
        }),
        LoopEvent::UserMessage {
            title,
            message,
            level,
        } => json!({ "title": title, "message": message, "level": level }),
        LoopEvent::ClassifierDecision {
            tool_name,
            decision,
            reason,
            stage,
        } => {
            json!({ "tool_name": tool_name, "decision": decision, "reason": reason, "stage": stage })
        }
        LoopEvent::TeammateSpawned { name, role } => json!({ "name": name, "role": role }),
        LoopEvent::TeammateTaskCompleted {
            name,
            task_id,
            result,
        } => json!({ "name": name, "task_id": task_id, "result_len": result.len() }),
        LoopEvent::TeammateTaskFailed {
            name,
            task_id,
            error,
        } => json!({ "name": name, "task_id": task_id, "error": error }),
        LoopEvent::TeammateCancelled { name } => json!({ "name": name }),
    }
}

fn summarize_json_shape(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            json!({ "type": "object", "keys": keys })
        }
        Value::Array(items) => json!({ "type": "array", "len": items.len() }),
        Value::String(_) => json!({ "type": "string" }),
        Value::Number(_) => json!({ "type": "number" }),
        Value::Bool(_) => json!({ "type": "bool" }),
        Value::Null => json!({ "type": "null" }),
    }
}

fn failure_category_for_event(event: &LoopEvent) -> Option<TraceFailureCategory> {
    match event {
        LoopEvent::ToolResult { is_error: true, .. } => {
            Some(TraceFailureCategory::ToolExecutionError)
        }
        LoopEvent::ToolDenied { .. } => Some(TraceFailureCategory::ToolDenied),
        LoopEvent::ServerToolError { .. } => Some(TraceFailureCategory::ServerToolError),
        LoopEvent::Finished { stop_reason, .. } => match stop_reason {
            LoopStopReason::Completed
            | LoopStopReason::AwaitingInput
            | LoopStopReason::Sleeping => None,
            LoopStopReason::ProviderError => Some(TraceFailureCategory::ProviderError),
            LoopStopReason::BudgetExhausted => Some(TraceFailureCategory::BudgetExhausted),
            LoopStopReason::LoopGuardTriggered => Some(TraceFailureCategory::LoopGuardTriggered),
            LoopStopReason::StreamIdleTimeout => Some(TraceFailureCategory::StreamIdleTimeout),
            LoopStopReason::UserAbort => Some(TraceFailureCategory::UserAbort),
            LoopStopReason::ContextCompactionFailed => {
                Some(TraceFailureCategory::ContextCompactionFailed)
            }
        },
        LoopEvent::Error { .. } => Some(TraceFailureCategory::AgentError),
        _ => None,
    }
}

fn stop_reason_for_event(event: &LoopEvent) -> Option<LoopStopReason> {
    match event {
        LoopEvent::Finished { stop_reason, .. } => Some(stop_reason.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::TempDir;

    use super::{ReplayExpectations, RuntimeTraceEvent, RuntimeTraceStore, TraceFailureCategory};
    use crate::agent::loop_events::{LoopEvent, LoopStopReason};
    use crate::storage::database::Database;

    fn create_test_db() -> (Database, TempDir, String) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("Failed to create database");
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, "Trace Test", now, now],
            )
            .expect("Failed to create session");
        (db, temp_dir, session_id)
    }

    #[test]
    fn runtime_trace_store_round_trip() {
        let (db, _temp_dir, session_id) = create_test_db();
        let store = RuntimeTraceStore::new(&db);

        let event = RuntimeTraceEvent::from_loop_event(
            "run-1",
            1,
            1,
            &LoopEvent::ToolCallComplete {
                id: "tool-1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({ "path": "src/main.rs" }),
            },
        );
        store
            .append_event(&session_id, &event)
            .expect("trace append should succeed");

        let events = store
            .list_events(&session_id, None)
            .expect("trace list should succeed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "tool_call_complete");
        assert_eq!(events[0].payload["arguments"]["type"], "object");
    }

    #[test]
    fn runtime_trace_store_limit_returns_most_recent_events_in_order() {
        let (db, _temp_dir, session_id) = create_test_db();
        let store = RuntimeTraceStore::new(&db);

        for sequence in 1..=3 {
            let event = RuntimeTraceEvent::from_loop_event(
                "run-1",
                sequence,
                1,
                &LoopEvent::TurnComplete {
                    turn: sequence as usize,
                    has_more: sequence < 3,
                },
            );
            store
                .append_event(&session_id, &event)
                .expect("trace append should succeed");
        }

        let events = store
            .list_events(&session_id, Some(2))
            .expect("trace list should succeed");
        let sequences: Vec<i64> = events.iter().map(|event| event.sequence).collect();
        assert_eq!(sequences, vec![2, 3]);
    }

    #[test]
    fn runtime_trace_summary_classifies_failures_and_compaction() {
        let (db, _temp_dir, session_id) = create_test_db();
        let store = RuntimeTraceStore::new(&db);

        let traces = [
            RuntimeTraceEvent::from_loop_event(
                "run-1",
                1,
                1,
                &LoopEvent::ContextCompacted {
                    reason: "pressure".to_string(),
                    estimated_tokens_before: 100_000,
                    estimated_tokens_after: 70_000,
                    replaced_messages: 4,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-1",
                2,
                1,
                &LoopEvent::ToolResult {
                    id: "tool-1".to_string(),
                    output: "permission denied".to_string(),
                    is_error: true,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-1",
                3,
                1,
                &LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::ProviderError,
                },
            ),
        ];

        for event in traces {
            store
                .append_event(&session_id, &event)
                .expect("trace append should succeed");
        }

        let summary = store
            .summarize_session(&session_id)
            .expect("summary should succeed");
        assert_eq!(summary.total_events, 3);
        assert_eq!(summary.total_runs, 1);
        assert_eq!(summary.total_turns, 1);
        assert_eq!(summary.tool_errors, 1);
        assert_eq!(summary.provider_failures, 1);
        assert_eq!(summary.context_compactions, 1);
        assert!(summary
            .failure_categories
            .contains(&TraceFailureCategory::ToolExecutionError));
        assert!(summary
            .failure_categories
            .contains(&TraceFailureCategory::ProviderError));
    }

    #[test]
    fn replay_gate_accepts_long_session_workload_with_compaction() {
        let (db, _temp_dir, session_id) = create_test_db();
        let store = RuntimeTraceStore::new(&db);

        let traces = [
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                1,
                1,
                &LoopEvent::ToolCallComplete {
                    id: "tool-1".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({ "file_path": "src/main.rs" }),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                2,
                1,
                &LoopEvent::ToolResult {
                    id: "tool-1".to_string(),
                    output: "read ok".to_string(),
                    is_error: false,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                3,
                1,
                &LoopEvent::TurnComplete {
                    turn: 1,
                    has_more: true,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                4,
                2,
                &LoopEvent::ContextCompacted {
                    reason: "context_pressure".to_string(),
                    estimated_tokens_before: 140_000,
                    estimated_tokens_after: 82_000,
                    replaced_messages: 12,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                5,
                2,
                &LoopEvent::ToolCallComplete {
                    id: "tool-2".to_string(),
                    name: "write".to_string(),
                    arguments: serde_json::json!({ "file_path": "src/lib.rs" }),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                6,
                2,
                &LoopEvent::ToolResult {
                    id: "tool-2".to_string(),
                    output: "write ok".to_string(),
                    is_error: false,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                7,
                2,
                &LoopEvent::TurnComplete {
                    turn: 2,
                    has_more: true,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-long",
                8,
                3,
                &LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::Completed,
                },
            ),
        ];

        for event in traces {
            store
                .append_event(&session_id, &event)
                .expect("trace append should succeed");
        }

        let summary = store
            .summarize_session(&session_id)
            .expect("summary should succeed");
        let expectations = ReplayExpectations {
            min_total_turns: 3,
            min_context_compactions: 1,
            required_event_types: vec![
                "tool_call_complete".to_string(),
                "tool_result".to_string(),
                "context_compacted".to_string(),
            ],
            ..ReplayExpectations::strict()
        };

        let result = expectations.evaluate(&summary);
        assert!(
            result.passed,
            "unexpected violations: {:?}",
            result.violations
        );
        assert_eq!(summary.context_compactions, 1);
        assert_eq!(summary.tool_calls, 2);
    }

    #[test]
    fn replay_gate_accepts_approval_pause_and_resume_workload() {
        let (db, _temp_dir, session_id) = create_test_db();
        let store = RuntimeTraceStore::new(&db);

        let traces = [
            RuntimeTraceEvent::from_loop_event(
                "run-awaiting-input",
                1,
                1,
                &LoopEvent::ToolApprovalRequired {
                    id: "tool-1".to_string(),
                    name: "write".to_string(),
                    arguments: serde_json::json!({ "file_path": "src/main.rs" }),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-awaiting-input",
                2,
                1,
                &LoopEvent::AwaitingInput {
                    tool_call_id: "tool-1".to_string(),
                    tool_name: "write".to_string(),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-awaiting-input",
                3,
                1,
                &LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::AwaitingInput,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-approved",
                4,
                2,
                &LoopEvent::ToolApproved {
                    id: "tool-1".to_string(),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-approved",
                5,
                2,
                &LoopEvent::ToolExecuting {
                    id: "tool-1".to_string(),
                    name: "write".to_string(),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-approved",
                6,
                2,
                &LoopEvent::ToolResult {
                    id: "tool-1".to_string(),
                    output: "write ok".to_string(),
                    is_error: false,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-approved",
                7,
                2,
                &LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::Completed,
                },
            ),
        ];

        for event in traces {
            store
                .append_event(&session_id, &event)
                .expect("trace append should succeed");
        }

        let summary = store
            .summarize_session(&session_id)
            .expect("summary should succeed");
        let expectations = ReplayExpectations {
            min_total_runs: 2,
            min_total_turns: 2,
            min_awaiting_input_events: 1,
            required_event_types: vec![
                "tool_approval_required".to_string(),
                "awaiting_input".to_string(),
                "tool_approved".to_string(),
                "tool_result".to_string(),
            ],
            ..ReplayExpectations::strict()
        };

        let result = expectations.evaluate(&summary);
        assert!(
            result.passed,
            "unexpected violations: {:?}",
            result.violations
        );
        assert_eq!(summary.awaiting_input_events, 1);
        assert_eq!(summary.total_runs, 2);
    }

    #[test]
    fn summarize_latest_run_ignores_prior_provider_interruption_after_recovery() {
        let (db, _temp_dir, session_id) = create_test_db();
        let store = RuntimeTraceStore::new(&db);

        let traces = [
            RuntimeTraceEvent::from_loop_event(
                "run-interrupted",
                1,
                1,
                &LoopEvent::Error {
                    error: "provider disconnected".to_string(),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-interrupted",
                2,
                1,
                &LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::ProviderError,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-recovered",
                3,
                2,
                &LoopEvent::ToolCallComplete {
                    id: "tool-2".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({ "file_path": "src/lib.rs" }),
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-recovered",
                4,
                2,
                &LoopEvent::ToolResult {
                    id: "tool-2".to_string(),
                    output: "read ok".to_string(),
                    is_error: false,
                },
            ),
            RuntimeTraceEvent::from_loop_event(
                "run-recovered",
                5,
                2,
                &LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::Completed,
                },
            ),
        ];

        for event in traces {
            store
                .append_event(&session_id, &event)
                .expect("trace append should succeed");
        }

        let whole_session = store
            .summarize_session(&session_id)
            .expect("summary should succeed");
        assert_eq!(whole_session.provider_failures, 1);

        let latest_run = store
            .summarize_latest_run(&session_id)
            .expect("latest run summary should succeed");
        assert_eq!(latest_run.total_runs, 1);
        assert_eq!(latest_run.provider_failures, 0);
        assert_eq!(latest_run.last_stop_reason, Some(LoopStopReason::Completed));

        let result = ReplayExpectations::strict().evaluate(&latest_run);
        assert!(
            result.passed,
            "unexpected violations: {:?}",
            result.violations
        );
    }

    #[test]
    fn replay_gate_rejects_loop_guard_workload() {
        let failing_summary = super::RuntimeTraceSummary {
            total_events: 3,
            total_runs: 1,
            total_turns: 4,
            last_stop_reason: Some(LoopStopReason::LoopGuardTriggered),
            failure_categories: vec![TraceFailureCategory::LoopGuardTriggered],
            event_counts: vec![
                super::TraceEventCount {
                    event_type: "tool_call_complete".to_string(),
                    count: 3,
                },
                super::TraceEventCount {
                    event_type: "finished".to_string(),
                    count: 1,
                },
            ],
            ..Default::default()
        };

        let expectations = ReplayExpectations {
            min_total_turns: 2,
            required_event_types: vec!["tool_call_complete".to_string()],
            ..ReplayExpectations::strict()
        };
        let result = expectations.evaluate(&failing_summary);

        assert!(!result.passed);
        assert!(result
            .violations
            .iter()
            .any(|violation| violation.contains("terminal reason")));
    }

    #[test]
    fn replay_gate_rejects_provider_failures() {
        let failing_summary = super::RuntimeTraceSummary {
            total_events: 2,
            total_runs: 1,
            total_turns: 1,
            provider_failures: 1,
            last_stop_reason: Some(LoopStopReason::ProviderError),
            failure_categories: vec![TraceFailureCategory::ProviderError],
            ..Default::default()
        };

        let result = ReplayExpectations::strict().evaluate(&failing_summary);
        assert!(!result.passed);
        assert!(result
            .violations
            .iter()
            .any(|violation| violation.contains("terminal reason")));
    }

    #[test]
    fn latest_sequence_and_after_filter_follow_monotonic_trace_order() {
        let (db, _temp_dir, session_id) = create_test_db();
        let store = RuntimeTraceStore::new(&db);

        store
            .append_event(
                &session_id,
                &RuntimeTraceEvent::from_loop_event(
                    "run-1".to_string(),
                    1,
                    1,
                    &LoopEvent::TextDelta {
                        delta: "one".to_string(),
                    },
                ),
            )
            .expect("first event should persist");
        store
            .append_event(
                &session_id,
                &RuntimeTraceEvent::from_loop_event(
                    "run-1".to_string(),
                    2,
                    1,
                    &LoopEvent::TurnComplete {
                        turn: 1,
                        has_more: true,
                    },
                ),
            )
            .expect("second event should persist");
        store
            .append_event(
                &session_id,
                &RuntimeTraceEvent::from_loop_event(
                    "run-1".to_string(),
                    3,
                    2,
                    &LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason: LoopStopReason::Completed,
                    },
                ),
            )
            .expect("third event should persist");

        assert_eq!(
            store
                .latest_sequence(&session_id)
                .expect("latest sequence should load"),
            Some(3)
        );

        let filtered = store
            .list_events_after(&session_id, 1, Some(10))
            .expect("filtered events should load");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].sequence, 2);
        assert_eq!(filtered[1].sequence, 3);
    }
}
