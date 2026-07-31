//! Semantic progress accounting for the canonical parent-agent loop.
//!
//! Turn count is a resource budget, not a loop detector. This ledger tracks
//! whether tool work produces new evidence or successful state changes so
//! cosmetically different calls cannot keep an agent alive indefinitely.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ai::types::{AiToolCall, Content};
use crate::tools::registry::{
    agent_call_action, agent_call_requests_write, effective_tool_call, tool_policy_for_call,
    trusted_changed, ToolCategory,
};

use super::failure;
use super::hooks::shell_policy::{classify_bash_command, semantic_bash_signature};

/// Consecutive evidence-free passive turns allowed before stopping.
pub const NO_PROGRESS_TURN_THRESHOLD: usize = 3;
/// Inject a model-visible change-of-strategy instruction before the terminal
/// threshold, giving a capable model one clean opportunity to recover.
pub const NO_PROGRESS_REPLAN_THRESHOLD: usize = 2;
/// A single observation intent may reveal a few changing outcomes (for
/// example, a growing log), but raw stdout churn is not unbounded proof of
/// progress. This closes random/timestamp output loops without imposing a
/// global turn limit.
pub const MAX_OUTCOME_VARIANTS_PER_INTENT: usize = 6;
/// Likewise, one identical outcome may be discovered through a bounded number
/// of distinct semantic intents (for example, several equal files). Varying
/// arguments around unchanged output cannot buy unlimited progress.
pub const MAX_INTENTS_PER_IDENTICAL_OUTCOME: usize = 8;
/// Opaque shell effects do not provide a trustworthy state delta. Permit a
/// short sequence of distinct build steps, then require independent evidence
/// before more unverified effects can keep the run alive.
pub const MAX_CONSECUTIVE_UNVERIFIED_EFFECT_TURNS: usize = 6;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    Observe,
    Mutate,
    Validate,
    Delegate,
    Communicate,
    Control,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionFingerprint {
    pub class: ActionClass,
    /// Stable hash of normalized intent; raw commands and arguments are not
    /// emitted into runtime telemetry.
    pub intent: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressGuardAction {
    Warn,
    Replan,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressGuardTelemetry {
    pub guard: String,
    pub no_progress_turns: usize,
    pub threshold: usize,
    pub mutation_epoch: u64,
    pub action_classes: Vec<ActionClass>,
    pub evidence_signature: String,
    pub action: ProgressGuardAction,
    pub triggered: bool,
}

impl ProgressGuardTelemetry {
    pub fn diagnostic(&self) -> Option<String> {
        self.triggered.then(|| {
            format!(
                "Stopping no-progress loop: repeated work produced no verified state change or bounded new evidence for {} consecutive turns. Synthesize the evidence already gathered, change strategy, or ask the user for direction.",
                self.no_progress_turns
            )
        })
    }

    pub fn replan_instruction(&self) -> Option<&'static str> {
        matches!(self.action, ProgressGuardAction::Replan).then_some(
            "[PROGRESS GUARD]\nThe last actions repeated existing evidence or produced no state change. Do not repeat the same command with cosmetic variations. Reassess the current evidence, choose a materially different action, finish with the answer if the task is already complete, or ask for missing input.",
        )
    }
}

#[derive(Debug)]
pub struct LoopGuardOutcome {
    pub repeated_failure: Option<String>,
    pub repeated_validation: Option<String>,
    /// Same pure-exploration tool batch repeated without mutation.
    pub repeated_read_only: Option<String>,
    pub progress: Option<ProgressGuardTelemetry>,
}

#[derive(Debug, Default)]
pub struct LoopGuard {
    progress: ProgressLedger,
    failure_signatures: HashMap<String, usize>,
    validation_signatures: HashMap<String, usize>,
    read_only_signatures: HashMap<String, usize>,
}

impl LoopGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_for_steering(&mut self) {
        self.progress.reset_for_steering();
        self.failure_signatures.clear();
        self.validation_signatures.clear();
        self.read_only_signatures.clear();
    }

    pub fn evaluate(
        &mut self,
        tool_calls: &[AiToolCall],
        tool_results: &[Content],
    ) -> LoopGuardOutcome {
        LoopGuardOutcome {
            repeated_failure: failure::detect_repeated_failures(
                &mut self.failure_signatures,
                tool_calls,
                tool_results,
            ),
            repeated_validation: failure::detect_repeated_validation_sequence(
                &mut self.validation_signatures,
                tool_calls,
                tool_results,
            ),
            repeated_read_only: failure::detect_repeated_read_only_sequence(
                &mut self.read_only_signatures,
                tool_calls,
            ),
            progress: self.progress.record_turn(tool_calls, tool_results),
        }
    }
}

