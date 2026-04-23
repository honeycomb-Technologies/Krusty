use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::streaming::StreamPart;
use serde_json::Value;

/// Apply a final stream-part transform after parser mapping.
///
/// This is currently used for provider families that need sanitized tool IDs so
/// tool call start/delta/complete events stay aligned.
pub fn apply_stream_part_transform(
    part: StreamPart,
    _provider_id: ProviderId,
    _api_format: ApiFormat,
    model_id: &str,
) -> StreamPart {
    if !requires_tool_call_id_sanitization(model_id) {
        return part;
    }

    match part {
        StreamPart::ToolCallStart { id, name } => StreamPart::ToolCallStart {
            id: sanitize_tool_call_id(&id),
            name,
        },
        StreamPart::ToolCallDelta { id, delta } => StreamPart::ToolCallDelta {
            id: sanitize_tool_call_id(&id),
            delta,
        },
        StreamPart::ToolCallComplete { mut tool_call } => {
            tool_call.id = sanitize_tool_call_id(&tool_call.id);
            StreamPart::ToolCallComplete { tool_call }
        }
        other => other,
    }
}

pub(super) fn requires_tool_call_id_sanitization(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    id.contains("mistral")
        || id.contains("deepseek")
        || id.contains("glm")
        || id.contains("minimax")
}

pub(super) fn sanitize_tool_call_id(id: &str) -> String {
    let normalized: String = id
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .chars()
        .take(9)
        .collect();

    let padding_len = 9_usize.saturating_sub(normalized.chars().count());
    let padding = std::iter::repeat_n('0', padding_len);
    normalized.chars().chain(padding).collect()
}

pub(super) fn sanitize_tool_call_ids_in_value(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                sanitize_tool_call_ids_in_value(item);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if matches!(
                    key.as_str(),
                    "toolCallId" | "tool_call_id" | "tool_use_id" | "call_id"
                ) {
                    if let Some(id) = child.as_str() {
                        *child = Value::String(sanitize_tool_call_id(id));
                        continue;
                    }
                }

                sanitize_tool_call_ids_in_value(child);
            }
        }
        _ => {}
    }
}
