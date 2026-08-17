//! One bounded parsing boundary for tool arguments and output.
//!
//! Tool results reach the TUI in two layers:
//! 1. **History wrapper** from `build_history_tool_result`:
//!    `{ tool, retention, summary, is_error, result }`
//! 2. **Inner payload** — either the raw tool envelope `{ ok, data, diff }`
//!    (read retain_full) or a summarized shape (`output_preview`, `diff_preview`).
//!
//! We peel both layers by tool family so Code / Diff / Terminal panels get the
//! primary body instead of a flattened metadata dump.

use serde_json::Value;

use crate::tui_v2::model::{
    artifact::{
        ArtifactContent, ArtifactField, ArtifactModel, ArtifactProvenance, ArtifactWarning,
        BoundedText, RetentionLevel,
    },
    conversation::ToolArguments,
};

pub const LIVE_ARTIFACT_BYTES: usize = 128 * 1024;
pub const HISTORICAL_ARTIFACT_BYTES: usize = 8 * 1024;
const MAX_ARGUMENT_FIELDS: usize = 16;
const MAX_STRUCTURED_FIELDS: usize = 64;
const MAX_FIELD_VALUE_BYTES: usize = 1_024;
/// Large presentation payloads (file content, diffs, patches) keep a bigger budget.
const MAX_PAYLOAD_FIELD_BYTES: usize = 64 * 1024;

pub fn parse_tool_arguments(value: &Value) -> ToolArguments {
    let mut fields = Vec::new();
    let mut redacted_fields = 0;
    flatten_value(
        value,
        "",
        0,
        MAX_ARGUMENT_FIELDS,
        &mut fields,
        &mut redacted_fields,
    );
    ToolArguments {
        fields,
        redacted_fields,
    }
}

pub fn parse_tool_output(name: &str, output: &str, historical: bool) -> ArtifactModel {
    let limit = if historical {
        HISTORICAL_ARTIFACT_BYTES
    } else {
        LIVE_ARTIFACT_BYTES
    };
    let sanitized = sanitize_terminal_text(output);
    let trimmed = sanitized.trim();

    if trimmed.is_empty() {
        return ArtifactModel::default();
    }

    let looks_structured = matches!(trimmed.as_bytes().first(), Some(b'{') | Some(b'['));
    if looks_structured {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            return artifact_from_tool_value(name, &value, limit, historical);
        }
    }
    ArtifactModel {
        content: ArtifactContent::Text(bound_text(&sanitized, limit)),
        warning: looks_structured.then(|| ArtifactWarning {
            message: format!("{name} returned malformed structured output; showing safe text"),
        }),
        retention: retention(historical),
        provenance: ArtifactProvenance::default(),
    }
}

/// Unwrap Mitsuro history + tool envelopes into panel-ready artifact content.
pub fn artifact_from_tool_value(
    name: &str,
    value: &Value,
    limit: usize,
    historical: bool,
) -> ArtifactModel {
    if let Some(text) = value.as_str() {
        return text_artifact(text, limit, historical, None);
    }

    // Layer 1: model-facing history wrapper from build_history_tool_result.
    if let Some(inner) = peel_history_wrapper(value) {
        let mut artifact = extract_family_payload(name, inner, limit, historical);
        // Carry summary only when we still have no usable body.
        if matches!(
            artifact.content,
            ArtifactContent::Empty | ArtifactContent::Fields(_)
        ) {
            if let Some(summary) = value.get("summary").and_then(Value::as_str) {
                if !summary.is_empty()
                    && !matches!(artifact.content, ArtifactContent::Text(ref t) if !t.text.is_empty())
                {
                    // Prefer summary as last-resort text for bash/edit when previews empty.
                    if matches!(artifact.content, ArtifactContent::Empty) {
                        return text_artifact(summary, limit, historical, artifact.warning);
                    }
                }
            }
        }
        if artifact.warning.is_none() {
            if let Some(summary) = value
                .get("summary")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                // Don't attach summary as warning when we already have a full body.
                if matches!(artifact.content, ArtifactContent::Fields(_)) {
                    artifact.warning = Some(ArtifactWarning {
                        message: summary.to_owned(),
                    });
                }
            }
        }
        if value
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(err) = string_at(inner, &["error", "message", "process_error"]) {
                if matches!(artifact.content, ArtifactContent::Empty) {
                    return text_artifact(err, limit, historical, artifact.warning);
                }
            }
        }
        return artifact;
    }

    extract_family_payload(name, value, limit, historical)
}

