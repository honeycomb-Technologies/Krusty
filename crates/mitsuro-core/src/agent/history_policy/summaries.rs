use serde_json::{json, Value};

use super::{
    diff_preview, tool_payload, truncate_array_strings, truncate_utf8, truncate_utf8_head_tail,
    unique_file_count, MAX_BASH_OUTPUT_CHARS, MAX_LIST_ITEMS, MAX_MATCH_ITEMS, MAX_PREVIEW_CHARS,
};

const MAX_BASH_ERROR_SUMMARY_OUTPUT_CHARS: usize = 700;
const MAX_AGENT_FINDINGS_HEAD_CHARS: usize = 1_900;
const MAX_AGENT_FINDINGS_TAIL_CHARS: usize = 900;
const MAX_AGENT_DETAIL_CHARS: usize = 600;
const MAX_AGENT_PATH_CHARS: usize = 300;

pub(super) fn summarize_tool_result(
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
        "agent" => summarize_agent(parsed, is_error),
        "explore" => {
            summarize_structured_or_large_text_tool(parsed, "explore", raw_output, is_error)
        }
        "build" => summarize_structured_or_large_text_tool(parsed, "build", raw_output, is_error),
        _ => summarize_generic(parsed, raw_output, is_error),
    }
}

pub(super) fn summarized_result(tool_name: &str, parsed: &Value, raw_output: &str) -> Value {
    match tool_name {
        "grep" => summarize_grep_result(parsed),
        "glob" => summarize_glob_result(parsed),
        "list" => summarize_list_result(parsed),
        "bash" => summarize_bash_result(parsed),
        "write" => summarize_write_result(parsed),
        "edit" => summarize_edit_result(parsed),
        "multiedit" => summarize_multiedit_result(parsed),
        "apply_patch" => summarize_apply_patch_result(parsed),
        "agent" => summarize_agent_result(parsed),
        "explore" | "build" => json!({
            "preview": truncate_utf8(raw_output, MAX_PREVIEW_CHARS),
        }),
        _ => json!({
            "preview": truncate_utf8(raw_output, MAX_PREVIEW_CHARS),
        }),
    }
}

fn summarize_agent(parsed: &Value, is_error: bool) -> String {
    if is_error {
        return summarize_message_tool(parsed, true);
    }

    let payload = tool_payload(parsed);
    let agent_type = payload
        .get("agent_type")
        .and_then(Value::as_str)
        .unwrap_or("delegated");
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| payload.get("outcome").and_then(Value::as_str))
        .unwrap_or("completed");
    let run_id = payload
        .get("delegated_run_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!(" (run {value})"))
        .unwrap_or_default();
    let detail = payload
        .get("investigation_summary")
        .or_else(|| payload.get("findings"))
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .map(|value| truncate_utf8(value, MAX_AGENT_DETAIL_CHARS))
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!(": {value}"))
        .unwrap_or_default();

    format!("{agent_type} agent {status}{run_id}{detail}")
}

