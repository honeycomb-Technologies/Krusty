//! Tool-result retention and shaping for conversation history.
//!
//! UI-facing tool output can stay verbose, but the model context should keep
//! only the evidence needed to continue the task.

use std::collections::BTreeSet;

use serde_json::{json, Value};

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

    pub(crate) fn from_output_value(output: &Value) -> Self {
        let Some(retention) = output.get("retention").and_then(|value| value.as_str()) else {
            return Self::RetainFull;
        };

        match retention {
            "summarize_after_turn" => Self::SummarizeAfterTurn,
            "drop_after_compaction" => Self::DropAfterCompaction,
            _ => Self::RetainFull,
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

fn summarize_tool_result(
    tool_name: &str,
    parsed: &Value,
    raw_output: &str,
    is_error: bool,
) -> String {
    match tool_name {
        "read" => summarize_read(parsed, is_error),
        "grep" => summarize_grep(parsed, is_error),
        "glob" => summarize_glob(parsed, is_error),
        "list" => summarize_list(parsed, is_error),
        "bash" => summarize_bash(parsed, is_error),
        "write" => summarize_write(parsed, is_error),
        "edit" => summarize_edit(parsed, is_error),
        "multiedit" => summarize_multiedit(parsed, is_error),
        "apply_patch" => summarize_apply_patch(parsed, is_error),
        "explore" => {
            summarize_structured_or_large_text_tool(parsed, "explore", raw_output, is_error)
        }
        "build" => summarize_structured_or_large_text_tool(parsed, "build", raw_output, is_error),
        _ => summarize_generic(parsed, raw_output, is_error),
    }
}

fn summarized_result(tool_name: &str, parsed: &Value, raw_output: &str) -> Value {
    match tool_name {
        "grep" => summarize_grep_result(parsed),
        "glob" => summarize_glob_result(parsed),
        "list" => summarize_list_result(parsed),
        "bash" => summarize_bash_result(parsed),
        "write" => summarize_write_result(parsed),
        "edit" => summarize_edit_result(parsed),
        "multiedit" => summarize_multiedit_result(parsed),
        "apply_patch" => summarize_apply_patch_result(parsed),
        "explore" | "build" => json!({
            "preview": truncate_utf8(raw_output, MAX_PREVIEW_CHARS),
        }),
        _ => json!({
            "preview": truncate_utf8(raw_output, MAX_PREVIEW_CHARS),
        }),
    }
}

fn summarize_read(parsed: &Value, is_error: bool) -> String {
    if is_error {
        return summarize_message_tool(parsed, true);
    }

    let payload = tool_payload(parsed);
    let lines_returned = payload
        .get("lines_returned")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let start_line = payload
        .get("start_line")
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    let total_lines = payload
        .get("total_lines")
        .and_then(|value| value.as_u64())
        .unwrap_or(lines_returned);

    format!(
        "read returned {} lines starting at line {} (file has {} total lines)",
        lines_returned, start_line, total_lines
    )
}

fn summarize_grep(parsed: &Value, is_error: bool) -> String {
    if is_error {
        return summarize_message_tool(parsed, true);
    }

    let payload = tool_payload(parsed);
    if let Some(total_matches) = payload
        .get("total_matches")
        .and_then(|value| value.as_u64())
    {
        let file_count = payload
            .get("matches")
            .and_then(|value| value.as_array())
            .map(|matches| unique_file_count(matches))
            .unwrap_or(0);
        return format!(
            "grep found {} matches across {} files",
            total_matches, file_count
        );
    }

    if let Some(count) = payload.get("count").and_then(|value| value.as_u64()) {
        return format!("grep found matches in {} files", count);
    }

    summarize_generic(parsed, &parsed.to_string(), false)
}

fn summarize_glob(parsed: &Value, is_error: bool) -> String {
    if is_error {
        return summarize_message_tool(parsed, true);
    }

    let payload = tool_payload(parsed);
    let count = payload
        .get("count")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            payload
                .get("matches")
                .and_then(|value| value.as_array())
                .map(|matches| matches.len() as u64)
        })
        .unwrap_or(0);
    format!("glob matched {} paths", count)
}

fn summarize_list(parsed: &Value, is_error: bool) -> String {
    if is_error {
        return summarize_message_tool(parsed, true);
    }

    let payload = tool_payload(parsed);
    let total_entries = payload
        .get("total_entries")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let directories = payload
        .get("directories")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let files = payload
        .get("files")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);

    format!(
        "list returned {} entries ({} dirs, {} files)",
        total_entries, directories, files
    )
}

fn summarize_bash(parsed: &Value, is_error: bool) -> String {
    let exit_code = parsed
        .get("metadata")
        .and_then(|value| value.get("exit_code"))
        .and_then(|value| value.as_i64())
        .unwrap_or(if is_error { 1 } else { 0 });

    if is_error {
        let message = parsed
            .get("error")
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or("command failed");
        return format!("bash failed (exit {}): {}", exit_code, message);
    }

    format!("bash completed successfully (exit {})", exit_code)
}

