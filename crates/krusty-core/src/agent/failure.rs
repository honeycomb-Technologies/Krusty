//! Repeated tool failure detection.
//!
//! Tracks tool error signatures across iterations and triggers a fail-fast
//! when the same tool keeps failing with the same error pattern, preventing
//! infinite retry loops.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::ai::types::{AiToolCall, Content, ModelMessage, Role};
use crate::tools::registry::{
    agent_call_execution_profile, effective_tool_call, tool_policy_for_call, ToolCategory,
};

/// Default threshold: stop after this many identical failures.
pub const REPEATED_FAILURE_THRESHOLD: usize = 2;
/// Default threshold: stop after this many identical read-only tool sequences.
pub const REPEATED_READ_ONLY_SEQUENCE_THRESHOLD: usize = 3;
/// Default threshold: stop after this many semantically equivalent successful
/// validation-only turns.
pub const REPEATED_VALIDATION_SEQUENCE_THRESHOLD: usize = 3;

fn exploration_signature(call: &AiToolCall) -> String {
    let (name, arguments) = effective_tool_call(&call.name, &call.arguments);
    let mut arguments = arguments.clone();

    // Increasing only an output cap is not a new exploration strategy. Treat
    // these calls as equivalent so agents cannot evade the loop guard by
    // repeatedly widening the same directory or glob result.
    if let Some(object) = arguments.as_object_mut() {
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

    format!("{}:{}", name, hash_arguments(&arguments))
}

/// Check tool results for repeated failures. Returns a diagnostic message
/// if the same tool+error signature has been seen `threshold` or more times.
///
/// On success, only the recovered tool signature is cleared.
pub fn detect_repeated_failures(
    counters: &mut HashMap<String, usize>,
    tool_calls: &[AiToolCall],
    tool_results: &[Content],
) -> Option<String> {
    let mut call_meta: HashMap<&str, (String, u64)> = HashMap::new();
    for call in tool_calls {
        call_meta.insert(
            call.id.as_str(),
            (call.name.clone(), hash_arguments(&call.arguments)),
        );
    }

    let mut recovered_calls = Vec::new();

    for result in tool_results {
        let Content::ToolResult {
            tool_use_id,
            output,
            is_error,
        } = result
        else {
            continue;
        };

        if !is_error.unwrap_or(false) {
            let Some((tool_name, args_hash)) = call_meta.get(tool_use_id.as_str()) else {
                continue;
            };
            recovered_calls.push((tool_name.clone(), *args_hash));
            continue;
        }

        let Some((tool_name, args_hash)) = call_meta.get(tool_use_id.as_str()) else {
            continue;
        };

        let output_str = match output {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let (error_code, error_fingerprint) = extract_error_signature(&output_str);
        let signature = format!(
            "{}|{}|{}|{}",
            tool_name, error_code, error_fingerprint, args_hash
        );
        let count = counters
            .entry(signature)
            .and_modify(|c| *c += 1)
            .or_insert(1);

        if *count >= REPEATED_FAILURE_THRESHOLD {
            return Some(format!(
                "Stopping tool loop: '{}' failed {} times with the same '{}' error. Do not retry the same command unchanged; inspect the previous output, fix the reported issue, or choose a different strategy.",
                tool_name, *count, error_code
            ));
        }
    }

    if !recovered_calls.is_empty() {
        counters.retain(|signature, _| {
            !recovered_calls
                .iter()
                .any(|(tool_name, args_hash)| matches_signature(signature, tool_name, *args_hash))
        });
    }

    None
}

/// Detect repeated identical read-only tool sequences across turns.
///
/// This guards against agent loops that keep reissuing the same exploration
/// pattern without taking action on the gathered evidence.
pub fn detect_repeated_read_only_sequence(
    counters: &mut HashMap<String, usize>,
    tool_calls: &[AiToolCall],
) -> Option<String> {
    if tool_calls.is_empty()
        || !tool_calls.iter().all(|call| {
            tool_policy_for_call(&call.name, &call.arguments).category == ToolCategory::ReadOnly
        })
    {
        counters.clear();
        return None;
    }

    let signature = tool_calls
        .iter()
        .map(exploration_signature)
        .collect::<Vec<_>>()
        .join("|");

    counters.retain(|key, _| key == &signature);
    let count = counters
        .entry(signature)
        .and_modify(|value| *value += 1)
        .or_insert(1);

    if *count >= REPEATED_READ_ONLY_SEQUENCE_THRESHOLD {
        return Some(format!(
            "Stopping exploration loop: the same read-only tool pattern repeated {} times without taking action. Act on the evidence or change strategy.",
            *count
        ));
    }

    None
}

/// Detect successful validation-only turns that repeat the same semantic work.
///
/// Validation commands frequently contain incidental differences (paths,
/// output formatting, or an extra assertion), so hashing the full Bash payload
/// misses the loop we actually care about. This signature records validation
/// families instead: test, lint, build, syntax, and similar intent.
pub fn detect_repeated_validation_sequence(
    counters: &mut HashMap<String, usize>,
    tool_calls: &[AiToolCall],
    tool_results: &[Content],
) -> Option<String> {
    if tool_calls.is_empty() || !tool_calls.iter().all(is_validation_call) {
        counters.clear();
        return None;
    }

    let successful_ids = tool_results
        .iter()
        .filter_map(|result| match result {
            Content::ToolResult {
                tool_use_id,
                is_error,
                ..
            } if !is_error.unwrap_or(false) => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    if !tool_calls
        .iter()
        .all(|call| successful_ids.contains(call.id.as_str()))
    {
        counters.clear();
        return None;
    }

    let signature = tool_calls
        .iter()
        .map(validation_signature)
        .collect::<Vec<_>>()
        .join("|");
    counters.retain(|key, _| key == &signature);
    let count = counters
        .entry(signature)
        .and_modify(|value| *value += 1)
        .or_insert(1);

    if *count >= REPEATED_VALIDATION_SEQUENCE_THRESHOLD {
        return Some(format!(
            "Stopping validation loop: the same successful validation pattern repeated {} times without new changes. The work is already validated; summarize the result instead of running it again.",
            *count
        ));
    }

    None
}

pub(crate) fn is_validation_call(call: &AiToolCall) -> bool {
    let (name, arguments) = effective_tool_call(&call.name, &call.arguments);
    if name == "agent" {
        return agent_call_execution_profile(arguments) == "verify";
    }
    if !matches!(name, "bash" | "shell" | "execute") {
        return false;
    }

    !validation_families(arguments).is_empty()
}

fn validation_signature(call: &AiToolCall) -> String {
    let (name, arguments) = effective_tool_call(&call.name, &call.arguments);
    if name == "agent" {
        return "agent:verify".to_string();
    }
    validation_families(arguments).join("+")
}

fn validation_families(arguments: &serde_json::Value) -> Vec<&'static str> {
    let command = arguments
        .get("command")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut families = Vec::new();
    let groups: &[(&str, &[&str])] = &[
        (
            "test",
            &[
                " test",
                "test ",
                "pytest",
                "go test",
                "mvn test",
                "gradle test",
                "forge test",
            ],
        ),
        ("check", &["cargo check", "git diff --check"]),
        ("lint", &["cargo clippy", " lint", "eslint"]),
        ("typecheck", &[" typecheck", "tsc "]),
        ("build", &[" run build", "cargo build", "npm run build"]),
        (
            "syntax",
            &["node --check", "py_compile", "json.tool", "syntax check"],
        ),
        ("integrity", &["integrity", "scaffold check", "assert "]),
    ];
    for (family, needles) in groups {
        if needles.iter().any(|needle| command.contains(needle)) {
            families.push(*family);
        }
    }
    families
}

/// Stop immediately when top-level exploration produced no usable evidence.
pub fn detect_terminal_explore_failure(
    tool_calls: &[AiToolCall],
    tool_results: &[Content],
) -> Option<String> {
    let explore_ids = tool_calls
        .iter()
        .filter(|call| {
            call.name == "agent" && agent_call_execution_profile(&call.arguments) == "explore"
        })
        .map(|call| call.id.as_str())
        .collect::<Vec<_>>();

    if explore_ids.is_empty() {
        return None;
    }

    for result in tool_results {
        let Content::ToolResult {
            tool_use_id,
            output,
            ..
        } = result
        else {
            continue;
        };

        if !explore_ids.contains(&tool_use_id.as_str()) {
            continue;
        }

        let parsed = match output {
            serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text).ok(),
            other => Some(other.clone()),
        }?;

        let summary = parsed
            .get("summary")
            .and_then(|value| value.as_str())
            .unwrap_or("explore returned degraded results");
        let result_payload = parsed.get("result").unwrap_or(&parsed);
        let status = result_payload
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if status == "background_started" {
            continue;
        }
        let outcome = result_payload
            .get("outcome")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let success = result_payload
            .get("success")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let usable_agents = result_payload
            .get("usable_agents")
            .or_else(|| result_payload.get("successful_agents"))
            .and_then(|value| value.as_u64());
        let files_examined_count = result_payload
            .get("files_examined_count")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);

        // New single-agent format: check success bool directly
        // Legacy multi-agent format: check usable_agents count
        let is_terminal = outcome == "failed"
            || !success
            || usable_agents.is_some_and(|count| count == 0)
            || files_examined_count == 0;

        if is_terminal {
            return Some(format!(
                "Stopping after degraded exploration: {} Summarize the failed investigation and ask the user to narrow scope before more reads.",
                summary
            ));
        }
    }

    None
}

/// Stop broad manual read-only probing when a prior explore already returned
/// usable delegated coverage and explicitly told the parent to summarize.
pub fn detect_post_explore_manual_fallback(
    conversation: &[ModelMessage],
    tool_calls: &[AiToolCall],
) -> Option<String> {
    if tool_calls.is_empty() {
        return None;
    }

    let read_only_probe = tool_calls
        .iter()
        .all(|call| matches!(call.name.as_str(), "read" | "glob" | "grep" | "list"));
    if !read_only_probe {
        return None;
    }

    let last_explore_result = conversation.iter().rev().find_map(|message| {
        if message.role != Role::User {
            return None;
        }

        message.content.iter().find_map(|content| {
            let Content::ToolResult { output, .. } = content else {
                return None;
            };

            let parsed = match output {
                serde_json::Value::String(text) => {
                    serde_json::from_str::<serde_json::Value>(text).ok()
                }
                other => Some(other.clone()),
            }?;

            let result_payload = parsed.get("result").unwrap_or(&parsed);
            let tool_name = parsed
                .get("tool")
                .and_then(|value| value.as_str())
                .or_else(|| result_payload.get("tool").and_then(|value| value.as_str()));
            if tool_name != Some("explore") {
                return None;
            }

            Some(result_payload.clone())
        })
    })?;

    let outcome = last_explore_result
        .get("outcome")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let usable_agents = last_explore_result
        .get("usable_agents")
        .or_else(|| last_explore_result.get("successful_agents"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let next_action_hint = last_explore_result
        .get("next_action_hint")
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    if usable_agents == 0 {
        return None;
    }

    if !matches!(outcome, "partial" | "success") {
        return None;
    }

    if next_action_hint.is_empty() {
        return None;
    }

    Some(
        "Stopping broad manual probing after delegated exploration: usable delegated coverage already exists. Summarize the gathered evidence, call out incomplete targets, and only continue if the user narrows scope.".to_string(),
    )
}

fn hash_arguments(arguments: &serde_json::Value) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    arguments.to_string().hash(&mut hasher);
    hasher.finish()
}

fn matches_signature(signature: &str, tool_name: &str, args_hash: u64) -> bool {
    let mut parts = signature.split('|');
    let Some(sig_tool_name) = parts.next() else {
        return false;
    };
    let Some(sig_args_hash) = signature.rsplit('|').next() else {
        return false;
    };
    sig_tool_name == tool_name && sig_args_hash == args_hash.to_string()
}

fn extract_error_signature(output_str: &str) -> (String, String) {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output_str) {
        if let Some(error) = value.get("error") {
            if let Some(error_obj) = error.as_object() {
                let message = error_obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let code = error_obj
                    .get("code")
                    .and_then(|v| v.as_str())
                    .map(|c| c.to_ascii_lowercase())
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| classify_error_code(message).to_string());
                return (code, normalize_error_fingerprint(message));
            }

            if let Some(message) = error.as_str() {
                return (
                    classify_error_code(message).to_string(),
                    normalize_error_fingerprint(message),
                );
            }
        }

        if let Some(signature) = extract_history_tool_error_signature(&value) {
            return signature;
        }
    }

    (
        classify_error_code(output_str).to_string(),
        normalize_error_fingerprint(output_str),
    )
}

