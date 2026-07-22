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
use crate::tools::registry::{effective_tool_call, tool_policy_for_call, ToolCategory};

use super::failure;
use super::hooks::shell_policy::{classify_bash_command, semantic_bash_signature};

/// Consecutive evidence-free passive turns allowed before stopping.
pub const NO_PROGRESS_TURN_THRESHOLD: usize = 3;
/// Inject a model-visible change-of-strategy instruction before the terminal
/// threshold, giving a capable model one clean opportunity to recover.
pub const NO_PROGRESS_REPLAN_THRESHOLD: usize = 2;

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
                "Stopping no-progress loop: semantically repeated observation work produced no new evidence for {} consecutive turns. Synthesize the evidence already gathered, change strategy, or ask the user for direction.",
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

#[derive(Debug, Default)]
pub struct ProgressLedger {
    mutation_epoch: u64,
    seen_evidence: HashSet<String>,
    seen_effects: HashSet<String>,
    no_progress_turns: usize,
}

impl ProgressLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_for_steering(&mut self) {
        self.mutation_epoch = self.mutation_epoch.saturating_add(1);
        self.seen_evidence.clear();
        self.seen_effects.clear();
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

        let mut successful_effects = tool_calls
            .iter()
            .zip(&fingerprints)
            .filter_map(|(call, fingerprint)| {
                if !matches!(
                    fingerprint.class,
                    ActionClass::Mutate | ActionClass::Communicate | ActionClass::Control
                ) {
                    return None;
                }
                let result = results.get(call.id.as_str())?;
                (!result.is_error).then(|| {
                    (
                        fingerprint.class,
                        // A repeated successful mutation intent is not fresh
                        // progress merely because its output includes a new
                        // timestamp, PID, or presentation detail. A different
                        // real mutation advances the epoch and permits later
                        // validation/observation to run again.
                        fingerprint.intent.clone(),
                        result.changed,
                    )
                })
            })
            .collect::<Vec<_>>();
        if !successful_effects.is_empty() {
            successful_effects.sort_by(|left, right| left.1.cmp(&right.1));
            successful_effects.dedup_by(|left, right| left.1 == right.1);
            let explicit_state_delta = successful_effects
                .iter()
                .any(|(_, _, changed)| *changed == Some(true));
            let discovered_new_effect = explicit_state_delta
                || successful_effects.iter().any(|(_, effect, changed)| {
                    changed.is_none() && !self.seen_effects.contains(effect)
                });
            self.seen_effects.extend(
                successful_effects
                    .iter()
                    .map(|(_, effect, _)| effect.clone()),
            );

            if discovered_new_effect {
                self.mutation_epoch = self.mutation_epoch.saturating_add(1);
                self.seen_evidence.clear();
                self.no_progress_turns = 0;
                return None;
            }

            self.no_progress_turns = self.no_progress_turns.saturating_add(1);
            let mut action_classes = successful_effects
                .iter()
                .map(|(class, _, _)| *class)
                .collect::<Vec<_>>();
            action_classes.sort_by_key(|class| action_class_order(*class));
            action_classes.dedup();
            return Some(
                self.telemetry(
                    action_classes,
                    &successful_effects
                        .iter()
                        .map(|(_, effect, _)| effect.as_str())
                        .collect::<Vec<_>>()
                        .join("|"),
                ),
            );
        }

        // Validation has its own outcome-aware semantic guard, and repeated
        // mutation failures have the failure-signature guard. This ledger owns
        // observation/delegated evidence only.
        if !fingerprints.iter().all(|fingerprint| {
            matches!(
                fingerprint.class,
                ActionClass::Observe | ActionClass::Delegate
            )
        }) {
            return None;
        }

        let mut turn_evidence = tool_calls
            .iter()
            .zip(&fingerprints)
            .map(|(call, fingerprint)| {
                let outcome = results
                    .get(call.id.as_str())
                    .map(|result| observation_signature(call, fingerprint.class, result))
                    .unwrap_or_default();
                format!("{}:{}", fingerprint.intent, outcome)
            })
            .collect::<Vec<_>>();
        turn_evidence.sort();
        turn_evidence.dedup();

