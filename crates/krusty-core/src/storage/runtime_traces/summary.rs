use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::model::{RuntimeTraceEvent, TraceFailureCategory};
use crate::agent::loop_events::LoopStopReason;

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
    pub session_pinches: usize,
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
                    | TraceFailureCategory::PinchFailed
                    | TraceFailureCategory::UserAbort => {}
                }
            }

            if event.event_type == "tool_call_complete" {
                summary.tool_calls += 1;
            }
            if event.event_type == "awaiting_input" {
                summary.awaiting_input_events += 1;
            }
            if matches!(
                event.event_type.as_str(),
                "session_pinched" | "context_compacted"
            ) {
                summary.session_pinches += 1;
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
    pub min_session_pinches: usize,
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
            min_session_pinches: 0,
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
        if summary.session_pinches < self.min_session_pinches {
            violations.push(format!(
                "session pinches {} fell below minimum {}",
                summary.session_pinches, self.min_session_pinches
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
