//! Tool-result retention and shaping for conversation history.
//!
//! UI-facing tool output can stay verbose, but the model context should keep
//! only the evidence needed to continue the task.

mod summaries;

use std::collections::BTreeSet;

use serde_json::{json, Value};

use self::summaries::{summarize_tool_result, summarized_result};

const MAX_PREVIEW_CHARS: usize = 2_000;
const MAX_FULL_RESULT_STRING_CHARS: usize = 16_000;
const MAX_READ_CONTENT_CHARS: usize = 16_000;
const MAX_DIFF_CHARS: usize = 6_000;
const MAX_BASH_OUTPUT_CHARS: usize = 3_000;
const MAX_LIST_ITEMS: usize = 12;
const MAX_MATCH_ITEMS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolRetention {
    RetainFull,
    SummarizeAfterTurn,
    DropAfterCompaction,
}

impl ToolRetention {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RetainFull => "retain_full",
            Self::SummarizeAfterTurn => "summarize_after_turn",
            Self::DropAfterCompaction => "drop_after_compaction",
        }
    }
}

pub(crate) fn tool_retention(name: &str) -> ToolRetention {
    match name {
        "grep" | "glob" | "list" | "write" | "edit" | "multiedit" | "apply_patch" => {
            ToolRetention::SummarizeAfterTurn
        }
        "agent" | "bash" | "processes" | "web_search" | "web_fetch" | "explore" | "build" => {
            ToolRetention::DropAfterCompaction
        }
        _ => ToolRetention::RetainFull,
    }
}

pub(crate) fn build_history_tool_result(
    tool_name: &str,
    raw_output: &str,
    is_error: bool,
) -> Value {
    let retention = tool_retention(tool_name);
    let parsed_output = serde_json::from_str::<Value>(raw_output)
        .unwrap_or_else(|_| Value::String(raw_output.to_string()));

    let summary = summarize_tool_result(tool_name, &parsed_output, raw_output, is_error);
    let error_code = is_error
        .then(|| parsed_error_code(&parsed_output))
        .flatten();
    let result = match retention {
        ToolRetention::RetainFull => bounded_full_result(&parsed_output),
        ToolRetention::SummarizeAfterTurn | ToolRetention::DropAfterCompaction => {
            summarized_result(tool_name, &parsed_output, raw_output)
        }
    };

    let mut history = json!({
        "tool": tool_name,
        "retention": retention.as_str(),
        "summary": summary,
        "is_error": is_error,
        "result": result,
    });
    if let Some(error_code) = error_code {
        if let Some(object) = history.as_object_mut() {
            object.insert("error_code".to_string(), Value::String(error_code));
        }
    }
    if let Some(changed) = parsed_output.get("changed").and_then(Value::as_bool) {
        if let Some(object) = history.as_object_mut() {
            object.insert("changed".to_string(), Value::Bool(changed));
        }
    }
    history
}

fn parsed_error_code(parsed: &Value) -> Option<String> {
    let error = parsed.get("error")?;
    if let Some(code) = error
        .get("code")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        return Some(code.to_ascii_lowercase());
    }

    error
        .as_str()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|_| "tool_error".to_string())
}

fn bounded_full_result(parsed: &Value) -> Value {
    let mut bounded = parsed.clone();
    bound_value(&mut bounded, None, 0);
    bounded
}

fn bound_value(value: &mut Value, key_hint: Option<&str>, depth: usize) {
    if depth > 6 {
        *value = Value::String("[TRUNCATED DEPTH]".to_string());
        return;
    }

    match value {
        Value::String(text) => {
            let limit = match key_hint {
                Some("content") => MAX_READ_CONTENT_CHARS,
                Some("diff") => MAX_DIFF_CHARS,
                Some("output") => MAX_BASH_OUTPUT_CHARS,
                Some("preview") => MAX_PREVIEW_CHARS,
                _ => MAX_FULL_RESULT_STRING_CHARS,
            };
            if text.len() > limit {
                *text = format!(
                    "{}\n\n[... TRUNCATED FOR HISTORY ...]",
                    truncate_utf8(text, limit)
                );
            }
        }
        Value::Array(items) => {
            if items.len() > MAX_LIST_ITEMS {
                items.truncate(MAX_LIST_ITEMS);
            }
            for item in items.iter_mut() {
                bound_value(item, key_hint, depth + 1);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                bound_value(child, Some(key.as_str()), depth + 1);
            }
        }
        _ => {}
    }
}

fn tool_payload(parsed: &Value) -> &Value {
    parsed.get("data").unwrap_or(parsed)
}

fn diff_preview(parsed: &Value) -> Option<String> {
    parsed
        .get("diff")
        .and_then(|value| value.as_str())
        .map(|diff| truncate_utf8(diff, MAX_DIFF_CHARS))
}

fn truncate_array_strings(items: &[Value], limit: usize, max_chars: usize) -> Vec<String> {
    items
        .iter()
        .take(limit)
        .filter_map(|value| value.as_str().map(|text| truncate_utf8(text, max_chars)))
        .collect()
}

