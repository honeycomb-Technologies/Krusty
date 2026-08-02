use anyhow::Result;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

use super::super::super::core::AiClient;
use crate::ai::retry::safe_provider_event_error;

impl AiClient {
    pub(super) async fn collect_codex_websocket_response<S>(
        &self,
        stream: &mut S,
        model: &str,
    ) -> Result<Value>
    where
        S: futures::Stream<
                Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
            > + Unpin,
    {
        let mut text_content = String::new();
        let mut tool_order: Vec<String> = Vec::new();
        let mut pending_tools: HashMap<String, (String, String)> = HashMap::new();
        let mut item_to_call_id: HashMap<String, String> = HashMap::new();
        let mut saw_completion = false;
        let mut finish_reason = "end_turn";
        let mut final_usage: Option<Value> = None;

        while let Some(msg) = stream.next().await {
            let message = match msg {
                Ok(message) => message,
                Err(error) => {
                    let detail = error.to_string();
                    return Err(anyhow::Error::msg(safe_provider_event_error(
                        "Sub-agent Codex websocket stream error",
                        None,
                        Some("server_error"),
                        Some(&detail),
                    )));
                }
            };
            let payload = match message {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Close(frame) => {
                    if !saw_completion {
                        let (code, reason) = frame
                            .as_ref()
                            .map(|f| (f.code.to_string(), f.reason.to_string()))
                            .unzip();
                        return Err(anyhow::Error::msg(safe_provider_event_error(
                            "Sub-agent Codex websocket closed before completion",
                            code.as_deref(),
                            Some("server_error"),
                            reason.as_deref(),
                        )));
                    }
                    break;
                }
            };

            let Ok(json) = serde_json::from_str::<Value>(&payload) else {
                continue;
            };

            let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match event_type {
                "error" | "response.failed" => {
                    let message = Self::codex_ws_error_message(&json).unwrap_or_else(|| {
                        "Codex websocket API error [metadata=unavailable]".to_string()
                    });
                    return Err(anyhow::Error::msg(format!("Sub-agent {message}")));
                }
                "response.output_text.delta" => {
                    if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                        text_content.push_str(delta);
                    }
                }
                "response.output_item.added" | "response.output_item.done" => {
                    if let Some(item) = json.get("item") {
                        if item.get("type").and_then(|t| t.as_str()) != Some("function_call") {
                            continue;
                        }
                        let call_id = item
                            .get("call_id")
                            .and_then(|i| i.as_str())
                            .or_else(|| item.get("id").and_then(|i| i.as_str()))
                            .unwrap_or("")
                            .to_string();
                        if call_id.is_empty() {
                            continue;
                        }
                        if let Some(item_id) = item
                            .get("id")
                            .and_then(|i| i.as_str())
                            .filter(|id| !id.is_empty())
                        {
                            item_to_call_id.insert(item_id.to_string(), call_id.clone());
                        }

                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !pending_tools.contains_key(&call_id) {
                            tool_order.push(call_id.clone());
                            pending_tools.insert(call_id.clone(), (name.clone(), String::new()));
                            debug!(
                                tool_name_bytes = name.len(),
                                tool_call_id_bytes = call_id.len(),
                                "Sub-agent Codex tool call started"
                            );
                        }

                        if let Some(arguments) = item.get("arguments").and_then(|a| a.as_str()) {
                            if let Some((_, args_buf)) = pending_tools.get_mut(&call_id) {
                                if args_buf.is_empty() {
                                    args_buf.push_str(arguments);
                                }
                            }
                        }
                    }
                }
                "response.function_call_arguments.delta" => {
                    if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                        if let Some(call_id) =
                            resolve_codex_tool_call_id(&json, &item_to_call_id, &pending_tools)
                        {
                            if let Some((_, args_buf)) = pending_tools.get_mut(&call_id) {
                                args_buf.push_str(delta);
                            }
                        }
                    }
                }
                "response.function_call_arguments.done" => {
                    if let Some(call_id) =
                        resolve_codex_tool_call_id(&json, &item_to_call_id, &pending_tools)
                    {
                        if let Some(arguments) = json.get("arguments").and_then(|a| a.as_str()) {
                            if let Some((_, args_buf)) = pending_tools.get_mut(&call_id) {
                                if args_buf.is_empty() {
                                    args_buf.push_str(arguments);
                                }
                            }
                        }
                    }
                }
                "response.usage" => {
                    let usage_obj = json.get("usage").unwrap_or(&json);
                    final_usage = Some(usage_obj.clone());
                    let input_tokens = usage_obj
                        .get("input_tokens")
                        .or_else(|| usage_obj.get("input"))
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    let output_tokens = usage_obj
                        .get("output_tokens")
                        .or_else(|| usage_obj.get("output"))
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    if input_tokens > 0 || output_tokens > 0 {
                        debug!(
                            "Sub-agent Codex usage: input={}, output={}",
                            input_tokens, output_tokens
                        );
                    }
                }
                "response.done" | "response.completed" => {
                    saw_completion = true;
                    if let Some(usage) = json
                        .get("response")
                        .and_then(|response| response.get("usage"))
                    {
                        final_usage = Some(usage.clone());
                    }
                    if let Some(response) = json.get("response") {
                        if response.get("status").and_then(|s| s.as_str()) == Some("incomplete") {
                            let reason = response
                                .get("incomplete_details")
                                .and_then(|d| d.get("reason"))
                                .and_then(|r| r.as_str())
                                .unwrap_or("incomplete");
                            if matches!(reason, "max_output_tokens" | "max_tokens" | "length") {
                                finish_reason = "max_tokens";
                            }
                        }
                    }
                    break;
                }
                _ => {}
            }
        }

        let mut content: Vec<Value> = vec![];
        if !text_content.is_empty() {
            content.push(serde_json::json!({
                "type": "text",
                "text": text_content
            }));
        }

        let mut has_tool_calls = false;
        for call_id in tool_order {
            if let Some((name, args_json)) = pending_tools.remove(&call_id) {
                let input = if args_json.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str::<Value>(&args_json)
                        .unwrap_or_else(|_| serde_json::json!({ "raw": args_json }))
                };
                content.push(serde_json::json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input
                }));
                has_tool_calls = true;
            }
        }
        for (call_id, (name, args_json)) in pending_tools {
            let input = if args_json.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str::<Value>(&args_json)
                    .unwrap_or_else(|_| serde_json::json!({ "raw": args_json }))
            };
            content.push(serde_json::json!({
                "type": "tool_use",
                "id": call_id,
                "name": name,
                "input": input
            }));
            has_tool_calls = true;
        }

        if has_tool_calls {
            finish_reason = "tool_use";
        }

        if !saw_completion {
            return Err(anyhow::anyhow!(
                "Sub-agent Codex websocket ended before response completion (websocket-only mode)"
            ));
        }

        let usage = final_usage.unwrap_or(Value::Null);
        Ok(serde_json::json!({
            "content": content,
            "stop_reason": finish_reason,
            "model": model,
            "usage": usage
        }))
    }
}

fn resolve_codex_tool_call_id(
    json: &Value,
    item_to_call_id: &HashMap<String, String>,
    pending_tools: &HashMap<String, (String, String)>,
) -> Option<String> {
    if let Some(call_id) = json
        .get("call_id")
        .and_then(|i| i.as_str())
        .filter(|id| !id.is_empty())
    {
        return Some(call_id.to_string());
    }

    if let Some(item_id) = json
        .get("item_id")
        .and_then(|i| i.as_str())
        .filter(|id| !id.is_empty())
    {
        if let Some(call_id) = item_to_call_id.get(item_id) {
            return Some(call_id.clone());
        }
    }

    if pending_tools.len() == 1 {
        return pending_tools.keys().next().cloned();
    }

    None
}