fn summarize_write(parsed: &Value, is_error: bool) -> String {
    summarize_path_message(parsed, is_error, |payload| {
        let line_count = payload.get("line_count").and_then(|value| value.as_u64());
        match line_count {
            Some(lines) => format!("wrote {} lines", lines),
            None => "wrote file".to_string(),
        }
    })
}

fn summarize_edit(parsed: &Value, is_error: bool) -> String {
    summarize_path_message(parsed, is_error, |payload| {
        let replacements = payload
            .get("replacements")
            .and_then(|value| value.as_u64())
            .unwrap_or(1);
        if let Some(pass) = payload.get("match_pass").and_then(|value| value.as_u64()) {
            format!(
                "replaced {} occurrence(s) with fuzzy pass {}",
                replacements, pass
            )
        } else {
            format!("replaced {} occurrence(s)", replacements)
        }
    })
}

fn summarize_multiedit(parsed: &Value, is_error: bool) -> String {
    summarize_path_message(parsed, is_error, |payload| {
        let applied = payload
            .get("edits_applied")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let total = payload
            .get("edits_total")
            .and_then(|value| value.as_u64())
            .unwrap_or(applied);
        if payload
            .get("partial")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            format!("applied {}/{} edits with partial success", applied, total)
        } else {
            format!("applied {}/{} edits", applied, total)
        }
    })
}

fn summarize_apply_patch(parsed: &Value, is_error: bool) -> String {
    if is_error {
        return summarize_message_tool(parsed, true);
    }

    let payload = tool_payload(parsed);
    let modified = payload
        .get("files_modified")
        .and_then(|value| value.as_array())
        .map(|files| files.len())
        .unwrap_or(0);
    let created = payload
        .get("files_created")
        .and_then(|value| value.as_array())
        .map(|files| files.len())
        .unwrap_or(0);
    let deleted = payload
        .get("files_deleted")
        .and_then(|value| value.as_array())
        .map(|files| files.len())
        .unwrap_or(0);

    format!(
        "apply_patch changed {} modified, {} created, {} deleted files",
        modified, created, deleted
    )
}

fn summarize_message_tool(parsed: &Value, is_error: bool) -> String {
    if is_error {
        if let Some(message) = parsed
            .get("error")
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
        {
            return truncate_utf8(message, MAX_PREVIEW_CHARS);
        }
    }

    tool_payload(parsed)
        .get("message")
        .and_then(|value| value.as_str())
        .map(|message| truncate_utf8(message, MAX_PREVIEW_CHARS))
        .unwrap_or_else(|| summarize_generic(parsed, &parsed.to_string(), is_error))
}

fn summarize_large_text_tool(tool_name: &str, raw_output: &str, is_error: bool) -> String {
    let first_line = raw_output
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| truncate_utf8(line.trim(), 160))
        .unwrap_or_else(|| tool_name.to_string());

    if is_error {
        format!("{} failed: {}", tool_name, first_line)
    } else {
        format!("{} returned aggregated findings: {}", tool_name, first_line)
    }
}

fn summarize_structured_or_large_text_tool(
    parsed: &Value,
    tool_name: &str,
    raw_output: &str,
    is_error: bool,
) -> String {
    if parsed.is_object() && tool_payload(parsed).get("message").is_some() {
        return summarize_message_tool(parsed, is_error);
    }

    summarize_large_text_tool(tool_name, raw_output, is_error)
}

fn summarize_generic(parsed: &Value, raw_output: &str, is_error: bool) -> String {
    if is_error {
        if let Some(message) = parsed
            .get("error")
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
        {
            return truncate_utf8(message, MAX_PREVIEW_CHARS);
        }
    }

    truncate_utf8(raw_output, MAX_PREVIEW_CHARS)
}