fn unique_file_count(matches: &[Value]) -> usize {
    let mut files = BTreeSet::new();
    for entry in matches {
        if let Some(file) = entry.get("file").and_then(|value| value.as_str()) {
            files.insert(file.to_string());
        }
    }
    files.len()
}

pub(crate) fn truncate_utf8(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }

    let mut boundary = limit.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }

    let truncated = &text[..boundary];
    let window_start = boundary.saturating_sub(160);
    let break_point = truncated[window_start..]
        .rfind('\n')
        .map(|relative| window_start + relative)
        .unwrap_or(boundary);
    truncated[..break_point].trim_end().to_string()
}

pub(crate) fn truncate_utf8_head_tail(text: &str, head_limit: usize, tail_limit: usize) -> String {
    if text.len() <= head_limit.saturating_add(tail_limit) {
        return text.to_string();
    }

    let mut head_end = head_limit.min(text.len());
    while head_end > 0 && !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let head_window_start = head_end.saturating_sub(160);
    if let Some(relative_break) = text[head_window_start..head_end].rfind('\n') {
        head_end = head_window_start + relative_break;
    }

    let mut tail_start = text.len().saturating_sub(tail_limit);
    while tail_start < text.len() && !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let tail_window_end = tail_start.saturating_add(160).min(text.len());
    if let Some(relative_break) = text[tail_start..tail_window_end].find('\n') {
        tail_start += relative_break + 1;
    }

    format!(
        "{}\n...[middle truncated]...\n{}",
        text[..head_end].trim_end(),
        text[tail_start..].trim_start()
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_history_tool_result, tool_retention, truncate_utf8, ToolRetention};

    #[test]
    fn prefix_truncation_does_not_collapse_to_an_early_heading() {
        let value = format!("# Plan\n{}", "x".repeat(2_000));
        let truncated = truncate_utf8(&value, 600);

        assert!(truncated.starts_with("# Plan\n"));
        assert!(truncated.len() > 500);
    }

    #[test]
    fn classifies_bash_as_drop_after_compaction() {
        assert_eq!(tool_retention("bash"), ToolRetention::DropAfterCompaction);
    }

    #[test]
    fn classifies_canonical_agent_as_drop_after_compaction() {
        assert_eq!(tool_retention("agent"), ToolRetention::DropAfterCompaction);
    }

    #[test]
    fn classifies_write_tools_as_summarize_after_turn() {
        assert_eq!(tool_retention("write"), ToolRetention::SummarizeAfterTurn);
        assert_eq!(tool_retention("edit"), ToolRetention::SummarizeAfterTurn);
        assert_eq!(
            tool_retention("multiedit"),
            ToolRetention::SummarizeAfterTurn
        );
        assert_eq!(
            tool_retention("apply_patch"),
            ToolRetention::SummarizeAfterTurn
        );
    }

    #[test]
    fn summarizes_grep_results_for_history() {
        let output = json!({
            "ok": true,
            "data": {
                "matches": [
                    {"file": "src/lib.rs", "line_number": 10, "line": "fn main() {"},
                    {"file": "src/lib.rs", "line_number": 20, "line": "fn test() {"}
                ],
                "total_matches": 2
            }
        })
        .to_string();

        let history = build_history_tool_result("grep", &output, false);
        assert_eq!(
            history.get("retention").and_then(|value| value.as_str()),
            Some("summarize_after_turn")
        );
        assert!(history
            .get("summary")
            .and_then(|value| value.as_str())
            .is_some_and(|summary| summary.contains("2 matches")));
    }

    #[test]
    fn bash_error_history_keeps_actionable_output_and_code() {
        let output = json!({
            "ok": false,
            "error": {
                "code": "command_failed",
                "message": "Command exited with code 2"
            },
            "data": {
                "output": "crates/grok-auth/README.md:76: trailing whitespace.\n+Just use the default path.  "
            },
            "metadata": {
                "exit_code": 2,
                "killed": false
            }
        })
        .to_string();

        let history = build_history_tool_result("bash", &output, true);
        assert_eq!(
            history.get("error_code").and_then(|value| value.as_str()),
            Some("command_failed")
        );
        assert!(history
            .get("summary")
            .and_then(|value| value.as_str())
            .is_some_and(|summary| summary.contains("trailing whitespace")));
        assert!(history
            .get("result")
            .and_then(|value| value.get("output_preview"))
            .and_then(|value| value.as_str())
            .is_some_and(|preview| preview.contains("crates/grok-auth/README.md:76")));
    }

    #[test]
    fn background_bash_history_keeps_process_handle_and_status_guidance() {
        let output = json!({
            "ok": true,
            "data": {
                "message": "Process started in background",
                "process_id": "process-123",
                "status": "running",
                "endpoint_hints": ["127.0.0.1:5940"],
                "next_action": "Use processes status/control when needed."
            }
        })
        .to_string();

        let history = build_history_tool_result("bash", &output, false);

        assert!(history
            .get("summary")
            .and_then(|value| value.as_str())
            .is_some_and(|summary| summary.contains("process-123")
                && summary.contains("127.0.0.1:5940")
                && summary.contains("processes status/control")));
        assert_eq!(
            history
                .get("result")
                .and_then(|value| value.get("process_id"))
                .and_then(|value| value.as_str()),
            Some("process-123")
        );
        assert_eq!(
            history
                .get("result")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str()),
            Some("running")
        );
        assert_eq!(
            history
                .get("result")
                .and_then(|value| value.get("endpoint_hints")),
            Some(&json!(["127.0.0.1:5940"]))
        );
    }

    #[test]
    fn agent_history_is_bounded_but_keeps_handoff_evidence() {
        let findings = format!("# Plan\n{}\nFINAL-CONCLUSION-SENTINEL", "x".repeat(20_000));
        let output = json!({
            "ok": true,
            "data": {
                "delegated_run_id": "run-123",
                "outcome": "success",
                "confidence": "high",
                "investigation_summary": "Implemented the requested component and handed off its preview process.",
                "findings": findings,
                "paths_examined": ["src/lib.rs", "src/main.rs"],
                "paths_examined_count": 2,
                "agent_count": 1,
                "successful_agents": 1,
                "failed_agents": 0,
                "files_modified": 2,
                "builders": [{"unbounded_nested_payload": "do not retain this"}],
                "background_processes": [{
                    "process_id": "process-123",
                    "status": "running",
                    "command": "npm run dev -- --port 5940",
                    "working_dir": "/workspace",
                    "endpoint_hints": ["127.0.0.1:5940"],
                    "reused_existing": false
                }]
            }
        })
        .to_string();

        let history = build_history_tool_result("agent", &output, false);
        assert_eq!(
            history.get("retention").and_then(|value| value.as_str()),
            Some("drop_after_compaction")
        );
        let result = history.get("result").expect("structured agent result");
        assert_eq!(
            result
                .get("delegated_run_id")
                .and_then(|value| value.as_str()),
            Some("run-123")
        );
        let retained_findings = result
            .get("findings")
            .and_then(|value| value.as_str())
            .expect("bounded findings");
        assert!(retained_findings.len() <= 3_000);
        assert!(retained_findings.contains("# Plan"));
        assert!(retained_findings.contains("xxxxx"));
        assert!(retained_findings.contains("FINAL-CONCLUSION-SENTINEL"));
        assert!(retained_findings.contains("middle truncated"));
        assert_eq!(
            result
                .get("background_processes")
                .and_then(|value| value.get(0))
                .and_then(|value| value.get("endpoint_hints")),
            Some(&json!(["127.0.0.1:5940"]))
        );
        assert!(result.get("builders").is_none());
        assert!(!history.to_string().contains("unbounded_nested_payload"));
    }

    #[test]
    fn retains_bounded_read_content_in_history() {
        let content = "x".repeat(20_000);
        let output = json!({
            "ok": true,
            "data": {
                "content": content,
                "total_lines": 500,
                "lines_returned": 500,
                "start_line": 1
            }
        })
        .to_string();

        let history = build_history_tool_result("read", &output, false);
        assert_eq!(
            history.get("retention").and_then(|value| value.as_str()),
            Some("retain_full")
        );
        let rendered = history
            .get("result")
            .and_then(|value| value.get("data"))
            .and_then(|value| value.get("content"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        assert!(rendered.len() < 18_000);
        assert!(rendered.contains("TRUNCATED FOR HISTORY"));
    }

    #[test]
    fn write_history_contract_keeps_path_and_diff_preview() {
        let output = json!({
            "ok": true,
            "changed": true,
            "data": {
                "message": "Created new file (3 lines)",
                "bytes_written": 42,
                "line_count": 3,
                "file_path": "src/lib.rs"
            },
            "diff": "--- src/lib.rs\n+++ src/lib.rs\n@@\n+fn main() {}\n"
        })
        .to_string();

        let history = build_history_tool_result("write", &output, false);
        assert_eq!(
            history.get("retention").and_then(|value| value.as_str()),
            Some("summarize_after_turn")
        );
        assert_eq!(
            history.get("changed").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            history
                .get("result")
                .and_then(|value| value.get("file_path"))
                .and_then(|value| value.as_str()),
            Some("src/lib.rs")
        );
        assert!(history
            .get("result")
            .and_then(|value| value.get("diff_preview"))
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.contains("+++ src/lib.rs")));
    }

    #[test]
    fn apply_patch_history_contract_keeps_file_lists() {
        let output = json!({
            "ok": true,
            "data": {
                "message": "Applied patch: 1 modified, 1 created, 0 deleted",
                "files_modified": ["src/lib.rs"],
                "files_created": ["src/new.rs"],
                "files_deleted": []
            }
        })
        .to_string();

        let history = build_history_tool_result("apply_patch", &output, false);
        assert_eq!(
            history.get("summary").and_then(|value| value.as_str()),
            Some("apply_patch changed 1 modified, 1 created, 0 deleted files")
        );
        assert_eq!(
            history
                .get("result")
                .and_then(|value| value.get("files_modified"))
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(1)
        );
    }
}