/// Detect `{ tool?, retention?, summary?, is_error?, result }` history shape.
fn peel_history_wrapper(value: &Value) -> Option<&Value> {
    let result = value.get("result")?;
    let looks_like_history = value.get("retention").is_some()
        || value.get("tool").is_some()
        || (value.get("summary").is_some() && value.get("is_error").is_some());
    looks_like_history.then_some(result)
}

/// Extract the primary panel body from a raw or summarized tool payload.
fn extract_family_payload(
    name: &str,
    value: &Value,
    limit: usize,
    historical: bool,
) -> ArtifactModel {
    if let Some(text) = value.as_str() {
        return text_artifact(text, limit, historical, None);
    }

    let tool = name.to_ascii_lowercase();
    let warning = envelope_warning(value);
    let data = value.get("data");

    // Nested history is rare but cheap to peel again.
    if let Some(inner) = peel_history_wrapper(value) {
        return extract_family_payload(name, inner, limit, historical);
    }

    match tool.as_str() {
        "read" | "read_file" => {
            if let Some(content) = first_string(value, data, &["content"]) {
                let mut artifact = text_artifact(content, limit, historical, warning);
                artifact.provenance = read_provenance(value, data);
                return artifact;
            }
        }
        "bash" | "shell" | "terminal" => {
            // Full stream, then history compact preview.
            if let Some(out) = first_string(value, data, &["output", "output_preview"]) {
                if !out.trim().is_empty() {
                    return text_artifact(out, limit, historical, warning);
                }
            }
            if let Some(msg) = first_string(value, data, &["message", "error", "process_error"]) {
                return text_artifact(msg, limit, historical, warning);
            }
        }
        "edit" | "write" | "write_file" | "str_replace" | "multiedit" | "apply_patch" => {
            // Full diff (retain paths / raw envelope) or history diff_preview.
            if let Some(diff) = first_string(value, data, &["diff", "diff_preview", "patch"]) {
                if !diff.trim().is_empty() {
                    let mut artifact = text_artifact(diff, limit, historical, warning);
                    artifact.provenance = mutation_provenance(value, data);
                    return artifact;
                }
            }
            if matches!(tool.as_str(), "write" | "write_file") {
                if let Some(content) = first_string(value, data, &["content"]) {
                    let mut artifact = text_artifact(content, limit, historical, warning);
                    artifact.provenance = mutation_provenance(value, data);
                    return artifact;
                }
            }
            if let Some(msg) = first_string(value, data, &["message"]) {
                let mut artifact = text_artifact(msg, limit, historical, warning);
                artifact.provenance = mutation_provenance(value, data);
                return artifact;
            }
        }
        "web_fetch" | "fetch" => {
            if let Some(content) =
                first_string(value, data, &["content", "text", "body", "preview"])
            {
                return text_artifact(content, limit, historical, warning);
            }
        }
        "grep" | "glob" | "list" | "list_files" => {
            if let Some(text) = first_string(value, data, &["output", "content", "preview"]) {
                return text_artifact(text, limit, historical, warning);
            }
            // Fall through to field listing of matches when structured.
        }
        _ => {}
    }

    // Generic: string data, else flatten data / root (never the outer history keys).
    if let Some(text) = data.and_then(Value::as_str) {
        return text_artifact(text, limit, historical, warning);
    }

    let flatten_root = data.unwrap_or(value);
    let mut artifact = artifact_from_value(flatten_root, limit, historical);
    if artifact.warning.is_none() {
        artifact.warning = warning;
    }
    if let Some(message) = value
        .get("error")
        .and_then(|e| e.get("message").or(Some(e)))
        .and_then(Value::as_str)
    {
        if matches!(artifact.content, ArtifactContent::Empty) {
            return text_artifact(message, limit, historical, artifact.warning);
        }
        if let ArtifactContent::Fields(ref mut fields) = artifact.content {
            fields.insert(
                0,
                ArtifactField {
                    key: "error".to_owned(),
                    value: message.to_owned(),
                },
            );
        }
    }
    artifact
}

