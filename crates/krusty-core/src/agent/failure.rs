//! Repeated tool failure detection.
//!
//! Tracks tool error signatures across iterations and triggers a fail-fast
//! when the same tool keeps failing with the same error pattern, preventing
//! infinite retry loops.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::ai::types::{AiToolCall, Content};
use crate::tools::registry::{tool_policy, ToolCategory};

/// Default threshold: stop after this many identical failures.
pub const REPEATED_FAILURE_THRESHOLD: usize = 2;
/// Default threshold: stop after this many identical read-only tool sequences.
pub const REPEATED_READ_ONLY_SEQUENCE_THRESHOLD: usize = 4;

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
                "Stopping tool loop: '{}' failed {} times with the same '{}' error. A different strategy is required.",
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
        || !tool_calls
            .iter()
            .all(|call| tool_policy(&call.name).category == ToolCategory::ReadOnly)
    {
        counters.clear();
        return None;
    }

    let signature = tool_calls
        .iter()
        .map(|call| format!("{}:{}", call.name, hash_arguments(&call.arguments)))
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
    }

    (
        classify_error_code(output_str).to_string(),
        normalize_error_fingerprint(output_str),
    )
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
}