#[derive(Debug, Default)]
pub struct ProgressLedger {
    mutation_epoch: u64,
    seen_evidence: HashSet<String>,
    seen_changed_effects: HashSet<String>,
    seen_unverified_effects: HashSet<String>,
    outcome_variants: HashMap<String, HashSet<String>>,
    outcome_intents: HashMap<String, HashSet<String>>,
    consecutive_unverified_effect_turns: usize,
    no_progress_turns: usize,
}

impl ProgressLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_for_steering(&mut self) {
        self.mutation_epoch = self.mutation_epoch.saturating_add(1);
        self.seen_evidence.clear();
        self.seen_changed_effects.clear();
        self.seen_unverified_effects.clear();
        self.outcome_variants.clear();
        self.outcome_intents.clear();
        self.consecutive_unverified_effect_turns = 0;
        self.no_progress_turns = 0;
    }

    /// Record one completed tool turn and return structured telemetry when a
    /// passive turn repeated existing evidence. Successful mutations and
    /// control actions are the only events that reset the evidence window.
    pub fn record_turn(
        &mut self,
        tool_calls: &[AiToolCall],
        tool_results: &[Content],
    ) -> Option<ProgressGuardTelemetry> {
        if tool_calls.is_empty() {
            return None;
        }

        let fingerprints = tool_calls
            .iter()
            .map(action_fingerprint)
            .collect::<Vec<_>>();
        let results = results_by_call_id(tool_results);

        let mut evidence_candidates = Vec::new();
        let mut action_classes = Vec::new();
        let mut discovered_new_changed_effect = false;
        let mut discovered_new_unverified_effect = false;

        for (call, fingerprint) in tool_calls.iter().zip(&fingerprints) {
            let Some(result) = results.get(call.id.as_str()) else {
                continue;
            };
            // Failed mutations and validations have dedicated outcome-aware
            // guards. Do not let this ledger race or double-count them.
            if result.is_error {
                continue;
            }

            let producer_declares_effect = result.changed.is_some();
            let effect_class = matches!(
                fingerprint.class,
                ActionClass::Mutate | ActionClass::Communicate | ActionClass::Control
            );
            let evidence_class = matches!(
                fingerprint.class,
                ActionClass::Observe | ActionClass::Delegate | ActionClass::Validate
            );
            if !producer_declares_effect && !effect_class && !evidence_class {
                continue;
            }

            action_classes.push(if result.changed == Some(true) {
                ActionClass::Mutate
            } else {
                fingerprint.class
            });

            // A physical delta is semantic progress only once for the same
            // normalized intent in the current steering window. This contains
            // timestamp churn, random rewrites, and repeated background starts
            // without reintroducing an arbitrary global turn cap.
            let effect_identity = result
                .progress_change_key
                .as_ref()
                .unwrap_or(&fingerprint.intent);
            if result.changed == Some(true)
                && self.seen_changed_effects.insert(effect_identity.clone())
            {
                discovered_new_changed_effect = true;
            } else if result.changed.is_none()
                && effect_class
                && self
                    .seen_unverified_effects
                    .insert(fingerprint.intent.clone())
            {
                discovered_new_unverified_effect = true;
            }

            // Intent spelling never proves progress by itself. Successful
            // observations, delegated work, and validations need a materially
            // new outcome. Opaque effects do not get outcome credit: a command
            // can manufacture arbitrary random stdout without advancing the
            // task, so it must be followed by independently observed evidence.
            if evidence_class && result.changed != Some(true) {
                let (effective_name, _) = effective_tool_call(&call.name, &call.arguments);
                evidence_candidates.push((
                    fingerprint.intent.clone(),
                    outcome_evidence_key(call, result),
                    matches!(effective_name, "bash" | "shell" | "execute"),
                ));
            }
        }

        if action_classes.is_empty() {
            return None;
        }

        evidence_candidates.sort();
        evidence_candidates.dedup();
        if discovered_new_changed_effect {
            self.mutation_epoch = self.mutation_epoch.saturating_add(1);
            self.seen_evidence.clear();
            self.outcome_variants.clear();
            self.outcome_intents.clear();
            self.seen_unverified_effects.clear();
            self.consecutive_unverified_effect_turns = 0;
        }

        let mut accepted_evidence = Vec::new();
        for (intent, evidence, bound_cross_intent_churn) in evidence_candidates {
            let evidence_identity = format!("{intent}:{evidence}");
            if self.seen_evidence.contains(&evidence_identity) {
                continue;
            }
            let variants = self.outcome_variants.entry(intent).or_default();
            if variants.len() >= MAX_OUTCOME_VARIANTS_PER_INTENT {
                continue;
            }
            if bound_cross_intent_churn {
                let intents = self.outcome_intents.entry(evidence.clone()).or_default();
                if intents.len() >= MAX_INTENTS_PER_IDENTICAL_OUTCOME {
                    continue;
                }
                intents.insert(evidence_identity.clone());
            }
            variants.insert(evidence.clone());
            self.seen_evidence.insert(evidence_identity.clone());
            accepted_evidence.push(evidence_identity);
        }

        if discovered_new_changed_effect {
            self.no_progress_turns = 0;
            return None;
        }

        if !accepted_evidence.is_empty() {
            self.seen_unverified_effects.clear();
            self.consecutive_unverified_effect_turns = 0;
            self.no_progress_turns = 0;
            return None;
        }

        if discovered_new_unverified_effect {
            self.consecutive_unverified_effect_turns =
                self.consecutive_unverified_effect_turns.saturating_add(1);
            if self.consecutive_unverified_effect_turns <= MAX_CONSECUTIVE_UNVERIFIED_EFFECT_TURNS {
                self.no_progress_turns = 0;
                return None;
            }
        }

        self.no_progress_turns = self.no_progress_turns.saturating_add(1);
        action_classes.sort_by_key(|class| action_class_order(*class));
        action_classes.dedup();

        Some(self.telemetry(action_classes, &accepted_evidence.join("|")))
    }

    fn telemetry(
        &self,
        action_classes: Vec<ActionClass>,
        evidence: &str,
    ) -> ProgressGuardTelemetry {
        ProgressGuardTelemetry {
            guard: "semantic_no_progress".to_string(),
            no_progress_turns: self.no_progress_turns,
            threshold: NO_PROGRESS_TURN_THRESHOLD,
            mutation_epoch: self.mutation_epoch,
            action_classes,
            evidence_signature: hash_text(evidence),
            action: if self.no_progress_turns >= NO_PROGRESS_TURN_THRESHOLD {
                ProgressGuardAction::Stop
            } else if self.no_progress_turns >= NO_PROGRESS_REPLAN_THRESHOLD {
                ProgressGuardAction::Replan
            } else {
                ProgressGuardAction::Warn
            },
            triggered: self.no_progress_turns >= NO_PROGRESS_TURN_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone)]