fn summarize_agent_result(parsed: &Value) -> Value {
    let payload = tool_payload(parsed);
    let paths = payload
        .get("paths_examined")
        .or_else(|| payload.get("files_examined"))
        .and_then(Value::as_array)
        .map(|items| truncate_array_strings(items, MAX_LIST_ITEMS, MAX_AGENT_PATH_CHARS))
        .unwrap_or_default();
    let errors = payload
        .get("errors")
        .and_then(Value::as_array)
        .map(|items| truncate_array_strings(items, MAX_MATCH_ITEMS, MAX_AGENT_DETAIL_CHARS))
        .unwrap_or_default();
    let background_processes = payload
        .get("background_processes")
        .and_then(Value::as_array)
        .map(|processes| {
            processes
                .iter()
                .take(MAX_MATCH_ITEMS)
                .map(|process| {
                    json!({
                        "process_id": process.get("process_id").and_then(Value::as_str),
                        "status": process.get("status").and_then(Value::as_str),
                        "command": process
                            .get("command")
                            .and_then(Value::as_str)
                            .map(|value| truncate_utf8(value, MAX_AGENT_DETAIL_CHARS)),
                        "working_dir": process.get("working_dir").and_then(Value::as_str),
                        "endpoint_hints": process
                            .get("endpoint_hints")
                            .and_then(Value::as_array)
                            .map(|items| truncate_array_strings(items, 6, 200))
                            .unwrap_or_default(),
                        "reused_existing": process
                            .get("reused_existing")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "status": payload.get("status").and_then(Value::as_str),
        "delegated_run_id": payload.get("delegated_run_id").and_then(Value::as_str),
        "agent_type": payload.get("agent_type").and_then(Value::as_str),
        "name": payload.get("name").and_then(Value::as_str),
        "message": payload
            .get("message")
            .and_then(Value::as_str)
            .map(|value| truncate_utf8(value, MAX_AGENT_DETAIL_CHARS)),
        "investigation_summary": payload
            .get("investigation_summary")
            .and_then(Value::as_str)
            .map(|value| truncate_utf8(value, MAX_AGENT_DETAIL_CHARS)),
        "findings": payload
            .get("findings")
            .and_then(Value::as_str)
            .map(|value| truncate_utf8_head_tail(
                value,
                MAX_AGENT_FINDINGS_HEAD_CHARS,
                MAX_AGENT_FINDINGS_TAIL_CHARS,
            )),
        "outcome": payload.get("outcome").and_then(Value::as_str),
        "outcome_reason": payload.get("outcome_reason").and_then(Value::as_str),
        "confidence": payload.get("confidence").and_then(Value::as_str),
        "paths_examined": paths,
        "paths_examined_count": payload
            .get("paths_examined_count")
            .or_else(|| payload.get("files_examined_count"))
            .and_then(Value::as_u64),
        "agent_count": payload
            .get("agent_count")
            .or_else(|| payload.get("builder_count"))
            .and_then(Value::as_u64),
        "successful_agents": payload.get("successful_agents").and_then(Value::as_u64),
        "failed_agents": payload.get("failed_agents").and_then(Value::as_u64),
        "files_modified": payload.get("files_modified").and_then(Value::as_u64),
        "lines_added": payload.get("lines_added").and_then(Value::as_u64),
        "lines_removed": payload.get("lines_removed").and_then(Value::as_u64),
        "errors": errors,
        "coverage_gap_notice": payload
            .get("coverage_gap_notice")
            .and_then(Value::as_str)
            .map(|value| truncate_utf8(value, MAX_AGENT_DETAIL_CHARS)),
        "background_processes": background_processes,
    })
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
        if let Some(output_preview) =
            bash_output_preview(parsed, MAX_BASH_ERROR_SUMMARY_OUTPUT_CHARS)
        {
            return format!(
                "bash failed (exit {}): {}; output: {}",
                exit_code, message, output_preview
            );
        }
        return format!("bash failed (exit {}): {}", exit_code, message);
    }

    let payload = tool_payload(parsed);
    if let Some(process_id) = payload.get("process_id").and_then(Value::as_str) {
        let status = payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let endpoint_suffix = payload
            .get("endpoint_hints")
            .and_then(Value::as_array)
            .map(|endpoints| {
                endpoints
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|endpoints| !endpoints.is_empty())
            .map(|endpoints| format!(" at {endpoints}"))
            .unwrap_or_default();
        return format!(
            "background process {} (id {}){}; it remains available through processes status/control while the harness is running",
            status, process_id, endpoint_suffix
        );
    }

    format!("bash completed successfully (exit {})", exit_code)
}

fn bash_output_preview(parsed: &Value, limit: usize) -> Option<String> {
    tool_payload(parsed)
        .get("output")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|output| !output.is_empty())
        .map(|output| truncate_utf8(output, limit))
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
    let payload = tool_payload(parsed);
    let output_preview = payload
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
        "message": payload.get("message").and_then(Value::as_str),
        "process_id": payload.get("process_id").and_then(Value::as_str),
        "status": payload.get("status").and_then(Value::as_str),
        "endpoint_hints": payload.get("endpoint_hints").cloned().unwrap_or_else(|| json!([])),
        "reused_existing": payload.get("reused_existing").and_then(Value::as_bool).unwrap_or(false),
        "next_action": payload.get("next_action").and_then(Value::as_str),
        "process_error": payload.get("error").and_then(Value::as_str),
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