/// Prefer keys on `data`, then the root value.
fn first_string<'a>(root: &'a Value, data: Option<&'a Value>, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(text) = data.and_then(|d| d.get(*key)).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text);
            }
        }
        if let Some(text) = root.get(*key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn string_at<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text);
            }
        }
        if let Some(text) = value
            .get(*key)
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str)
        {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

pub fn artifact_from_value(value: &Value, limit: usize, historical: bool) -> ArtifactModel {
    if let Some(text) = value.as_str() {
        return text_artifact(text, limit, historical, None);
    }

    let mut fields = Vec::new();
    let mut redacted_fields = 0;
    flatten_value(
        value,
        "",
        0,
        MAX_STRUCTURED_FIELDS,
        &mut fields,
        &mut redacted_fields,
    );

    let warning = (fields.len() == MAX_STRUCTURED_FIELDS).then(|| ArtifactWarning {
        message: "Structured output was bounded for terminal presentation".to_owned(),
    });
    ArtifactModel {
        content: if fields.is_empty() {
            ArtifactContent::Empty
        } else {
            ArtifactContent::Fields(fields)
        },
        warning,
        retention: retention(historical),
        provenance: ArtifactProvenance::default(),
    }
}

fn text_artifact(
    text: &str,
    limit: usize,
    historical: bool,
    warning: Option<ArtifactWarning>,
) -> ArtifactModel {
    ArtifactModel {
        content: ArtifactContent::Text(bound_text(&sanitize_terminal_text(text), limit)),
        warning,
        retention: retention(historical),
        provenance: ArtifactProvenance::default(),
    }
}

fn read_provenance(root: &Value, data: Option<&Value>) -> ArtifactProvenance {
    let source = data.unwrap_or(root);
    ArtifactProvenance {
        path: first_string(root, data, &["file_path", "path"]).map(str::to_owned),
        start_line: u32_at(source, "start_line").or_else(|| u32_at(root, "start_line")),
        total_lines: u32_at(source, "total_lines").or_else(|| u32_at(root, "total_lines")),
        lines_returned: u32_at(source, "lines_returned").or_else(|| u32_at(root, "lines_returned")),
    }
}

fn mutation_provenance(root: &Value, data: Option<&Value>) -> ArtifactProvenance {
    ArtifactProvenance {
        path: first_string(root, data, &["file_path", "path"]).map(str::to_owned),
        start_line: None,
        total_lines: u32_at(data.unwrap_or(root), "line_count"),
        lines_returned: None,
    }
}

fn u32_at(value: &Value, key: &str) -> Option<u32> {
    value.get(key).and_then(|v| {
        v.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .or_else(|| v.as_i64().and_then(|n| u32::try_from(n).ok()))
    })
}

fn envelope_warning(value: &Value) -> Option<ArtifactWarning> {
    let warnings = value.get("warnings")?.as_array()?;
    let messages: Vec<&str> = warnings
        .iter()
        .filter_map(Value::as_str)
        .filter(|m| !m.is_empty())
        .collect();
    if messages.is_empty() {
        return None;
    }
    Some(ArtifactWarning {
        message: messages.join("; "),
    })
}

pub fn append_tool_delta(artifact: &mut ArtifactModel, delta: &str) {
    let text = match &mut artifact.content {
        ArtifactContent::Text(text) => text,
        ArtifactContent::Empty => {
            artifact.content = ArtifactContent::Text(BoundedText::default());
            let ArtifactContent::Text(text) = &mut artifact.content else {
                unreachable!();
            };
            text
        }
        _ => {
            artifact.warning = Some(ArtifactWarning {
                message: "Live output changed from structured data to text".to_owned(),
            });
            artifact.content = ArtifactContent::Text(BoundedText::default());
            let ArtifactContent::Text(text) = &mut artifact.content else {
                unreachable!();
            };
            text
        }
    };

    let previous_omitted = text.omitted_bytes;
    let mut combined = String::with_capacity(text.text.len() + delta.len());
    combined.push_str(&text.text);
    combined.push_str(delta);
    *text = bound_text(&sanitize_terminal_text(&combined), LIVE_ARTIFACT_BYTES);
    text.omitted_bytes = text.omitted_bytes.saturating_add(previous_omitted);
}

pub fn finalize_tool_output(artifact: &mut ArtifactModel, name: &str, output: &str) {
    let final_artifact = parse_tool_output(name, output, false);

    if is_shell_tool(name) {
        match (&artifact.content, &final_artifact.content) {
            (ArtifactContent::Text(current), ArtifactContent::Text(final_text))
                if !current.text.is_empty() =>
            {
                // Prefer the longer transcript; never drop streamed evidence for a short final.
                if current.text.contains(&final_text.text) || final_text.text.is_empty() {
                    if final_artifact.warning.is_some() {
                        artifact.warning = final_artifact.warning;
                    }
                    return;
                }
                if final_text.text.contains(&current.text) {
                    *artifact = final_artifact;
                    return;
                }
                // Distinct streams: keep both.
                append_tool_delta(artifact, "\n");
                append_tool_delta(artifact, &final_text.text);
                if final_artifact.warning.is_some() {
                    artifact.warning = final_artifact.warning;
                }
                return;
            }
            (ArtifactContent::Text(current), _) if !current.text.is_empty() => {
                // Final did not yield panel-ready text — keep the live stream.
                if final_artifact.warning.is_some() {
                    artifact.warning = final_artifact.warning;
                }
                return;
            }
            _ => {}
        }
    }

    *artifact = final_artifact;
}

fn is_shell_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "bash" | "shell" | "terminal"
    )
}