struct ResultEvidence {
    is_error: bool,
    output: Value,
    signature: String,
    status_signature: String,
    changed: Option<bool>,
    progress_change_key: Option<String>,
}

fn results_by_call_id(tool_results: &[Content]) -> HashMap<&str, ResultEvidence> {
    tool_results
        .iter()
        .filter_map(|result| match result {
            Content::ToolResult {
                tool_use_id,
                output,
                is_error,
            } => Some((
                tool_use_id.as_str(),
                ResultEvidence {
                    is_error: is_error.unwrap_or(false),
                    output: output.clone(),
                    signature: hash_value(output),
                    status_signature: stable_status_signature(output, is_error.unwrap_or(false)),
                    changed: changed_value(output),
                    progress_change_key: progress_change_key(output),
                },
            )),
            _ => None,
        })
        .collect()
}

fn outcome_evidence_key(call: &AiToolCall, result: &ResultEvidence) -> String {
    let (name, arguments) = effective_tool_call(&call.name, &call.arguments);
    let signature = if matches!(name, "bash" | "shell" | "execute") {
        let background = arguments
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if background {
            result.status_signature.clone()
        } else {
            bash_output_signature(result)
        }
    } else {
        result.signature.clone()
    };
    format!("{name}:{signature}")
}

fn bash_output_signature(result: &ResultEvidence) -> String {
    let output = extract_output_text(&result.output).unwrap_or_default();
    hash_text(&format!("{}:{output}", result.status_signature))
}

fn extract_output_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|decoded| extract_output_text(&decoded))
            .or_else(|| Some(text.clone())),
        Value::Object(object) => object
            .get("output")
            .and_then(extract_output_text)
            .or_else(|| {
                ["data", "result", "summary"]
                    .into_iter()
                    .filter_map(|key| object.get(key))
                    .find_map(extract_output_text)
            }),
        Value::Array(values) => {
            let parts = values
                .iter()
                .filter_map(extract_output_text)
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Value::Null => None,
        scalar => Some(scalar.to_string()),
    }
}

fn stable_status_signature(output: &Value, is_error: bool) -> String {
    fn find<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
        let object = value.as_object()?;
        object.get(name).or_else(|| {
            ["metadata", "result", "data", "summary"]
                .into_iter()
                .filter_map(|key| object.get(key))
                .find_map(|nested| find(nested, name))
        })
    }

    let stable = serde_json::json!({
        "is_error": is_error,
        "error_code": find(output, "error_code"),
        "exit_code": find(output, "exit_code"),
        "killed": find(output, "killed"),
        "status": find(output, "status"),
        "process_error": find(output, "process_error"),
        "reused_existing": find(output, "reused_existing"),
    });
    hash_value(&stable)
}

fn changed_value(output: &Value) -> Option<bool> {
    trusted_changed(output)
}

fn progress_change_key(output: &Value) -> Option<String> {
    match output {
        Value::Object(object) => object
            .get("progress_change_key")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        Value::String(serialized) => serde_json::from_str::<Value>(serialized)
            .ok()
            .as_ref()
            .and_then(progress_change_key),
        _ => None,
    }
}

