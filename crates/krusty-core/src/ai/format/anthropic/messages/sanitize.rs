use std::collections::HashSet;

use serde_json::Value;
use tracing::debug;

pub(super) fn sanitize_tool_results(messages: &mut Vec<Value>) {
    let mut i = 0;

    while i < messages.len() {
        let role = messages[i]["role"].as_str().unwrap_or("");

        if role == "assistant" {
            let mut tool_use_ids: Vec<String> = Vec::new();
            let mut tool_use_lookup: HashSet<String> = HashSet::new();
            if let Some(content) = messages[i]["content"].as_array() {
                for block in content {
                    if block["type"].as_str() == Some("tool_use") {
                        if let Some(id) = block["id"].as_str() {
                            let id = id.to_string();
                            if tool_use_lookup.insert(id.clone()) {
                                tool_use_ids.push(id);
                            }
                        }
                    }
                }
            }

            if !tool_use_ids.is_empty() {
                let next_is_user =
                    i + 1 < messages.len() && messages[i + 1]["role"].as_str() == Some("user");

                if next_is_user {
                    let user_msg = &mut messages[i + 1];
                    let content = user_msg["content"].as_array().cloned().unwrap_or_default();

                    let mut filtered: Vec<Value> =
                        Vec::with_capacity(content.len() + tool_use_ids.len());
                    let mut result_ids: HashSet<String> =
                        HashSet::with_capacity(tool_use_ids.len());
                    for block in content {
                        if block["type"].as_str() == Some("tool_result") {
                            let id = block["tool_use_id"].as_str().unwrap_or("");
                            if tool_use_lookup.contains(id) {
                                result_ids.insert(id.to_string());
                                filtered.push(block);
                            } else {
                                debug!("Stripping orphaned tool_result for tool_use_id={}", id);
                            }
                        } else {
                            filtered.push(block);
                        }
                    }

                    for id in &tool_use_ids {
                        if !result_ids.contains(id) {
                            debug!("Injecting stub tool_result for missing tool_use_id={}", id);
                            filtered.push(stub_tool_result(id));
                        }
                    }

                    user_msg["content"] = Value::Array(filtered);
                } else {
                    debug!(
                        "Injecting user message with {} stub tool_results (no user message followed assistant with tool_use)",
                        tool_use_ids.len()
                    );
                    let stubs: Vec<Value> =
                        tool_use_ids.iter().map(|id| stub_tool_result(id)).collect();
                    messages.insert(
                        i + 1,
                        serde_json::json!({
                            "role": "user",
                            "content": stubs
                        }),
                    );
                }
            }
        }

        i += 1;
    }
}

fn stub_tool_result(tool_use_id: &str) -> Value {
    serde_json::json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": "Tool execution was interrupted",
        "is_error": true
    })
}