fn summarize_grep_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);

    if let Some(matches) = payload.get("matches").and_then(|value| value.as_array()) {
        let preview = matches
            .iter()
            .take(MAX_MATCH_ITEMS)
            .map(|entry| {
                json!({
                    "file": entry.get("file").and_then(|value| value.as_str()).unwrap_or(""),
                    "line_number": entry.get("line_number").and_then(|value| value.as_u64()),
                    "line": entry
                        .get("line")
                        .and_then(|value| value.as_str())
                        .map(|line| truncate_utf8(line, 200))
                        .unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();

        return json!({
            "total_matches": payload
                .get("total_matches")
                .and_then(|value| value.as_u64())
                .unwrap_or(matches.len() as u64),
            "matches": preview,
        });
    }

    if let Some(files) = payload.get("files").and_then(|value| value.as_array()) {
        return json!({
            "count": payload
                .get("count")
                .and_then(|value| value.as_u64())
                .unwrap_or(files.len() as u64),
            "files": truncate_array_strings(files, MAX_LIST_ITEMS, 180),
        });
    }

    if let Some(counts) = payload.get("counts").and_then(|value| value.as_array()) {
        return json!({
            "total": payload.get("total").and_then(|value| value.as_u64()).unwrap_or(0),
            "counts": counts.iter().take(MAX_LIST_ITEMS).cloned().collect::<Vec<_>>(),
        });
    }

    json!({
        "preview": truncate_utf8(&parsed.to_string(), MAX_PREVIEW_CHARS),
    })
}

fn summarize_glob_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);
    let matches = payload
        .get("matches")
        .and_then(|value| value.as_array())
        .map(|entries| truncate_array_strings(entries, MAX_LIST_ITEMS, 180))
        .unwrap_or_default();

    json!({
        "count": payload.get("count").and_then(|value| value.as_u64()).unwrap_or(matches.len() as u64),
        "matches": matches,
    })
}

fn summarize_list_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);
    let output_preview = payload
        .get("output")
        .and_then(|value| value.as_str())
        .map(|output| {
            output
                .lines()
                .take(MAX_LIST_ITEMS)
                .map(|line| truncate_utf8(line, 180))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "total_entries": payload.get("total_entries").and_then(|value| value.as_u64()).unwrap_or(0),
        "directories": payload.get("directories").and_then(|value| value.as_u64()).unwrap_or(0),
        "files": payload.get("files").and_then(|value| value.as_u64()).unwrap_or(0),
        "entries": output_preview,
    })
}

fn summarize_bash_result(parsed: &Value) -> Value {
    let output_preview = tool_payload(parsed)
        .get("output")
        .and_then(|value| value.as_str())
        .map(|output| truncate_utf8(output, MAX_BASH_OUTPUT_CHARS))
        .unwrap_or_default();

    json!({
        "exit_code": parsed
            .get("metadata")
            .and_then(|value| value.get("exit_code"))
            .and_then(|value| value.as_i64()),
        "killed": parsed
            .get("metadata")
            .and_then(|value| value.get("killed"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        "error": parsed
            .get("error")
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str()),
        "output_preview": output_preview,
    })
}

fn summarize_write_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);
    json!({
        "message": payload.get("message").and_then(|value| value.as_str()),
        "file_path": payload.get("file_path").and_then(|value| value.as_str()),
        "bytes_written": payload.get("bytes_written").and_then(|value| value.as_u64()),
        "line_count": payload.get("line_count").and_then(|value| value.as_u64()),
        "diff_preview": diff_preview(parsed),
    })
}

fn summarize_edit_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);
    json!({
        "message": payload.get("message").and_then(|value| value.as_str()),
        "file_path": payload.get("file_path").and_then(|value| value.as_str()),
        "replacements": payload.get("replacements").and_then(|value| value.as_u64()),
        "match_pass": payload.get("match_pass").and_then(|value| value.as_u64()),
        "warnings": parsed.get("warnings").cloned(),
        "diff_preview": diff_preview(parsed),
    })
}

fn summarize_multiedit_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);
    json!({
        "message": payload.get("message").and_then(|value| value.as_str()),
        "file_path": payload.get("file_path").and_then(|value| value.as_str()),
        "edits_applied": payload.get("edits_applied").and_then(|value| value.as_u64()),
        "edits_total": payload.get("edits_total").and_then(|value| value.as_u64()),
        "partial": payload.get("partial").and_then(|value| value.as_bool()),
        "warnings": parsed.get("warnings").cloned(),
        "diff_preview": diff_preview(parsed),
    })
}

fn summarize_apply_patch_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);
    json!({
        "message": payload.get("message").and_then(|value| value.as_str()),
        "files_modified": truncate_array_strings(
            payload
                .get("files_modified")
                .and_then(|value| value.as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            MAX_LIST_ITEMS,
            180,
        ),
        "files_created": truncate_array_strings(
            payload
                .get("files_created")
                .and_then(|value| value.as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            MAX_LIST_ITEMS,
            180,
        ),
        "files_deleted": truncate_array_strings(
            payload
                .get("files_deleted")
                .and_then(|value| value.as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            MAX_LIST_ITEMS,
            180,
        ),
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

fn summarize_path_message(
    parsed: &Value,
    is_error: bool,
    action: impl FnOnce(&Value) -> String,
) -> String {
    if is_error {
        return summarize_message_tool(parsed, true);
    }

    let payload = tool_payload(parsed);
    let file_path = payload
        .get("file_path")
        .and_then(|value| value.as_str())
        .map(|path| truncate_utf8(path, 180))
        .unwrap_or_else(|| "unknown file".to_string());

    format!("{} in {}", action(payload), file_path)
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
    use super::{build_history_tool_result, tool_retention, ToolRetention};
    use serde_json::json;

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