pub fn action_fingerprint(call: &AiToolCall) -> ActionFingerprint {
    let (name, arguments) = effective_tool_call(&call.name, &call.arguments);
    let class = action_class(name, arguments, call);
    let normalized = if matches!(name, "bash" | "shell" | "execute") {
        let command = arguments
            .get("command")
            .and_then(Value::as_str)
            .map(semantic_bash_signature)
            .unwrap_or_else(|| "missing-command".to_string());
        let mode = if arguments
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "background"
        } else {
            "foreground"
        };
        format!("{mode}:{command}")
    } else {
        normalized_arguments(name, arguments)
    };

    ActionFingerprint {
        class,
        intent: hash_text(&format!("{name}:{normalized}")),
    }
}

fn action_class(name: &str, arguments: &Value, call: &AiToolCall) -> ActionClass {
    if failure::is_validation_call(call) {
        return ActionClass::Validate;
    }

    match name {
        "bash" | "shell" | "execute" => {
            let background = arguments
                .get("run_in_background")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mutates = background
                || arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| {
                        classify_bash_command(command).modifies_filesystem_or_process
                    });
            if mutates {
                ActionClass::Mutate
            } else {
                ActionClass::Observe
            }
        }
        "agent" => match agent_call_action(arguments) {
            "list" | "status" | "wait" => ActionClass::Observe,
            "message" | "followup" => ActionClass::Communicate,
            "interrupt" => ActionClass::Control,
            _ if agent_call_requests_write(arguments) => ActionClass::Mutate,
            _ => ActionClass::Delegate,
        },
        "send_user_message" | "report" => ActionClass::Communicate,
        "set_work_mode" | "enter_plan_mode" | "workflow_propose" | "workflow_update"
        | "task_start" | "task_complete" | "add_subtask" | "set_dependency" | "sleep" => {
            ActionClass::Control
        }
        _ => match tool_policy_for_call(name, arguments).category {
            ToolCategory::ReadOnly => ActionClass::Observe,
            ToolCategory::Write => ActionClass::Mutate,
            ToolCategory::Interactive => ActionClass::Control,
        },
    }
}

fn normalized_arguments(name: &str, arguments: &Value) -> String {
    let mut normalized = arguments.clone();
    if let Some(object) = normalized.as_object_mut() {
        // Output size/presentation changes can reveal more evidence, which is
        // captured by the result signature. They are not a new action intent.
        match name {
            "list" => {
                object.remove("limit");
            }
            "glob" => {
                object.remove("max_results");
            }
            _ => {}
        }
    }
    normalized.to_string()
}

fn hash_value(value: &Value) -> String {
    hash_text(&value.to_string())
}

fn hash_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