pub fn bound_text(text: &str, max_bytes: usize) -> BoundedText {
    if text.len() <= max_bytes {
        return BoundedText {
            text: text.to_owned(),
            omitted_bytes: 0,
        };
    }

    let marker = "\n… output omitted …\n";
    let payload_budget = max_bytes.saturating_sub(marker.len());
    let head_budget = payload_budget / 3;
    let tail_budget = payload_budget.saturating_sub(head_budget);
    let head_end = floor_char_boundary(text, head_budget);
    let tail_start = ceil_char_boundary(text, text.len().saturating_sub(tail_budget));
    let mut bounded = String::with_capacity(max_bytes);
    bounded.push_str(&text[..head_end]);
    bounded.push_str(marker);
    bounded.push_str(&text[tail_start..]);

    BoundedText {
        omitted_bytes: text
            .len()
            .saturating_sub(head_end)
            .saturating_sub(text.len().saturating_sub(tail_start)),
        text: bounded,
    }
}

/// Removes terminal control sequences before they can enter measurement or
/// rendering. Newlines and tabs remain semantic text; carriage returns replace
/// the current progress line instead of leaking cursor motion.
pub fn sanitize_terminal_text(input: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut state = State::Text;
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match state {
            State::Text => match character {
                '\u{1b}' => state = State::Escape,
                '\r' => {
                    if let Some(line_start) = output.rfind('\n') {
                        output.truncate(line_start + 1);
                    } else {
                        output.clear();
                    }
                }
                '\n' | '\t' => output.push(character),
                value if value.is_control() => {}
                value => output.push(value),
            },
            State::Escape => {
                state = match character {
                    '[' => State::Csi,
                    ']' => State::Osc,
                    _ => State::Text,
                };
            }
            State::Csi => {
                if ('@'..='~').contains(&character) {
                    state = State::Text;
                }
            }
            State::Osc => match character {
                '\u{7}' => state = State::Text,
                '\u{1b}' => state = State::OscEscape,
                _ => {}
            },
            State::OscEscape => {
                state = if character == '\\' {
                    State::Text
                } else {
                    State::Osc
                };
            }
        }
    }
    output
}