        let discovered_new_evidence = turn_evidence
            .iter()
            .any(|evidence| !self.seen_evidence.contains(evidence));
        self.seen_evidence.extend(turn_evidence.iter().cloned());
        if discovered_new_evidence {
            self.no_progress_turns = 0;
            return None;
        }

        self.no_progress_turns = self.no_progress_turns.saturating_add(1);
        let mut action_classes = fingerprints
            .iter()
            .map(|fingerprint| fingerprint.class)
            .collect::<Vec<_>>();
        action_classes.sort_by_key(|class| action_class_order(*class));
        action_classes.dedup();

        Some(self.telemetry(action_classes, &turn_evidence.join("|")))
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
    signature: String,
    status_signature: String,
    changed: Option<bool>,
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
                    signature: hash_value(output),
                    status_signature: stable_status_signature(output, is_error.unwrap_or(false)),
                    changed: changed_value(output),
                },
            )),
            _ => None,
        })
        .collect()
}

fn observation_signature(call: &AiToolCall, class: ActionClass, result: &ResultEvidence) -> String {
    let (name, _) = effective_tool_call(&call.name, &call.arguments);
    if class == ActionClass::Observe && matches!(name, "bash" | "shell" | "execute") {
        // Re-running the same read-only shell intent must not look productive
        // merely because stdout contains a different timestamp, PID, elapsed
        // time, log prefix, or presentation detail. Preserve only explicit
        // lifecycle/error state; a real mutation epoch permits observation
        // again through the normal ledger reset.
        result.status_signature.clone()
    } else {
        result.signature.clone()
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
    match output {
        Value::Object(object) => object
            .get("changed")
            .and_then(Value::as_bool)
            .or_else(|| object.get("metadata").and_then(changed_value))
            .or_else(|| object.get("data").and_then(changed_value)),
        Value::String(serialized) => serde_json::from_str::<Value>(serialized)
            .ok()
            .as_ref()
            .and_then(changed_value),
        _ => None,
    }
}

pub fn action_fingerprint(call: &AiToolCall) -> ActionFingerprint {
    let (name, arguments) = effective_tool_call(&call.name, &call.arguments);
    let class = action_class(name, arguments, call);
    let normalized = if matches!(name, "bash" | "shell" | "execute") {
        arguments
            .get("command")
            .and_then(Value::as_str)
            .map(semantic_bash_signature)
            .unwrap_or_else(|| "missing-command".to_string())
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
            let mutates = arguments
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
        "agent" => match arguments.get("agent_type").and_then(Value::as_str) {
            Some("build") => ActionClass::Mutate,
            _ => ActionClass::Delegate,
        },
        "send_user_message" | "report" => ActionClass::Communicate,
        "set_work_mode" | "enter_plan_mode" | "task_start" | "task_complete" | "add_subtask"
        | "set_dependency" | "sleep" => ActionClass::Control,
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
            output: json!({ "ok": true, "metadata": { "changed": changed } }),
            is_error: None,
        }
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
    fn changed_evidence_and_successful_mutation_reset_no_progress() {
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
            .record_turn(&write, &[result("write-1", "written", false)])
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
    fn volatile_read_only_bash_output_cannot_evade_guard() {
        let mut ledger = ProgressLedger::new();

        for attempt in 0..=3 {
            let id = format!("bash-{attempt}");
            let calls = vec![call(
                &id,
                "bash",
                json!({ "command": "ps -axo pid,command" }),
            )];
            let results = vec![result(
                &id,
                &format!("pid={} timestamp={attempt}", 10_000 + attempt),
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
    fn explicit_process_status_change_is_new_evidence() {
        let mut ledger = ProgressLedger::new();
        let calls = vec![call(
            "bash",
            "bash",
            json!({ "command": "check-worker-status" }),
        )];
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