const fn action_class_order(class: ActionClass) -> u8 {
    match class {
        ActionClass::Observe => 0,
        ActionClass::Mutate => 1,
        ActionClass::Validate => 2,
        ActionClass::Delegate => 3,
        ActionClass::Communicate => 4,
        ActionClass::Control => 5,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn call(id: &str, name: &str, arguments: Value) -> AiToolCall {
        AiToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        }
    }

    fn result(id: &str, output: &str, is_error: bool) -> Content {
        Content::ToolResult {
            tool_use_id: id.to_string(),
            output: Value::String(output.to_string()),
            is_error: is_error.then_some(true),
        }
    }

    fn result_with_changed(id: &str, changed: bool) -> Content {
        Content::ToolResult {
            tool_use_id: id.to_string(),
            output: json!({ "ok": true, "changed": changed }),
            is_error: None,
        }
    }

    fn result_with_changed_key(id: &str, key: &str) -> Content {
        Content::ToolResult {
            tool_use_id: id.to_string(),
            output: json!({
                "ok": true,
                "changed": true,
                "progress_change_key": key
            }),
            is_error: None,
        }
    }

    #[test]
    fn nested_payload_cannot_spoof_producer_change_metadata() {
        assert_eq!(changed_value(&json!({"data": {"changed": true}})), None);
        assert_eq!(
            progress_change_key(&json!({
                "data": {"progress_change_key": "untrusted-nested-key"}
            })),
            None
        );
    }

    #[test]
    fn read_only_and_noop_bash_are_observations_not_mutations() {
        for command in ["pwd", "git status --short", "rg needle src", "true"] {
            let fingerprint =
                action_fingerprint(&call("bash-1", "bash", json!({ "command": command })));
            assert_eq!(fingerprint.class, ActionClass::Observe, "{command}");
        }
    }

    #[test]
    fn cosmetic_bash_variants_have_the_same_intent() {
        let first = action_fingerprint(&call(
            "bash-1",
            "bash",
            json!({
                "command": "pwd && rg -n 'needle' src | head -n 20",
                "timeout": 30_000,
                "description": "first look"
            }),
        ));
        let second = action_fingerprint(&call(
            "bash-2",
            "bash",
            json!({
                "command": "rg --line-number 'needle' src | head -50",
                "timeout": 60_000,
                "description": "try again"
            }),
        ));

        assert_eq!(first, second);
    }

    #[test]
    fn path_and_invocation_cosmetics_do_not_evade_bash_intent() {
        let calls = [
            "rg -n TODO src",
            "rg --line-number 'TODO' ./src/",
            "cd . && command rg -n TODO src/",
        ];
        let fingerprints = calls
            .into_iter()
            .map(|command| action_fingerprint(&call("bash", "bash", json!({ "command": command }))))
            .collect::<Vec<_>>();

        assert!(fingerprints.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn effectful_shell_cosmetics_do_not_evade_bash_intent() {
        let calls = [
            "mkdir -p out",
            "mkdir  -p out",
            "mkdir -p ./out",
            "command mkdir -p out",
            "true && mkdir -p out",
        ];
        let fingerprints = calls
            .into_iter()
            .map(|command| action_fingerprint(&call("bash", "bash", json!({ "command": command }))))
            .collect::<Vec<_>>();

        assert!(fingerprints.windows(2).all(|pair| pair[0] == pair[1]));
        assert!(fingerprints
            .iter()
            .all(|fingerprint| fingerprint.class == ActionClass::Mutate));
    }

    #[test]
    fn interpreter_and_background_bash_calls_are_effectful() {
        for command in [
            "python3 -c 'open(\"out\", \"w\").write(\"x\")'",
            "node -e 'require(\"fs\").writeFileSync(\"out\", \"x\")'",
            "sh -c 'touch out'",
            "echo ok\ntouch out",
            "printf '%s' ok\nrm -f victim",
        ] {
            assert_eq!(
                action_fingerprint(&call("bash", "bash", json!({"command": command}))).class,
                ActionClass::Mutate,
                "{command}"
            );
        }

        let foreground = action_fingerprint(&call(
            "foreground",
            "bash",
            json!({"command": "ps -axo pid,command"}),
        ));
        let background = action_fingerprint(&call(
            "background",
            "bash",
            json!({"command": "ps -axo pid,command", "run_in_background": true}),
        ));
        assert_eq!(foreground.class, ActionClass::Observe);
        assert_eq!(background.class, ActionClass::Mutate);
        assert_ne!(foreground.intent, background.intent);
    }

    #[test]
    fn repeated_semantic_observations_converge_after_three_no_progress_turns() {
        let mut ledger = ProgressLedger::new();
        let commands = [
            "rg -n needle src | head -20",
            "pwd && rg --line-number needle src | head -50",
            "rg -n needle src | head -100",
            "rg --line-number needle src",
        ];

        for (index, command) in commands.iter().enumerate() {
            let id = format!("bash-{index}");
            let calls = vec![call(&id, "bash", json!({ "command": command }))];
            let results = vec![result(&id, "same evidence", false)];
            let telemetry = ledger.record_turn(&calls, &results);
            if index == 0 {
                assert!(telemetry.is_none());
            } else if index < 3 {
                let telemetry = telemetry.expect("repeat telemetry");
                assert!(!telemetry.triggered);
                assert_eq!(
                    telemetry.action,
                    if index == 2 {
                        ProgressGuardAction::Replan
                    } else {
                        ProgressGuardAction::Warn
                    }
                );
            } else {
                let telemetry = telemetry.expect("guard telemetry");
                assert!(telemetry.triggered);
                assert_eq!(telemetry.action, ProgressGuardAction::Stop);
                assert_eq!(telemetry.no_progress_turns, 3);
                assert!(telemetry.diagnostic().is_some());
            }
        }
    }

    #[test]
    fn changed_evidence_and_positive_mutation_reset_no_progress() {
        let mut ledger = ProgressLedger::new();
        let read = vec![call("read-1", "read", json!({ "file_path": "src/lib.rs" }))];
        assert!(ledger
            .record_turn(&read, &[result("read-1", "before", false)])
            .is_none());
        assert!(ledger
            .record_turn(&read, &[result("read-1", "before", false)])
            .is_some());
        assert!(ledger
            .record_turn(&read, &[result("read-1", "after", false)])
            .is_none());

        let write = vec![call(
            "write-1",
            "write",
            json!({ "file_path": "src/lib.rs", "content": "after" }),
        )];
        assert!(ledger
            .record_turn(&write, &[result_with_changed("write-1", true)])
            .is_none());
        assert!(ledger
            .record_turn(&read, &[result("read-1", "after", false)])
            .is_none());
    }

    #[test]
    fn explicit_noop_mutations_trigger_after_three_attempts() {
        let mut ledger = ProgressLedger::new();

        for attempt in 1..=3 {
            let id = format!("bash-{attempt}");
            let calls = vec![call(
                &id,
                "bash",
                json!({ "command": "mkdir -p already-present" }),
            )];
            let telemetry = ledger
                .record_turn(&calls, &[result_with_changed(&id, false)])
                .expect("explicit no-op must produce telemetry");
            assert_eq!(telemetry.no_progress_turns, attempt);
            assert_eq!(telemetry.triggered, attempt == 3);
            assert_eq!(
                telemetry.action,
                match attempt {
                    1 => ProgressGuardAction::Warn,
                    2 => ProgressGuardAction::Replan,
                    _ => ProgressGuardAction::Stop,
                }
            );
        }
    }

    #[test]
    fn distinct_producer_change_keys_progress_and_exact_repeats_converge() {
        let mut ledger = ProgressLedger::new();
        for (index, (command, change_key)) in [
            ("echo a > a.txt", "target-a"),
            ("echo b > b.txt", "target-b"),
            ("echo c > c.txt", "target-c"),
            ("echo d > d.txt", "target-d"),
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("write-{index}");
            assert!(
                ledger
                    .record_turn(
                        &[call(&id, "bash", json!({"command": command}))],
                        &[result_with_changed_key(&id, change_key)],
                    )
                    .is_none(),
                "distinct mutation must be progress: {command}"
            );
        }

        for (attempt, expected) in [
            ProgressGuardAction::Warn,
            ProgressGuardAction::Replan,
            ProgressGuardAction::Stop,
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("repeat-{attempt}");
            let telemetry = ledger
                .record_turn(
                    &[call(
                        &id,
                        "bash",
                        json!({"command": format!("echo repeat-{attempt} > d.txt")}),
                    )],
                    &[result_with_changed_key(&id, "target-d")],
                )
                .expect("a repeated producer change target must converge");
            assert_eq!(telemetry.action, expected);
            assert_eq!(telemetry.triggered, expected == ProgressGuardAction::Stop);
        }
    }

    #[test]
    fn opaque_mutation_batches_do_not_gain_novelty_from_order_or_multiplicity() {
        let mut ledger = ProgressLedger::new();
        let batch = |prefix: &str, commands: &[&str]| {
            let calls = commands
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    call(
                        &format!("{prefix}-{index}"),
                        "bash",
                        json!({"command": command}),
                    )
                })
                .collect::<Vec<_>>();
            let results = calls
                .iter()
                .map(|call| result(&call.id, "ok", false))
                .collect::<Vec<_>>();
            (calls, results)
        };

        let (first_calls, first_results) = batch("first", &["touch a", "touch b"]);
        assert!(ledger.record_turn(&first_calls, &first_results).is_none());

        let (reordered_calls, reordered_results) = batch("reordered", &["touch b", "touch a"]);
        let telemetry = ledger
            .record_turn(&reordered_calls, &reordered_results)
            .expect("reordering known opaque effects is not progress");
        assert_eq!(telemetry.action, ProgressGuardAction::Warn);

        let (repeated_calls, repeated_results) = batch("repeat", &["touch b", "touch a"]);
        let telemetry = ledger
            .record_turn(&repeated_calls, &repeated_results)
            .expect("repeating an ordered opaque effect sequence is not progress");
        assert_eq!(telemetry.action, ProgressGuardAction::Replan);

        let (duplicate_calls, duplicate_results) = batch("duplicate", &["touch a", "touch a"]);
        let telemetry = ledger
            .record_turn(&duplicate_calls, &duplicate_results)
            .expect("multiplicity alone must not create a new effect sequence");
        assert_eq!(telemetry.action, ProgressGuardAction::Stop);
        assert!(telemetry.triggered);
    }

    #[test]
    fn growing_single_effect_batches_cannot_evade_guard() {
        let mut ledger = ProgressLedger::new();

        for (turn, multiplicity) in (1..=4).enumerate() {
            let calls = (0..multiplicity)
                .map(|index| {
                    call(
                        &format!("turn-{turn}-{index}"),
                        "bash",
                        json!({"command": "touch same"}),
                    )
                })
                .collect::<Vec<_>>();
            let results = calls
                .iter()
                .map(|call| result(&call.id, "ok", false))
                .collect::<Vec<_>>();
            let telemetry = ledger.record_turn(&calls, &results);

            if turn == 0 {
                assert!(telemetry.is_none(), "the first opaque effect is novel");
            } else {
                let telemetry = telemetry.expect("multiplicity-only turn must be no-progress");
                assert_eq!(telemetry.no_progress_turns, turn);
                assert_eq!(telemetry.triggered, turn == 3);
            }
        }
    }

    #[test]
    fn explicit_no_change_overrides_novel_mutation_intent() {
        let mut ledger = ProgressLedger::new();
        for (index, command) in ["mkdir one", "mkdir two", "mkdir three"]
            .into_iter()
            .enumerate()
        {
            let id = format!("noop-{index}");
            let telemetry = ledger
                .record_turn(
                    &[call(&id, "bash", json!({"command": command}))],
                    &[result_with_changed(&id, false)],
                )
                .expect("producer-declared no-change must not count as progress");
            assert_eq!(telemetry.no_progress_turns, index + 1);
            assert_eq!(telemetry.triggered, index == 2);
        }
    }

    #[test]
    fn repeated_opaque_mutation_cannot_evade_guard_with_volatile_output() {
        let mut ledger = ProgressLedger::new();

        for attempt in 0..=3 {
            let id = format!("bash-{attempt}");
            let calls = vec![call(
                &id,
                "bash",
                json!({ "command": "mkdir -p already-present" }),
            )];
            let results = vec![result(
                &id,
                &format!("ok pid={} timestamp={attempt}", 10_000 + attempt),
                false,
            )];
            let telemetry = ledger.record_turn(&calls, &results);
            if attempt == 0 {
                assert!(telemetry.is_none());
            } else if attempt < 3 {
                assert!(!telemetry.expect("warning").triggered);
            } else {
                assert!(telemetry.expect("terminal guard").triggered);
            }
        }
    }

    #[test]
    fn shell_parameter_churn_is_unverified_and_converges() {
        let mut ledger = ProgressLedger::new();
        for attempt in 0..=NO_PROGRESS_TURN_THRESHOLD {
            let id = format!("random-{attempt}");
            let calls = vec![call(&id, "bash", json!({"command": "echo \"$RANDOM\""}))];
            assert_eq!(action_fingerprint(&calls[0]).class, ActionClass::Mutate);
            let telemetry =
                ledger.record_turn(&calls, &[result(&id, &format!("random-{attempt}"), false)]);
            if attempt == 0 {
                assert!(telemetry.is_none());
            } else {
                let telemetry = telemetry.expect("random stdout must not prove progress");
                assert_eq!(telemetry.no_progress_turns, attempt);
                assert_eq!(telemetry.triggered, attempt == NO_PROGRESS_TURN_THRESHOLD);
            }
        }
    }

    #[test]
    fn distinct_opaque_effects_require_evidence_after_a_bounded_runway() {
        let mut ledger = ProgressLedger::new();
        let total = MAX_CONSECUTIVE_UNVERIFIED_EFFECT_TURNS + NO_PROGRESS_TURN_THRESHOLD;

        for attempt in 0..total {
            let id = format!("opaque-{attempt}");
            let telemetry = ledger.record_turn(
                &[call(
                    &id,
                    "bash",
                    json!({"command": format!("sh -c 'printf step-{attempt}'")}),
                )],
                &[result(&id, "ok", false)],
            );
            if attempt < MAX_CONSECUTIVE_UNVERIFIED_EFFECT_TURNS {
                assert!(telemetry.is_none(), "bounded build runway: {attempt}");
            } else {
                let telemetry = telemetry.expect("unverified effect streak must converge");
                let no_progress = attempt - MAX_CONSECUTIVE_UNVERIFIED_EFFECT_TURNS + 1;
                assert_eq!(telemetry.no_progress_turns, no_progress);
                assert_eq!(
                    telemetry.triggered,
                    no_progress == NO_PROGRESS_TURN_THRESHOLD
                );
            }
        }
    }

    #[test]
    fn cosmetic_observation_intents_cannot_recredit_identical_output() {
        let mut ledger = ProgressLedger::new();
        let total = MAX_INTENTS_PER_IDENTICAL_OUTCOME + NO_PROGRESS_TURN_THRESHOLD;
        for attempt in 0..total {
            let id = format!("rg-{attempt}");
            let telemetry = ledger.record_turn(
                &[call(
                    &id,
                    "bash",
                    json!({
                        "command": format!("rg needle . --glob '!__noop_{attempt}__'")
                    }),
                )],
                &[result(&id, "same match", false)],
            );
            if attempt < MAX_INTENTS_PER_IDENTICAL_OUTCOME {
                assert!(telemetry.is_none());
            } else {
                let telemetry = telemetry.expect("identical evidence must converge");
                let no_progress = attempt - MAX_INTENTS_PER_IDENTICAL_OUTCOME + 1;
                assert_eq!(telemetry.no_progress_turns, no_progress);
                assert_eq!(
                    telemetry.triggered,
                    no_progress == NO_PROGRESS_TURN_THRESHOLD
                );
            }
        }
    }

    #[test]
    fn distinct_resources_with_equal_output_are_distinct_evidence() {
        let mut ledger = ProgressLedger::new();
        for index in 0..(MAX_INTENTS_PER_IDENTICAL_OUTCOME + 5) {
            let id = format!("read-equal-{index}");
            assert!(ledger
                .record_turn(
                    &[call(
                        &id,
                        "read",
                        json!({"file_path": format!("file-{index}.txt")}),
                    )],
                    &[result(&id, "same file contents", false)],
                )
                .is_none());
        }
    }

    #[test]
    fn materially_changing_log_and_listing_outcomes_count_within_the_bound() {
        let mut ledger = ProgressLedger::new();
        for (index, output) in ["line one", "line one\nline two", "line two\nline three"]
            .into_iter()
            .enumerate()
        {
            let id = format!("tail-{index}");
            assert!(ledger
                .record_turn(
                    &[call(
                        &id,
                        "bash",
                        json!({"command": "tail -n 20 server.log"}),
                    )],
                    &[result(&id, output, false)],
                )
                .is_none());
        }

        for (index, (command, output)) in [
            ("ls src", "a"),
            ("ls -d src", "src"),
            ("ls -R src", "src:\\na"),
            ("ls -A src", ".hidden\\na"),
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("ls-{index}");
            assert!(ledger
                .record_turn(
                    &[call(&id, "bash", json!({"command": command}))],
                    &[result(&id, output, false)],
                )
                .is_none());
        }
    }

    #[test]
    fn repeated_effect_plus_new_observation_evidence_is_progress() {
        let mut ledger = ProgressLedger::new();
        let effect = call("effect-1", "bash", json!({"command": "mkdir -p out"}));
        assert!(ledger
            .record_turn(
                std::slice::from_ref(&effect),
                &[result("effect-1", "ok", false)],
            )
            .is_none());

        let repeated_effect = call("effect-2", "bash", json!({"command": "mkdir -p out"}));
        let observation = call("read-1", "read", json!({"file_path": "out/state"}));
        assert!(ledger
            .record_turn(
                &[repeated_effect, observation],
                &[
                    result("effect-2", "ok", false),
                    result("read-1", "new state", false),
                ],
            )
            .is_none());
    }

    #[test]
    fn repeated_positive_delta_for_same_intent_converges_but_distinct_intents_progress() {
        let mut ledger = ProgressLedger::new();
        for (index, path) in ["a", "b", "c"].into_iter().enumerate() {
            let id = format!("distinct-{index}");
            assert!(ledger
                .record_turn(
                    &[call(
                        &id,
                        "bash",
                        json!({"command": format!("date +%s%N > {path}")}),
                    )],
                    &[result_with_changed_key(&id, &format!("target-{path}"))],
                )
                .is_none());
        }

        for (attempt, command) in [
            "printf '%s' \"$RANDOM\" > c",
            "python3 -c 'open(\"c\", \"w\").write(\"new\")'",
            "date +%s%N > c",
        ]
        .into_iter()
        .enumerate()
        {
            let attempt = attempt + 1;
            let id = format!("repeat-{attempt}");
            let telemetry = ledger
                .record_turn(
                    &[call(&id, "bash", json!({"command": command}))],
                    &[result_with_changed_key(&id, "target-c")],
                )
                .expect("repeated physical churn is not fresh semantic progress");
            assert_eq!(telemetry.no_progress_turns, attempt);
            assert_eq!(telemetry.triggered, attempt == NO_PROGRESS_TURN_THRESHOLD);
        }
    }

    #[test]
    fn changing_observation_output_has_a_bounded_novelty_allowance() {
        let mut ledger = ProgressLedger::new();

        for attempt in 0..(MAX_OUTCOME_VARIANTS_PER_INTENT + NO_PROGRESS_TURN_THRESHOLD) {
            let id = format!("bash-{attempt}");
            let calls = vec![call(
                &id,
                "bash",
                json!({ "command": "tail -n 20 changing.log" }),
            )];
            let results = vec![result(&id, &format!("random-outcome-{attempt}"), false)];
            let telemetry = ledger.record_turn(&calls, &results);
            if attempt < MAX_OUTCOME_VARIANTS_PER_INTENT {
                assert!(telemetry.is_none());
            } else {
                let telemetry = telemetry.expect("outcome churn must converge");
                let no_progress = attempt - MAX_OUTCOME_VARIANTS_PER_INTENT + 1;
                assert_eq!(telemetry.no_progress_turns, no_progress);
                assert_eq!(
                    telemetry.triggered,
                    no_progress == NO_PROGRESS_TURN_THRESHOLD
                );
            }
        }
    }

    #[test]
    fn explicit_process_status_change_is_new_evidence() {
        let mut ledger = ProgressLedger::new();
        let calls = vec![call("bash", "bash", json!({ "command": "ps -p 123" }))];
        let running = Content::ToolResult {
            tool_use_id: "bash".to_string(),
            output: json!({ "result": { "status": "running", "process_id": "123" } }),
            is_error: None,
        };
        let finished = Content::ToolResult {
            tool_use_id: "bash".to_string(),
            output: json!({ "result": { "status": "finished", "process_id": "456" } }),
            is_error: None,
        };

        assert!(ledger
            .record_turn(&calls, std::slice::from_ref(&running))
            .is_none());
        assert!(ledger.record_turn(&calls, &[running]).is_some());
        assert!(ledger.record_turn(&calls, &[finished]).is_none());
    }
}
