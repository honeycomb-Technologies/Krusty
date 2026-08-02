use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioStopReason {
    Completed,
    LoopGuardTriggered,
    ProviderError,
    BudgetExhausted,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioFixtureFile {
    pub scenarios: Vec<ScriptedScenario>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptedScenario {
    pub name: String,
    #[serde(default)]
    pub policy: ScenarioPolicy,
    pub steps: Vec<ScriptedModelStep>,
    pub expect: ExpectedMetrics,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioPolicy {
    #[serde(default = "default_provider_retries")]
    pub max_provider_retries: usize,
    #[serde(default = "default_empty_recoveries")]
    pub max_empty_recoveries: usize,
    #[serde(default = "default_overflow_recoveries")]
    pub max_overflow_recoveries: usize,
    #[serde(default = "default_no_progress_cycles")]
    pub max_no_progress_cycles: usize,
    #[serde(default)]
    pub max_provider_calls: Option<usize>,
}

fn default_provider_retries() -> usize {
    2
}

fn default_empty_recoveries() -> usize {
    1
}

fn default_overflow_recoveries() -> usize {
    1
}

fn default_no_progress_cycles() -> usize {
    3
}

impl Default for ScenarioPolicy {
    fn default() -> Self {
        Self {
            max_provider_retries: default_provider_retries(),
            max_empty_recoveries: default_empty_recoveries(),
            max_overflow_recoveries: default_overflow_recoveries(),
            max_no_progress_cycles: default_no_progress_cycles(),
            max_provider_calls: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScriptedModelStep {
    ToolBatch { calls: Vec<ScriptedToolCall> },
    Complete { text: String },
    TransientError { code: String },
    MalformedStream { detail: String },
    EmptyCompletion,
    ContextOverflow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptedToolCall {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    /// Provider-neutral semantic intent. Production adapters derive this from
    /// the concrete tool call; fixtures state it explicitly so cosmetic shell
    /// drift cannot redefine the acceptance contract.
    pub intent: String,
    pub effect: VirtualEffect,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VirtualEffect {
    pub class: EffectClass,
    #[serde(default = "default_true")]
    pub ok: bool,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub changed: bool,
    #[serde(default)]
    pub side_effect_key: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    Observe,
    Mutate,
    Validate,
    Delegate,
    Communicate,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExpectedMetrics {
    pub stop_reason: Option<ScenarioStopReason>,
    pub provider_calls: Option<usize>,
    pub provider_retries: Option<usize>,
    pub semantic_recoveries: Option<usize>,
    pub overflow_recoveries: Option<usize>,
    pub compactions: Option<usize>,
    pub tool_calls: Option<usize>,
    pub tool_errors: Option<usize>,
    pub mutation_attempts: Option<usize>,
    pub side_effects: Option<usize>,
    pub duplicate_side_effects: Option<usize>,
    pub validation_calls: Option<usize>,
    pub evidence_deltas: Option<usize>,
    pub state_deltas: Option<usize>,
    pub validation_deltas: Option<usize>,
    pub progress_events: Option<usize>,
    pub no_progress_cycles: Option<usize>,
    pub max_no_progress_streak: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ScenarioMetrics {
    pub provider_calls: usize,
    pub provider_retries: usize,
    pub semantic_recoveries: usize,
    pub overflow_recoveries: usize,
    pub compactions: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub mutation_attempts: usize,
    pub side_effects: usize,
    pub duplicate_side_effects: usize,
    pub validation_calls: usize,
    pub evidence_deltas: usize,
    pub state_deltas: usize,
    pub validation_deltas: usize,
    pub progress_events: usize,
    pub no_progress_cycles: usize,
    pub max_no_progress_streak: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayTraceEvent {
    pub run_id: String,
    pub sequence: usize,
    pub turn: usize,
    pub event_type: String,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<ScenarioStopReason>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioReport {
    pub name: String,
    pub stop_reason: ScenarioStopReason,
    pub metrics: ScenarioMetrics,
    pub trace: Vec<ReplayTraceEvent>,
}

impl ScenarioReport {
    pub fn expectation_violations(&self, expected: &ExpectedMetrics) -> Vec<String> {
        let mut violations = Vec::new();

        if let Some(value) = expected.stop_reason {
            if self.stop_reason != value {
                violations.push(format!(
                    "stop_reason was {:?}, expected {:?}",
                    self.stop_reason, value
                ));
            }
        }

        macro_rules! compare_metric {
            ($field:ident) => {
                if let Some(value) = expected.$field {
                    if self.metrics.$field != value {
                        violations.push(format!(
                            "{} was {}, expected {}",
                            stringify!($field),
                            self.metrics.$field,
                            value
                        ));
                    }
                }
            };
        }

        compare_metric!(provider_calls);
        compare_metric!(provider_retries);
        compare_metric!(semantic_recoveries);
        compare_metric!(overflow_recoveries);
        compare_metric!(compactions);
        compare_metric!(tool_calls);
        compare_metric!(tool_errors);
        compare_metric!(mutation_attempts);
        compare_metric!(side_effects);
        compare_metric!(duplicate_side_effects);
        compare_metric!(validation_calls);
        compare_metric!(evidence_deltas);
        compare_metric!(state_deltas);
        compare_metric!(validation_deltas);
        compare_metric!(progress_events);
        compare_metric!(no_progress_cycles);
        compare_metric!(max_no_progress_streak);

        violations
    }
}

#[derive(Debug, Default)]
struct VirtualWorld {
    workspace_revision: usize,
    evidence: BTreeSet<String>,
    validation_results: BTreeSet<String>,
    applied_side_effects: BTreeSet<String>,
    seen_call_ids: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct BatchDelta {
    evidence: usize,
    state: usize,
    validation: usize,
}

impl BatchDelta {
    fn made_progress(&self) -> bool {
        self.evidence > 0 || self.state > 0 || self.validation > 0
    }
}

pub fn run_scenario(scenario: &ScriptedScenario) -> Result<ScenarioReport, String> {
    if scenario.policy.max_no_progress_cycles == 0 {
        return Err("max_no_progress_cycles must be positive".to_string());
    }

    let mut metrics = ScenarioMetrics::default();
    let mut world = VirtualWorld::default();
    let mut trace = Vec::new();
    let mut stop_reason = None;
    let mut no_progress_streak = 0;

    for (index, step) in scenario.steps.iter().enumerate() {
        if stop_reason.is_some() {
            break;
        }

        if scenario
            .policy
            .max_provider_calls
            .is_some_and(|maximum| metrics.provider_calls >= maximum)
        {
            stop_reason = Some(ScenarioStopReason::BudgetExhausted);
            break;
        }

        let turn = index + 1;
        metrics.provider_calls += 1;
        push_trace(
            &mut trace,
            turn,
            "provider_call",
            serde_json::json!({"step_kind": step_kind(step)}),
            None,
        );

        match step {
            ScriptedModelStep::ToolBatch { calls } => {
                let delta = execute_tool_batch(calls, turn, &mut world, &mut metrics, &mut trace)?;
                if delta.made_progress() {
                    metrics.progress_events += 1;
                    no_progress_streak = 0;
                    push_trace(
                        &mut trace,
                        turn,
                        "progress",
                        serde_json::json!({
                            "evidence_delta": delta.evidence,
                            "state_delta": delta.state,
                            "validation_delta": delta.validation,
                            "workspace_revision": world.workspace_revision,
                        }),
                        None,
                    );
                } else {
                    metrics.no_progress_cycles += 1;
                    no_progress_streak += 1;
                    metrics.max_no_progress_streak =
                        metrics.max_no_progress_streak.max(no_progress_streak);
                    push_trace(
                        &mut trace,
                        turn,
                        "no_progress",
                        serde_json::json!({
                            "streak": no_progress_streak,
                            "threshold": scenario.policy.max_no_progress_cycles,
                        }),
                        None,
                    );
                    if no_progress_streak >= scenario.policy.max_no_progress_cycles {
                        stop_reason = Some(ScenarioStopReason::LoopGuardTriggered);
                    }
                }
            }
            ScriptedModelStep::Complete { text } => {
                push_trace(
                    &mut trace,
                    turn,
                    "assistant_complete",
                    serde_json::json!({"text": text}),
                    None,
                );
                stop_reason = Some(ScenarioStopReason::Completed);
            }
            ScriptedModelStep::TransientError { code } => {
                if metrics.provider_retries < scenario.policy.max_provider_retries {
                    metrics.provider_retries += 1;
                    push_trace(
                        &mut trace,
                        turn,
                        "provider_retry",
                        serde_json::json!({"code": code, "retry": metrics.provider_retries}),
                        None,
                    );
                } else {
                    stop_reason = Some(ScenarioStopReason::ProviderError);
                }
            }
            ScriptedModelStep::MalformedStream { detail } => {
                push_trace(
                    &mut trace,
                    turn,
                    "malformed_stream",
                    serde_json::json!({"detail": detail}),
                    None,
                );
                stop_reason = Some(ScenarioStopReason::ProviderError);
            }
            ScriptedModelStep::EmptyCompletion => {
                if metrics.semantic_recoveries < scenario.policy.max_empty_recoveries {
                    metrics.semantic_recoveries += 1;
                    push_trace(
                        &mut trace,
                        turn,
                        "semantic_recovery",
                        serde_json::json!({"recovery": metrics.semantic_recoveries}),
                        None,
                    );
                } else {
                    stop_reason = Some(ScenarioStopReason::ProviderError);
                }
            }
            ScriptedModelStep::ContextOverflow => {
                if metrics.overflow_recoveries < scenario.policy.max_overflow_recoveries {
                    metrics.overflow_recoveries += 1;
                    metrics.compactions += 1;
                    push_trace(
                        &mut trace,
                        turn,
                        "context_compacted",
                        serde_json::json!({"recovery": metrics.overflow_recoveries}),
                        None,
                    );
                } else {
                    stop_reason = Some(ScenarioStopReason::ProviderError);
                }
            }
        }
    }

    let stop_reason = stop_reason.ok_or_else(|| {
        format!(
            "scenario '{}' exhausted its script without a terminal decision",
            scenario.name
        )
    })?;
    let final_turn = metrics.provider_calls;
    push_trace(
        &mut trace,
        final_turn,
        "finished",
        serde_json::json!({"stop_reason": stop_reason}),
        Some(stop_reason),
    );

    Ok(ScenarioReport {
        name: scenario.name.clone(),
        stop_reason,
        metrics,
        trace,
    })
}

fn execute_tool_batch(
    calls: &[ScriptedToolCall],
    turn: usize,
    world: &mut VirtualWorld,
    metrics: &mut ScenarioMetrics,
    trace: &mut Vec<ReplayTraceEvent>,
) -> Result<BatchDelta, String> {
    let mut delta = BatchDelta::default();

    for call in calls {
        if !world.seen_call_ids.insert(call.id.clone()) {
            return Err(format!("duplicate scripted tool call id '{}'", call.id));
        }

        metrics.tool_calls += 1;
        if !call.effect.ok {
            metrics.tool_errors += 1;
        }
        push_trace(
            trace,
            turn,
            "tool_call_complete",
            serde_json::json!({
                "id": call.id,
                "name": call.tool,
                "arguments": call.arguments,
                "intent": call.intent,
                "effect_class": call.effect.class,
            }),
            None,
        );

        if !call.effect.ok {
            continue;
        }

        match call.effect.class {
            EffectClass::Observe | EffectClass::Delegate => {
                if let Some(evidence) = call.effect.evidence.as_ref() {
                    if world.evidence.insert(evidence.clone()) {
                        delta.evidence += 1;
                        metrics.evidence_deltas += 1;
                    }
                }
            }
            EffectClass::Mutate => {
                metrics.mutation_attempts += 1;
                if call.effect.changed {
                    if let Some(key) = call.effect.side_effect_key.as_ref() {
                        if !world.applied_side_effects.insert(key.clone()) {
                            metrics.duplicate_side_effects += 1;
                        }
                    }
                    metrics.side_effects += 1;
                    metrics.state_deltas += 1;
                    delta.state += 1;
                    world.workspace_revision += 1;
                }
            }
            EffectClass::Validate => {
                metrics.validation_calls += 1;
                let fingerprint = format!(
                    "{}:{}:{}",
                    world.workspace_revision,
                    call.intent,
                    call.effect.result.as_deref().unwrap_or("unknown")
                );
                if world.validation_results.insert(fingerprint) {
                    metrics.validation_deltas += 1;
                    delta.validation += 1;
                }
            }
            EffectClass::Communicate => {}
        }

        push_trace(
            trace,
            turn,
            "tool_result",
            serde_json::json!({
                "id": call.id,
                "ok": call.effect.ok,
                "changed": call.effect.changed,
                "resource": call.effect.resource,
                "evidence": call.effect.evidence,
            }),
            None,
        );
    }

    Ok(delta)
}

fn push_trace(
    trace: &mut Vec<ReplayTraceEvent>,
    turn: usize,
    event_type: &str,
    payload: Value,
    stop_reason: Option<ScenarioStopReason>,
) {
    trace.push(ReplayTraceEvent {
        run_id: "scripted-run".to_string(),
        sequence: trace.len() + 1,
        turn,
        event_type: event_type.to_string(),
        payload,
        stop_reason,
    });
}

fn step_kind(step: &ScriptedModelStep) -> &'static str {
    match step {
        ScriptedModelStep::ToolBatch { .. } => "tool_batch",
        ScriptedModelStep::Complete { .. } => "complete",
        ScriptedModelStep::TransientError { .. } => "transient_error",
        ScriptedModelStep::MalformedStream { .. } => "malformed_stream",
        ScriptedModelStep::EmptyCompletion => "empty_completion",
        ScriptedModelStep::ContextOverflow => "context_overflow",
    }
}

pub fn validate_replay_trace(report: &ScenarioReport) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    for (index, event) in report.trace.iter().enumerate() {
        if event.sequence != index + 1 {
            violations.push(format!(
                "trace sequence {} at index {} was not contiguous",
                event.sequence, index
            ));
        }
    }

    let terminal_events = report
        .trace
        .iter()
        .filter(|event| event.event_type == "finished")
        .collect::<Vec<_>>();
    if terminal_events.len() != 1 {
        violations.push(format!(
            "trace contained {} terminal events instead of one",
            terminal_events.len()
        ));
    }
    if report.trace.last().map(|event| event.event_type.as_str()) != Some("finished") {
        violations.push("finished was not the final trace event".to_string());
    }
    if terminal_events.first().and_then(|event| event.stop_reason) != Some(report.stop_reason) {
        violations.push("terminal trace reason did not match the report".to_string());
    }

    let provider_calls = report
        .trace
        .iter()
        .filter(|event| event.event_type == "provider_call")
        .count();
    if provider_calls != report.metrics.provider_calls {
        violations.push(format!(
            "trace provider calls {} did not match metrics {}",
            provider_calls, report.metrics.provider_calls
        ));
    }
    let tool_calls = report
        .trace
        .iter()
        .filter(|event| event.event_type == "tool_call_complete")
        .count();
    if tool_calls != report.metrics.tool_calls {
        violations.push(format!(
            "trace tool calls {} did not match metrics {}",
            tool_calls, report.metrics.tool_calls
        ));
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SurfaceParityFixtureFile {
    pub fixtures: Vec<SurfaceParityFixture>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SurfaceParityFixture {
    pub name: String,
    pub expected: ResolvedRunFingerprint,
    pub surfaces: Vec<SurfaceResolvedRun>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SurfaceResolvedRun {
    pub surface: String,
    pub resolved: ResolvedRunFingerprint,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ResolvedRunFingerprint {
    pub provider: String,
    pub model: String,
    pub transport: String,
    pub reasoning_effort: Option<String>,
    pub permission_mode: String,
    pub max_turns: Option<usize>,
    pub tools: Vec<String>,
}

impl ResolvedRunFingerprint {
    fn canonicalized(mut self) -> Self {
        self.tools.sort();
        self.tools.dedup();
        self
    }
}

pub fn surface_parity_violations(fixture: &SurfaceParityFixture) -> Vec<String> {
    let expected = fixture.expected.clone().canonicalized();
    fixture
        .surfaces
        .iter()
        .filter_map(|surface| {
            let actual = surface.resolved.clone().canonicalized();
            (actual != expected).then(|| {
                format!(
                    "{} resolved {:?}, expected {:?}",
                    surface.surface, actual, expected
                )
            })
        })
        .collect()
}