fn extract_history_tool_error_signature(value: &serde_json::Value) -> Option<(String, String)> {
    let summary = value.get("summary").and_then(|v| v.as_str())?;
    let result = value.get("result");

    let mut message_parts = vec![summary.to_string()];
    if let Some(error) = result
        .and_then(|v| v.get("error"))
        .and_then(|v| v.as_str())
        .filter(|text| !text.trim().is_empty())
    {
        message_parts.push(error.to_string());
    }
    if let Some(output_preview) = result
        .and_then(|v| v.get("output_preview"))
        .and_then(|v| v.as_str())
        .filter(|text| !text.trim().is_empty())
    {
        message_parts.push(output_preview.to_string());
    }

    let message = message_parts.join("\n");
    let code = value
        .get("error_code")
        .and_then(|v| v.as_str())
        .map(|code| code.to_ascii_lowercase())
        .filter(|code| !code.is_empty())
        .unwrap_or_else(|| classify_error_code(&message).to_string());

    Some((code, normalize_error_fingerprint(&message)))
}

fn classify_error_code(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("invalid parameters")
        || lower.contains("missing field")
        || lower.contains("unknown field")
    {
        "invalid_parameters"
    } else if lower.contains("unknown tool") {
        "unknown_tool"
    } else if lower.contains("access denied") || lower.contains("outside workspace") {
        "access_denied"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if lower.contains("denied") {
        "permission_denied"
    } else {
        "tool_error"
    }
}

fn normalize_error_fingerprint(message: &str) -> String {
    let mut compact = String::new();
    for part in message.split_whitespace() {
        if !compact.is_empty() {
            compact.push(' ');
        }
        compact.push_str(part);
    }

    if compact.is_empty() {
        return "unknown".to_string();
    }

    compact.make_ascii_lowercase();
    compact.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trips_at_threshold() {
        let call = AiToolCall {
            id: "call_1".to_string(),
            name: "glob".to_string(),
            arguments: json!({"pattern":"**/*"}),
        };
        let result = Content::ToolResult {
            tool_use_id: "call_1".to_string(),
            output: serde_json::Value::String(
                r#"{"error":"Invalid parameters: missing field `pattern`"}"#.to_string(),
            ),
            is_error: Some(true),
        };

        let mut counters = HashMap::new();
        let first = detect_repeated_failures(
            &mut counters,
            std::slice::from_ref(&call),
            std::slice::from_ref(&result),
        );
        assert!(first.is_none());

        let second = detect_repeated_failures(
            &mut counters,
            std::slice::from_ref(&call),
            std::slice::from_ref(&result),
        );
        assert!(second.is_some());
    }

    #[test]
    fn success_clears_counters() {
        let call = AiToolCall {
            id: "call_1".to_string(),
            name: "glob".to_string(),
            arguments: json!({"pattern":"**/*"}),
        };

        let mut counters = HashMap::new();

        // First failure
        let fail_result = Content::ToolResult {
            tool_use_id: "call_1".to_string(),
            output: serde_json::Value::String("error".to_string()),
            is_error: Some(true),
        };
        detect_repeated_failures(
            &mut counters,
            std::slice::from_ref(&call),
            std::slice::from_ref(&fail_result),
        );
        assert!(!counters.is_empty());

        // Success resets
        let ok_result = Content::ToolResult {
            tool_use_id: "call_1".to_string(),
            output: serde_json::Value::String("ok".to_string()),
            is_error: None,
        };
        detect_repeated_failures(
            &mut counters,
            std::slice::from_ref(&call),
            std::slice::from_ref(&ok_result),
        );
        assert!(counters.is_empty());
    }

    #[test]
    fn success_only_clears_matching_signature() {
        let failing_call = AiToolCall {
            id: "call_fail".to_string(),
            name: "glob".to_string(),
            arguments: json!({"pattern":"src/**/*.rs"}),
        };
        let succeeding_call = AiToolCall {
            id: "call_ok".to_string(),
            name: "read".to_string(),
            arguments: json!({"file_path":"src/main.rs"}),
        };

        let fail_result = Content::ToolResult {
            tool_use_id: "call_fail".to_string(),
            output: serde_json::Value::String("timeout".to_string()),
            is_error: Some(true),
        };
        let ok_result = Content::ToolResult {
            tool_use_id: "call_ok".to_string(),
            output: serde_json::Value::String("ok".to_string()),
            is_error: Some(false),
        };

        let mut counters = HashMap::new();
        detect_repeated_failures(
            &mut counters,
            &[failing_call.clone(), succeeding_call.clone()],
            &[fail_result.clone(), ok_result.clone()],
        );
        assert_eq!(counters.len(), 1);

        let second = detect_repeated_failures(
            &mut counters,
            &[failing_call, succeeding_call],
            &[fail_result, ok_result],
        );
        assert!(second.is_some());
    }

    #[test]
    fn repeated_failure_uses_history_error_code_and_output_fingerprint() {
        let call = AiToolCall {
            id: "call_1".to_string(),
            name: "bash".to_string(),
            arguments: json!({"command":"git diff --cached --check"}),
        };
        let raw_output = json!({
            "ok": false,
            "error": {
                "code": "command_failed",
                "message": "Command exited with code 2"
            },
            "data": {
                "output": "crates/grok-auth/README.md:76: trailing whitespace.\n+Just use the default path.  "
            },
            "metadata": {"exit_code": 2, "killed": false}
        })
        .to_string();
        let result = Content::ToolResult {
            tool_use_id: "call_1".to_string(),
            output: crate::agent::history_policy::build_history_tool_result(
                "bash",
                &raw_output,
                true,
            ),
            is_error: Some(true),
        };

        let mut counters = HashMap::new();
        assert!(detect_repeated_failures(
            &mut counters,
            std::slice::from_ref(&call),
            std::slice::from_ref(&result),
        )
        .is_none());
        let diagnostic = detect_repeated_failures(
            &mut counters,
            std::slice::from_ref(&call),
            std::slice::from_ref(&result),
        )
        .expect("second identical command failure should trip loop guard");

        assert!(diagnostic.contains("same 'command_failed' error"));
    }

    #[test]
    fn classify_error_code_matches_categories() {
        assert_eq!(
            classify_error_code("Invalid parameters: missing field `x`"),
            "invalid_parameters"
        );
        assert_eq!(classify_error_code("unknown tool: foo"), "unknown_tool");
        assert_eq!(
            classify_error_code("access denied to /etc/passwd"),
            "access_denied"
        );
        assert_eq!(
            classify_error_code("operation timed out after 30s"),
            "timeout"
        );
        assert_eq!(classify_error_code("some random error"), "tool_error");
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(
            normalize_error_fingerprint("  A   spaced\n error\tmessage  "),
            "a spaced error message"
        );
    }

    #[test]
    fn repeated_read_only_sequence_trips_threshold() {
        let calls = vec![
            AiToolCall {
                id: "call_1".to_string(),
                name: "glob".to_string(),
                arguments: json!({"pattern":"src/**/*.rs"}),
            },
            AiToolCall {
                id: "call_2".to_string(),
                name: "grep".to_string(),
                arguments: json!({"pattern":"TODO"}),
            },
        ];

        let mut counters = HashMap::new();
        for _ in 0..(REPEATED_READ_ONLY_SEQUENCE_THRESHOLD - 1) {
            assert!(detect_repeated_read_only_sequence(&mut counters, &calls).is_none());
        }

        assert!(detect_repeated_read_only_sequence(&mut counters, &calls).is_some());
    }

    #[test]
    fn repeated_read_only_sequence_resets_on_write() {
        let readonly = vec![AiToolCall {
            id: "call_1".to_string(),
            name: "glob".to_string(),
            arguments: json!({"pattern":"src/**/*.rs"}),
        }];
        let write = vec![AiToolCall {
            id: "call_2".to_string(),
            name: "edit".to_string(),
            arguments: json!({"file_path":"src/main.rs"}),
        }];

        let mut counters = HashMap::new();
        detect_repeated_read_only_sequence(&mut counters, &readonly);
        assert!(!counters.is_empty());

        assert!(detect_repeated_read_only_sequence(&mut counters, &write).is_none());
        assert!(counters.is_empty());
    }

    #[test]
    fn repeated_semantic_validation_sequence_trips_despite_command_drift() {
        let validation = |id: &str, command: &str| AiToolCall {
            id: id.to_string(),
            name: "bash".to_string(),
            arguments: json!({"command": command}),
        };
        let success = |id: &str| Content::ToolResult {
            tool_use_id: id.to_string(),
            output: json!({"ok": true}),
            is_error: None,
        };
        let mut counters = HashMap::new();

        assert!(detect_repeated_validation_sequence(
            &mut counters,
            &[validation("v1", "npm test && node --check app.js")],
            &[success("v1")],
        )
        .is_none());
        assert!(detect_repeated_validation_sequence(
            &mut counters,
            &[validation(
                "v2",
                "npm test -- --runInBand && node --check src/app.js"
            )],
            &[success("v2")],
        )
        .is_none());
        let diagnostic = detect_repeated_validation_sequence(
            &mut counters,
            &[validation(
                "v3",
                "npm test -- --silent && node --check scripts/deploy.js",
            )],
            &[success("v3")],
        )
        .expect("third equivalent validation turn should stop");

        assert!(diagnostic.contains("already validated"));
    }

    #[test]
    fn failed_or_non_validation_turn_resets_validation_sequence() {
        let validation = AiToolCall {
            id: "v1".to_string(),
            name: "bash".to_string(),
            arguments: json!({"command": "cargo test --workspace"}),
        };
        let failed = Content::ToolResult {
            tool_use_id: "v1".to_string(),
            output: json!({"error": "test failed"}),
            is_error: Some(true),
        };
        let mut counters = HashMap::new();

        assert!(detect_repeated_validation_sequence(
            &mut counters,
            std::slice::from_ref(&validation),
            &[failed],
        )
        .is_none());
        assert!(counters.is_empty());

        let edit = AiToolCall {
            id: "e1".to_string(),
            name: "edit".to_string(),
            arguments: json!({"file_path": "src/lib.rs"}),
        };
        assert!(detect_repeated_validation_sequence(&mut counters, &[edit], &[]).is_none());
        assert!(counters.is_empty());
    }

    #[test]
    fn repeated_wrapped_list_calls_ignore_only_the_result_limit() {
        let wrapped = |id: &str, limit: usize| AiToolCall {
            id: id.to_string(),
            name: "tool_search".to_string(),
            arguments: json!({
                "action": "execute",
                "tool": "list",
                "arguments": {
                    "path": "/home/example",
                    "depth": 1,
                    "limit": limit
                }
            }),
        };

        let mut counters = HashMap::new();
        assert!(detect_repeated_read_only_sequence(&mut counters, &[wrapped("1", 100)]).is_none());
        assert!(detect_repeated_read_only_sequence(&mut counters, &[wrapped("2", 200)]).is_none());
        assert!(detect_repeated_read_only_sequence(&mut counters, &[wrapped("3", 300)]).is_some());
    }

    #[test]
    fn wrapped_write_tools_are_not_classified_as_read_only() {
        let wrapped_write = AiToolCall {
            id: "write-1".to_string(),
            name: "tool_search".to_string(),
            arguments: json!({
                "action": "execute",
                "tool": "write",
                "arguments": {"file_path": "src/main.rs", "content": "fn main() {}"}
            }),
        };

        let mut counters = HashMap::new();
        assert!(detect_repeated_read_only_sequence(&mut counters, &[wrapped_write]).is_none());
        assert!(counters.is_empty());
    }

    #[test]
    fn terminal_explore_failure_stops_after_zero_success_run() {
        let tool_calls = vec![AiToolCall {
            id: "tool-1".to_string(),
            name: "agent".to_string(),
            arguments: json!({"agent_type":"explore","prompt":"audit"}),
        }];
        let tool_results = vec![Content::ToolResult {
            tool_use_id: "tool-1".to_string(),
            output: json!({
                "summary": "Explore completed: 2 agents (0 succeeded, 2 failed), 0 turns, 0 files examined",
                "result": {
                    "outcome": "failed",
                    "successful_agents": 0,
                    "failed_agents": 2,
                    "files_examined_count": 0
                }
            }),
            is_error: None,
        }];

        let diagnostic = detect_terminal_explore_failure(&tool_calls, &tool_results)
            .expect("failed explore should stop");
        assert!(diagnostic.contains("degraded exploration"));
    }

    #[test]
    fn terminal_explore_failure_stops_after_zero_evidence_partial_run() {
        let tool_calls = vec![AiToolCall {
            id: "tool-1".to_string(),
            name: "agent".to_string(),
            arguments: json!({"agent_type":"explore","prompt":"audit"}),
        }];
        let tool_results = vec![Content::ToolResult {
            tool_use_id: "tool-1".to_string(),
            output: json!({
                "summary": "Explore completed: 2 agents (1 usable, 1 degraded, 0 failed), 8 turns, 0 files examined",
                "result": {
                    "outcome": "partial",
                    "usable_agents": 1,
                    "failed_agents": 0,
                    "files_examined_count": 0
                }
            }),
            is_error: None,
        }];

        let diagnostic = detect_terminal_explore_failure(&tool_calls, &tool_results)
            .expect("zero-evidence explore should stop");
        assert!(diagnostic.contains("degraded exploration"));
    }

    #[test]
    fn terminal_explore_failure_allows_background_start_to_continue() {
        let tool_calls = vec![AiToolCall {
            id: "tool-1".to_string(),
            name: "agent".to_string(),
            arguments: json!({
                "profile": "audit-specialist",
                "capabilities": ["read"],
                "prompt": "audit"
            }),
        }];
        let tool_results = vec![Content::ToolResult {
            tool_use_id: "tool-1".to_string(),
            output: json!({
                "summary": "explore agent background_started",
                "result": {
                    "status": "background_started",
                    "delegated_run_id": "run-1"
                }
            }),
            is_error: None,
        }];

        let diagnostic = detect_terminal_explore_failure(&tool_calls, &tool_results);
        assert!(diagnostic.is_none());
    }

    #[test]
    fn terminal_explore_failure_does_not_stop_successful_usable_run() {
        let tool_calls = vec![AiToolCall {
            id: "tool-1".to_string(),
            name: "agent".to_string(),
            arguments: json!({"agent_type":"explore","prompt":"audit"}),
        }];
        let tool_results = vec![Content::ToolResult {
            tool_use_id: "tool-1".to_string(),
            output: json!({
                "summary": "Explore completed: 2 agents (2 usable, 0 degraded, 0 failed), 10 turns, 24 files examined",
                "result": {
                    "outcome": "success",
                    "usable_agents": 2,
                    "successful_agents": 2,
                    "failed_agents": 0,
                    "files_examined_count": 24
                }
            }),
            is_error: None,
        }];

        let diagnostic = detect_terminal_explore_failure(&tool_calls, &tool_results);
        assert!(diagnostic.is_none());
    }

    #[test]
    fn post_explore_manual_fallback_stops_read_only_probe_after_partial_usable_run() {
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::ToolResult {
                tool_use_id: "explore-1".to_string(),
                output: serde_json::json!({
                    "tool": "explore",
                    "result": {
                        "outcome": "partial",
                        "usable_agents": 1,
                        "next_action_hint": "Use the evidence already gathered."
                    }
                }),
                is_error: Some(false),
            }],
        }];
        let tool_calls = vec![AiToolCall {
            id: "read-1".to_string(),
            name: "read".to_string(),
            arguments: serde_json::json!({"file_path":"src/main.rs"}),
        }];

        let diagnostic = detect_post_explore_manual_fallback(&conversation, &tool_calls)
            .expect("manual fallback should stop");
        assert!(diagnostic.contains("usable delegated coverage already exists"));
    }

    #[test]
    fn post_explore_manual_fallback_allows_non_probe_actions() {
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::ToolResult {
                tool_use_id: "explore-1".to_string(),
                output: serde_json::json!({
                    "tool": "explore",
                    "result": {
                        "outcome": "partial",
                        "usable_agents": 1,
                        "next_action_hint": "Use the evidence already gathered."
                    }
                }),
                is_error: Some(false),
            }],
        }];
        let tool_calls = vec![AiToolCall {
            id: "bash-1".to_string(),
            name: "bash".to_string(),
            arguments: serde_json::json!({"command":"cargo test"}),
        }];

        let diagnostic = detect_post_explore_manual_fallback(&conversation, &tool_calls);
        assert!(diagnostic.is_none());
    }
}