fn flatten_value(
    value: &Value,
    path: &str,
    depth: usize,
    limit: usize,
    output: &mut Vec<ArtifactField>,
    redacted_fields: &mut usize,
) {
    if output.len() >= limit {
        return;
    }

    // Multiedit nests `{ edits: [ { old_string, new_string } ] }` — allow one extra level.
    let max_depth = if path.starts_with("edits") { 3 } else { 2 };

    match value {
        Value::Object(map) if depth < max_depth => {
            for (key, value) in map {
                if output.len() >= limit {
                    break;
                }
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if is_sensitive_key(key) {
                    *redacted_fields += 1;
                    output.push(ArtifactField {
                        key: bounded_scalar(&path, 96),
                        value: "[redacted]".to_owned(),
                    });
                } else {
                    flatten_value(value, &path, depth + 1, limit, output, redacted_fields);
                }
            }
        }
        Value::Object(_) => output.push(ArtifactField {
            key: bounded_scalar(if path.is_empty() { "value" } else { path }, 96),
            value: "{structured value omitted}".to_owned(),
        }),
        Value::Array(items) if depth < max_depth => {
            for (index, value) in items.iter().enumerate() {
                if output.len() >= limit {
                    break;
                }
                let path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}[{index}]")
                };
                flatten_value(value, &path, depth + 1, limit, output, redacted_fields);
            }
        }
        Value::Array(_) => output.push(ArtifactField {
            key: bounded_scalar(if path.is_empty() { "value" } else { path }, 96),
            value: "[structured value omitted]".to_owned(),
        }),
        _ => output.push(ArtifactField {
            key: bounded_scalar(if path.is_empty() { "value" } else { path }, 96),
            value: bounded_scalar(&scalar_text(value), max_bytes_for_path(path)),
        }),
    }
}

