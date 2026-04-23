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
        "bash" | "processes" | "web_search" | "web_fetch" | "explore" | "build" => {
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
    let result = match retention {
        ToolRetention::RetainFull => bounded_full_result(&parsed_output),
        ToolRetention::SummarizeAfterTurn | ToolRetention::DropAfterCompaction => {
            summarized_result(tool_name, &parsed_output, raw_output)
        }
    };

    json!({
        "tool": tool_name,
        "retention": retention.as_str(),
        "summary": summary,
        "is_error": is_error,
        "result": result,
    })
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
    let break_point = truncated.rfind('\n').unwrap_or(boundary);
    truncated[..break_point].trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_history_tool_result, tool_retention, ToolRetention};

    #[test]
    fn classifies_bash_as_drop_after_compaction() {
        assert_eq!(tool_retention("bash"), ToolRetention::DropAfterCompaction);
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
