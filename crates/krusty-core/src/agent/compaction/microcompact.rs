//! Layer 0: cheap per-turn pressure relief before full compaction.

use crate::agent::history_policy::ToolRetention;
use crate::ai::types::{Content, ModelMessage, Role};

const KEEP_RECENT_MESSAGES: usize = 6;
const KEEP_RECENT_THINKING_ASSISTANTS: usize = 2;

pub(crate) struct MicrocompactResult {
    pub messages: Vec<ModelMessage>,
    pub changed: bool,
}

pub(crate) fn microcompact_messages(conversation: &[ModelMessage]) -> MicrocompactResult {
    if conversation.is_empty() {
        return MicrocompactResult {
            messages: Vec::new(),
            changed: false,
        };
    }

    let mut messages = conversation.to_vec();
    let mut changed = false;

    if strip_old_thinking(&mut messages) {
        changed = true;
    }
    if compact_old_tool_results(&mut messages) {
        changed = true;
    }

    MicrocompactResult { messages, changed }
}

fn strip_old_thinking(messages: &mut [ModelMessage]) -> bool {
    let mut assistant_with_thinking = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role == Role::Assistant
            && message
                .content
                .iter()
                .any(|content| matches!(content, Content::Thinking { .. }))
        {
            assistant_with_thinking.push(index);
        }
    }

    if assistant_with_thinking.len() <= KEEP_RECENT_THINKING_ASSISTANTS {
        return false;
    }

    let strip_count = assistant_with_thinking.len() - KEEP_RECENT_THINKING_ASSISTANTS;
    let mut changed = false;
    for index in assistant_with_thinking.into_iter().take(strip_count) {
        let message = &mut messages[index];
        let original_len = message.content.len();
        message
            .content
            .retain(|content| !matches!(content, Content::Thinking { .. }));
        if message.content.len() != original_len {
            changed = true;
        }
    }
    changed
}

fn compact_old_tool_results(messages: &mut [ModelMessage]) -> bool {
    if messages.len() <= KEEP_RECENT_MESSAGES {
        return false;
    }

    let cutoff = messages.len().saturating_sub(KEEP_RECENT_MESSAGES);
    let mut changed = false;

    for message in &mut messages[..cutoff] {
        if message.role != Role::User {
            continue;
        }
        for content in &mut message.content {
            let Content::ToolResult { output, .. } = content else {
                continue;
            };
            let retention = output
                .get("retention")
                .and_then(|value| value.as_str())
                .unwrap_or("retain_full");
            let summary = output
                .get("summary")
                .and_then(|value| value.as_str())
                .unwrap_or("tool result cleared");

            match retention {
                "drop_after_compaction" => {
                    *output = serde_json::json!({
                        "retention": ToolRetention::DropAfterCompaction.as_str(),
                        "summary": summary,
                        "result": "[Old tool result content cleared during microcompact — re-run tool if needed]",
                    });
                    changed = true;
                }
                "summarize_after_turn" => {
                    if let Some(result) = output.get("result") {
                        let compact = serde_json::to_string(result).unwrap_or_default();
                        if compact.chars().count() > 500 {
                            output["result"] = serde_json::json!(format!(
                                "{}...[microcompact truncated]",
                                truncate_chars(&compact, 500)
                            ));
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    changed
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}