fn max_bytes_for_path(path: &str) -> usize {
    let leaf = path
        .rsplit(['.', '[', ']'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_ascii_lowercase();
    match leaf.as_str() {
        "content" | "old_string" | "new_string" | "old_str" | "new_str" | "patch" | "output"
        | "command" | "diff" | "text" | "body" => MAX_PAYLOAD_FIELD_BYTES,
        _ => MAX_FIELD_VALUE_BYTES,
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(value) => sanitize_terminal_text(value),
        Value::Null => "null".to_owned(),
        value => value.to_string(),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "cookie",
        "credential",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn bounded_scalar(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        value.to_owned()
    } else {
        let end = floor_char_boundary(value, max_bytes.saturating_sub(1));
        format!("{}…", &value[..end])
    }
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

const fn retention(historical: bool) -> RetentionLevel {
    if historical {
        RetentionLevel::Preview
    } else {
        RetentionLevel::Full
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn arguments_are_bounded_and_secrets_are_redacted() {
        let arguments = parse_tool_arguments(&json!({
            "command": "cargo test",
            "api_key": "do-not-render",
            "nested": {"password": "also-secret"}
        }));

        assert_eq!(arguments.redacted_fields, 2);
        assert!(arguments
            .fields
            .iter()
            .all(|field| !field.value.contains("secret")));
    }

    #[test]
    fn depth_limit_never_serializes_nested_secret_json() {
        let arguments = parse_tool_arguments(&json!({
            "outer": {"inner": {"token": "do-not-render"}}
        }));

        assert!(!format!("{arguments:?}").contains("do-not-render"));
        assert!(arguments
            .fields
            .iter()
            .any(|field| field.value.contains("omitted")));
    }

    #[test]
    fn terminal_controls_and_carriage_return_progress_are_normalized() {
        assert_eq!(
            sanitize_terminal_text("one\rprogress 10%\rprogress 20%\n\u{1b}[31mred\u{1b}[0m"),
            "progress 20%\nred"
        );
        assert_eq!(
            sanitize_terminal_text("\u{1b}]8;;https://example.com\u{7}link\u{1b}]8;;\u{7}"),
            "link"
        );
    }

    #[test]
    fn carriage_return_rewrites_progress_across_stream_deltas() {
        let mut artifact = ArtifactModel::default();
        append_tool_delta(&mut artifact, "progress 10%");
        append_tool_delta(&mut artifact, "\rprogress 20%");

        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text) if text.text == "progress 20%"
        ));
    }

    #[test]
    fn terse_shell_completion_keeps_streamed_evidence() {
        let mut artifact = ArtifactModel::default();
        append_tool_delta(&mut artifact, "compiling crate\n");
        finalize_tool_output(&mut artifact, "bash", "finished");

        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text)
                if text.text.contains("compiling crate") && text.text.contains("finished")
        ));
    }

    #[test]
    fn hostile_output_remains_utf8_safe_and_bounded() {
        let input = format!("head\n{}\ntail", "蟹".repeat(LIVE_ARTIFACT_BYTES));
        let bounded = bound_text(&input, 4096);

        assert!(bounded.truncated());
        assert!(bounded.text.len() <= 4096);
        assert!(bounded.text.starts_with("head"));
        assert!(bounded.text.ends_with("tail"));
    }

    #[test]
    fn malformed_structured_output_becomes_typed_text_fallback() {
        let artifact = parse_tool_output("read", r#"{"path":"broken""#, false);

        assert!(matches!(artifact.content, ArtifactContent::Text(_)));
        assert!(artifact.warning.is_some());
    }

    #[test]
    fn read_envelope_extracts_file_content_text() {
        let output = json!({
            "ok": true,
            "data": {
                "content": "fn main() {}\n",
                "total_lines": 1,
                "lines_returned": 1,
                "start_line": 1
            }
        })
        .to_string();
        let artifact = parse_tool_output("read", &output, false);
        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text) if text.text.contains("fn main")
        ));
    }

    #[test]
    fn read_history_wrapper_extracts_content_not_metadata_fields() {
        // Shape actually emitted on LoopEvent::ToolResult for read (retain_full).
        let output = json!({
            "tool": "read",
            "retention": "retain_full",
            "summary": "read returned 50 lines starting at line 100 (file has 511 total lines)",
            "is_error": false,
            "result": {
                "ok": true,
                "data": {
                    "content": "const GRID = 20;\nfunction tick() {}\n",
                    "total_lines": 511,
                    "lines_returned": 50,
                    "start_line": 100
                }
            }
        })
        .to_string();
        let artifact = parse_tool_output("read", &output, false);
        match artifact.content {
            ArtifactContent::Text(text) => {
                assert!(text.text.contains("const GRID"));
                assert!(!text.text.contains("retain_full"));
                assert!(!text.text.contains("is_error"));
            }
            other => panic!("expected Text body, got {other:?}"),
        }
        assert_eq!(artifact.provenance.start_line, Some(100));
        assert_eq!(artifact.provenance.total_lines, Some(511));
        assert_eq!(artifact.provenance.lines_returned, Some(50));
    }

    #[test]
    fn bash_envelope_extracts_output_text() {
        let output = json!({
            "ok": true,
            "data": { "output": "compiled ok\n2 tests passed\n" }
        })
        .to_string();
        let artifact = parse_tool_output("bash", &output, false);
        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text) if text.text.contains("2 tests passed")
        ));
    }

    #[test]
    fn bash_history_wrapper_uses_output_preview() {
        let output = json!({
            "tool": "bash",
            "retention": "drop_after_compaction",
            "summary": "bash completed successfully (exit 0)",
            "is_error": false,
            "result": {
                "exit_code": 0,
                "killed": false,
                "error": null,
                "message": null,
                "output_preview": "511 tests/snake-tetris/game.js\nok\n",
                "endpoint_hints": [],
                "reused_existing": false
            }
        })
        .to_string();
        let artifact = parse_tool_output("bash", &output, false);
        match artifact.content {
            ArtifactContent::Text(text) => {
                assert!(text.text.contains("511 tests"));
                assert!(!text.text.contains("drop_after_compaction"));
                assert!(!text.text.contains("endpoint_hints"));
            }
            other => panic!("expected Text body, got {other:?}"),
        }
    }

    #[test]
    fn bash_finalize_keeps_stream_when_final_is_field_noise() {
        let mut artifact = ArtifactModel::default();
        append_tool_delta(&mut artifact, "line one\nline two\n");
        // Malformed / unexpected structured shape without extractable output —
        // still must not wipe the stream when parse yields empty-ish fields.
        finalize_tool_output(
            &mut artifact,
            "bash",
            &json!({ "ok": true, "data": { "status": "done" } }).to_string(),
        );
        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text) if text.text.contains("line one")
        ));
    }

    #[test]
    fn bash_finalize_prefers_longer_envelope_output() {
        let mut artifact = ArtifactModel::default();
        append_tool_delta(&mut artifact, "partial\n");
        finalize_tool_output(
            &mut artifact,
            "bash",
            &json!({
                "ok": true,
                "data": { "output": "partial\nfull output\n" }
            })
            .to_string(),
        );
        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text)
                if text.text.contains("full output") && text.text.contains("partial")
        ));
    }

    #[test]
    fn bash_finalize_keeps_stream_over_short_history_preview() {
        let mut artifact = ArtifactModel::default();
        append_tool_delta(&mut artifact, "compiling…\nrunning 12 tests\nok\n");
        finalize_tool_output(
            &mut artifact,
            "bash",
            &json!({
                "tool": "bash",
                "retention": "drop_after_compaction",
                "summary": "bash completed successfully (exit 0)",
                "is_error": false,
                "result": { "exit_code": 0, "output_preview": "ok\n" }
            })
            .to_string(),
        );
        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text)
                if text.text.contains("running 12 tests") && text.text.contains("ok")
        ));
    }

    #[test]
    fn edit_envelope_extracts_top_level_diff() {
        let diff = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let output = json!({
            "ok": true,
            "data": { "file_path": "src/main.rs" },
            "diff": diff
        })
        .to_string();
        let artifact = parse_tool_output("edit", &output, false);
        assert!(matches!(
            artifact.content,
            ArtifactContent::Text(ref text) if text.text.contains("-old") && text.text.contains("+new")
        ));
    }

    #[test]
    fn write_history_wrapper_extracts_diff_preview() {
        let output = json!({
            "tool": "write",
            "retention": "summarize_after_turn",
            "summary": "Created new file (3 lines)",
            "is_error": false,
            "changed": true,
            "result": {
                "message": "Created new file (3 lines)",
                "file_path": "tests/snake-tetris/README.md",
                "bytes_written": 120,
                "line_count": 3,
                "diff_preview": "--- tests/snake-tetris/README.md\n+++ tests/snake-tetris/README.md\n@@\n+# Snake Tetris\n"
            }
        })
        .to_string();
        let artifact = parse_tool_output("write", &output, false);
        match artifact.content {
            ArtifactContent::Text(text) => {
                assert!(text.text.contains("# Snake Tetris") || text.text.contains("+++"));
                assert!(!text.text.contains("summarize_after_turn"));
            }
            other => panic!("expected Text body, got {other:?}"),
        }
    }

    #[test]
    fn multiedit_arguments_preserve_nested_edit_strings() {
        let arguments = parse_tool_arguments(&json!({
            "file_path": "src/lib.rs",
            "edits": [
                { "old_string": "aaa", "new_string": "bbb" },
                { "old_string": "ccc", "new_string": "ddd" }
            ]
        }));
        assert!(arguments
            .fields
            .iter()
            .any(|f| f.key.contains("old_string") && f.value == "aaa"));
        assert!(arguments
            .fields
            .iter()
            .any(|f| f.key.contains("new_string") && f.value == "ddd"));
    }

    #[test]
    fn payload_argument_fields_are_not_capped_at_1kb() {
        let big = "x".repeat(4_000);
        let arguments = parse_tool_arguments(&json!({
            "file_path": "a.rs",
            "content": big
        }));
        let content = arguments
            .fields
            .iter()
            .find(|f| f.key == "content")
            .expect("content field");
        assert!(content.value.len() >= 4_000);
        assert!(!content.value.ends_with('…') || content.value.len() > 1_024);
    }
}
