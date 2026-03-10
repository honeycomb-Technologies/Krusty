//! Live conversation compaction for long-running agent loops.
//!
//! This is distinct from pinch:
//! - compaction keeps the same session alive
//! - pinch creates a continuation session with a handoff artifact

use std::collections::HashSet;

use serde_json::Value;

use crate::agent::history_policy::{truncate_utf8, ToolRetention};
use crate::ai::model_profile::ModelProfile;
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::constants;

const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
const KEEP_RECENT_MESSAGES: usize = 6;
const KEEP_RECENT_THINKING_MESSAGES: usize = 2;
const SUMMARY_KEEP_RECENT_OPTIONS: &[usize] = &[KEEP_RECENT_MESSAGES, 4, 2, 1];
const MAX_TOOL_RESULT_PREVIEW_CHARS: usize = 480;
const MAX_SUMMARY_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionReason {
    ContextPressure,
}

impl CompactionReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ContextPressure => "context_pressure",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompactionOutcome {
    pub(crate) conversation: Vec<ModelMessage>,
    pub(crate) replaced_messages: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CompactionManager {
    trigger_tokens: usize,
    target_tokens: usize,
    hard_failure_tokens: usize,
}

impl Default for CompactionManager {
    fn default() -> Self {
        Self::for_model(
            ProviderId::MiniMax,
            ApiFormat::Anthropic,
            constants::ai::DEFAULT_MODEL,
            constants::ai::CONTEXT_WINDOW_TOKENS,
        )
    }
}

#[cfg(test)]
impl CompactionManager {
    fn with_budgets(
        trigger_tokens: usize,
        target_tokens: usize,
        hard_failure_tokens: usize,
    ) -> Self {
        Self {
            trigger_tokens,
            target_tokens,
            hard_failure_tokens,
        }
    }
}

impl CompactionManager {
    pub(crate) fn for_model(
        provider: ProviderId,
        api_format: ApiFormat,
        model_id: &str,
        context_window: usize,
    ) -> Self {
        let budgets = ModelProfile::resolve(provider, api_format, model_id)
            .compaction_budgets(context_window);

        Self {
            trigger_tokens: budgets.trigger_tokens,
            target_tokens: budgets.target_tokens,
            hard_failure_tokens: budgets.hard_failure_tokens,
        }
    }

    pub(crate) fn should_compact(&self, estimated_tokens: usize) -> bool {
        estimated_tokens >= self.trigger_tokens
    }

    pub(crate) fn is_hard_failure(&self, estimated_tokens: usize) -> bool {
        estimated_tokens >= self.hard_failure_tokens
    }

    pub(crate) fn compact_conversation(
        &self,
        conversation: &[ModelMessage],
        reason: CompactionReason,
    ) -> Option<CompactionOutcome> {
        let mut compacted = conversation.to_vec();
        let mut replaced_messages = 0usize;

        replaced_messages += strip_old_thinking(&mut compacted);
        replaced_messages += compact_old_tool_results(&mut compacted);
        compacted.retain(message_has_meaningful_content);

        let baseline_estimate = estimate_tokens(&compacted);
        if baseline_estimate > self.target_tokens {
            let mut best_replacement: Option<(SummaryReplacement, usize)> = None;

            for &keep_recent_messages in SUMMARY_KEEP_RECENT_OPTIONS {
                let Some(candidate) =
                    build_summary_replacement(&compacted, reason, keep_recent_messages)
                else {
                    continue;
                };
                let candidate_estimate = estimate_tokens(&candidate.conversation);

                if best_replacement
                    .as_ref()
                    .is_none_or(|(_, best_estimate)| candidate_estimate < *best_estimate)
                {
                    best_replacement = Some((candidate, candidate_estimate));
                }

                if candidate_estimate <= self.target_tokens {
                    break;
                }
            }

            let (mut summary_replacement, best_estimate) = best_replacement?;
            if best_estimate > self.target_tokens {
                summary_replacement = build_continuation_replacement(&compacted, reason, 1)?;
            }

            replaced_messages += summary_replacement.replaced_messages;
            compacted = summary_replacement.conversation;
            compacted.retain(message_has_meaningful_content);
        }

        if replaced_messages == 0 {
            return None;
        }

        Some(CompactionOutcome {
            conversation: compacted,
            replaced_messages,
        })
    }
}

pub(crate) fn estimate_tokens(messages: &[ModelMessage]) -> usize {
    let total_chars: usize = messages
        .iter()
        .flat_map(|message| &message.content)
        .map(content_char_len)
        .sum();

    total_chars / CHARS_PER_TOKEN_ESTIMATE
}

fn content_char_len(content: &Content) -> usize {
    match content {
        Content::Text { text } => text.len(),
        Content::ToolUse { name, input, .. } => name.len() + input.to_string().len(),
        Content::ToolResult { output, .. } => output.to_string().len(),
        Content::Image { .. } => 1_000,
        Content::Document { .. } => 5_000,
        Content::Thinking { thinking, .. } => thinking.len(),
        Content::RedactedThinking { .. } => 100,
    }
}

fn strip_old_thinking(conversation: &mut [ModelMessage]) -> usize {
    let mut assistant_with_thinking = conversation
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message.role == Role::Assistant
                && message.content.iter().any(|content| {
                    matches!(
                        content,
                        Content::Thinking { .. } | Content::RedactedThinking { .. }
                    )
                })
        })
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();

    if assistant_with_thinking.len() <= KEEP_RECENT_THINKING_MESSAGES {
        return 0;
    }

    let keep_from = assistant_with_thinking
        .len()
        .saturating_sub(KEEP_RECENT_THINKING_MESSAGES);
    let keep = assistant_with_thinking.split_off(keep_from);
    let keep_set = keep.into_iter().collect::<HashSet<_>>();
    let mut replaced = 0usize;

    for (idx, message) in conversation.iter_mut().enumerate() {
        if !keep_set.contains(&idx) {
            let before = message.content.len();
            message.content.retain(|content| {
                !matches!(
                    content,
                    Content::Thinking { .. } | Content::RedactedThinking { .. }
                )
            });
            replaced += before.saturating_sub(message.content.len());
        }
    }

    replaced
}

fn compact_old_tool_results(conversation: &mut [ModelMessage]) -> usize {
    let non_system_indices = conversation
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role != Role::System)
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();

    if non_system_indices.len() <= KEEP_RECENT_MESSAGES {
        return 0;
    }

    let keep_recent = non_system_indices
        .iter()
        .rev()
        .take(KEEP_RECENT_MESSAGES)
        .copied()
        .collect::<HashSet<_>>();
    let mut replaced = 0usize;

    for (idx, message) in conversation.iter_mut().enumerate() {
        if keep_recent.contains(&idx) {
            continue;
        }

        for content in &mut message.content {
            if let Content::ToolResult { output, .. } = content {
                let retention = ToolRetention::from_output_value(output);
                let rendered = output.to_string();

                if retention == ToolRetention::DropAfterCompaction {
                    *output = compacted_tool_result(
                        output,
                        "Older tool output was dropped after compaction. Re-run the tool if exact details are still needed.",
                        retention,
                    );
                    replaced += 1;
                    continue;
                }

                if rendered.len() <= MAX_TOOL_RESULT_PREVIEW_CHARS
                    && retention != ToolRetention::SummarizeAfterTurn
                {
                    continue;
                }

                *output = compacted_tool_result(
                    output,
                    "Older tool output was compacted to preserve the live context budget. Re-run the tool or inspect the filesystem if exact output is needed.",
                    retention,
                );
                replaced += 1;
            }
        }
    }

    replaced
}

fn compacted_tool_result(output: &Value, note: &str, retention: ToolRetention) -> Value {
    let summary = output.get("summary").cloned().unwrap_or_else(|| {
        Value::String(truncate_utf8(
            &output.to_string(),
            MAX_TOOL_RESULT_PREVIEW_CHARS,
        ))
    });

    serde_json::json!({
        "compacted": true,
        "tool": output.get("tool").cloned().unwrap_or(Value::String("tool".to_string())),
        "retention": retention.as_str(),
        "summary": summary,
        "note": note,
    })
}

struct SummaryReplacement {
    conversation: Vec<ModelMessage>,
    replaced_messages: usize,
}

fn build_summary_replacement(
    conversation: &[ModelMessage],
    reason: CompactionReason,
    keep_recent_messages: usize,
) -> Option<SummaryReplacement> {
    let first_user_idx = conversation
        .iter()
        .position(|message| message.role == Role::User)?;
    let non_system_indices = conversation
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role != Role::System)
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();

    if non_system_indices.len() <= keep_recent_messages + 1 {
        return None;
    }

    let recent_tail = non_system_indices
        .iter()
        .rev()
        .take(keep_recent_messages)
        .copied()
        .collect::<Vec<_>>();
    let recent_tail_start = recent_tail.into_iter().min().unwrap_or(first_user_idx + 1);

    if recent_tail_start <= first_user_idx + 1 {
        return None;
    }

    let middle = conversation[first_user_idx + 1..recent_tail_start]
        .iter()
        .filter(|message| message.role != Role::System)
        .cloned()
        .collect::<Vec<_>>();
    if middle.is_empty() {
        return None;
    }

    let summary_text = build_summary_text(&middle, reason);
    if summary_text.is_empty() {
        return None;
    }

    let summary_message = ModelMessage {
        role: Role::Assistant,
        content: vec![Content::Text { text: summary_text }],
    };

    let mut compacted = Vec::with_capacity(conversation.len() - middle.len() + 1);
    let mut inserted_summary = false;

    for (idx, message) in conversation.iter().enumerate() {
        if idx <= first_user_idx || idx >= recent_tail_start || message.role == Role::System {
            compacted.push(message.clone());
            continue;
        }

        if !inserted_summary {
            compacted.push(summary_message.clone());
            inserted_summary = true;
        }
    }

    Some(SummaryReplacement {
        conversation: compacted,
        replaced_messages: middle.len(),
    })
}

fn build_continuation_replacement(
    conversation: &[ModelMessage],
    reason: CompactionReason,
    keep_recent_messages: usize,
) -> Option<SummaryReplacement> {
    let non_system_indices = conversation
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role != Role::System)
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();

    if non_system_indices.len() <= keep_recent_messages {
        return None;
    }

    let recent_tail = non_system_indices
        .iter()
        .rev()
        .take(keep_recent_messages)
        .copied()
        .collect::<Vec<_>>();
    let recent_tail_start = recent_tail.into_iter().min()?;

    let earlier = conversation[..recent_tail_start]
        .iter()
        .filter(|message| message.role != Role::System)
        .cloned()
        .collect::<Vec<_>>();
    if earlier.is_empty() {
        return None;
    }

    let summary_text = build_summary_text(&earlier, reason);
    if summary_text.is_empty() {
        return None;
    }

    let summary_message = ModelMessage {
        role: Role::Assistant,
        content: vec![Content::Text { text: summary_text }],
    };

    let mut compacted = vec![summary_message];
    compacted.extend(
        conversation
            .iter()
            .enumerate()
            .filter(|(idx, message)| *idx >= recent_tail_start || message.role == Role::System)
            .map(|(_, message)| message.clone()),
    );

    Some(SummaryReplacement {
        conversation: compacted,
        replaced_messages: earlier.len(),
    })
}

fn build_summary_text(messages: &[ModelMessage], reason: CompactionReason) -> String {
    let mut user_goals = Vec::new();
    let mut assistant_progress = Vec::new();
    let mut tool_activity = Vec::new();
    let mut latest_user_request = None;

    for message in messages {
        match message.role {
            Role::User => {
                for content in &message.content {
                    match content {
                        Content::Text { text } => {
                            let line = first_line(text);
                            if !line.is_empty() {
                                latest_user_request = Some(truncate_utf8(&line, 200).to_string());
                            }
                            push_unique_limited(
                                &mut user_goals,
                                truncate_utf8(&line, 160).to_string(),
                                6,
                            );
                        }
                        Content::ToolResult { output, .. } => {
                            push_unique_limited(
                                &mut tool_activity,
                                format!(
                                    "tool result compacted: {}",
                                    truncate_utf8(&output.to_string(), 120)
                                ),
                                8,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Role::Assistant => {
                for content in &message.content {
                    match content {
                        Content::Text { text } => {
                            let line = first_line(text);
                            push_unique_limited(
                                &mut assistant_progress,
                                truncate_utf8(&line, 160).to_string(),
                                6,
                            );
                        }
                        Content::ToolUse { name, input, .. } => {
                            push_unique_limited(
                                &mut tool_activity,
                                describe_tool_use(name, input),
                                8,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Role::System | Role::Tool => {}
        }
    }

    let mut summary = String::from("[LIVE CONTEXT COMPACTION]\n\n");
    summary.push_str("Earlier turn history was compacted to keep this session alive. Continue from the preserved recent context and current filesystem state.\n\n");
    summary.push_str(&format!("Reason: {}\n\n", reason.as_str()));

    if let Some(latest_user_request) = latest_user_request {
        summary.push_str("Latest carried-forward user request:\n");
        summary.push_str("- ");
        summary.push_str(&latest_user_request);
        summary.push_str("\n\n");
    }

    if !user_goals.is_empty() {
        summary.push_str("User goals still in scope:\n");
        for item in &user_goals {
            summary.push_str("- ");
            summary.push_str(item);
            summary.push('\n');
        }
        summary.push('\n');
    }

    if !assistant_progress.is_empty() {
        summary.push_str("Earlier progress and decisions:\n");
        for item in &assistant_progress {
            summary.push_str("- ");
            summary.push_str(item);
            summary.push('\n');
        }
        summary.push('\n');
    }

    if !tool_activity.is_empty() {
        summary.push_str("Important tool activity:\n");
        for item in &tool_activity {
            summary.push_str("- ");
            summary.push_str(item);
            summary.push('\n');
        }
    }

    truncate_utf8(&summary, MAX_SUMMARY_CHARS)
}

fn push_unique_limited(items: &mut Vec<String>, value: String, limit: usize) {
    if value.is_empty() || items.len() >= limit || items.iter().any(|existing| existing == &value) {
        return;
    }
    items.push(value);
}

fn describe_tool_use(name: &str, input: &serde_json::Value) -> String {
    let maybe_path = input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(|path| truncate_utf8(path, 120));

    match maybe_path {
        Some(path) => format!("{name} {path}"),
        None => truncate_utf8(&format!("{name} {}", input), 160),
    }
}

fn message_has_meaningful_content(message: &ModelMessage) -> bool {
    !message.content.is_empty()
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{build_summary_text, estimate_tokens, CompactionManager, CompactionReason};
    use crate::ai::types::{Content, ModelMessage, Role};

    fn text_message(role: Role, text: &str) -> ModelMessage {
        ModelMessage {
            role,
            content: vec![Content::Text {
                text: text.to_string(),
            }],
        }
    }

    #[test]
    fn compaction_reduces_large_tool_output() {
        let manager = CompactionManager::with_budgets(1, 1, usize::MAX);
        let large_output = "x".repeat(8_000);
        let conversation = vec![
            text_message(Role::User, "Implement the feature."),
            ModelMessage {
                role: Role::Assistant,
                content: vec![Content::ToolUse {
                    id: "tool-1".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"command": "cargo test"}),
                }],
            },
            ModelMessage {
                role: Role::User,
                content: vec![Content::ToolResult {
                    tool_use_id: "tool-1".to_string(),
                    output: serde_json::Value::String(large_output),
                    is_error: Some(false),
                }],
            },
            text_message(Role::Assistant, "I found the failing tests."),
            text_message(Role::User, "Fix them and continue."),
            text_message(Role::Assistant, "Working on the next patch."),
            text_message(Role::User, "Keep going."),
            text_message(Role::Assistant, "Recent context should stay verbatim."),
            text_message(Role::User, "Now focus on the final polish."),
            text_message(Role::Assistant, "Preparing the last patch."),
            text_message(Role::User, "Stay on this thread."),
            text_message(Role::Assistant, "Still using the latest context."),
        ];

        let before = estimate_tokens(&conversation);
        let outcome = manager
            .compact_conversation(&conversation, CompactionReason::ContextPressure)
            .expect("expected compaction to make progress");
        let after = estimate_tokens(&outcome.conversation);

        assert!(after < before);
        assert!(outcome.replaced_messages > 0);
    }

    #[test]
    fn compaction_inserts_summary_when_history_is_long() {
        let manager = CompactionManager::with_budgets(1, 10, usize::MAX);
        let mut conversation = vec![text_message(Role::User, "Build the feature end to end.")];

        for idx in 0..12 {
            conversation.push(text_message(
                Role::Assistant,
                &format!("Completed earlier step {idx}. {}", "details ".repeat(80)),
            ));
            conversation.push(text_message(
                Role::User,
                &format!(
                    "Continue with follow-up step {idx}. {}",
                    "more context ".repeat(60)
                ),
            ));
        }

        let outcome = manager
            .compact_conversation(&conversation, CompactionReason::ContextPressure)
            .expect("expected summary compaction");

        assert!(outcome
            .conversation
            .iter()
            .any(|message| message.content.iter().any(|content| matches!(
                content,
                Content::Text { text } if text.contains("[LIVE CONTEXT COMPACTION]")
            ))));
    }

    #[test]
    fn compaction_can_shrink_recent_tail_more_aggressively() {
        let manager = CompactionManager::with_budgets(1, 220, usize::MAX);
        let mut conversation = vec![text_message(Role::User, "Ship the feature.")];

        for idx in 0..14 {
            conversation.push(text_message(
                Role::Assistant,
                &format!("Earlier assistant step {idx}. {}", "details ".repeat(60)),
            ));
            conversation.push(text_message(
                Role::User,
                &format!("Earlier user instruction {idx}. {}", "context ".repeat(40)),
            ));
        }

        let outcome = manager
            .compact_conversation(&conversation, CompactionReason::ContextPressure)
            .expect("expected aggressive summary compaction");

        assert!(estimate_tokens(&outcome.conversation) < estimate_tokens(&conversation));
        assert!(outcome.conversation.len() <= 2);
        assert!(outcome
            .conversation
            .iter()
            .any(|message| message.content.iter().any(|content| matches!(
                content,
                Content::Text { text } if text.contains("[LIVE CONTEXT COMPACTION]")
            ))));
    }

    #[test]
    fn summary_text_carries_forward_latest_user_request() {
        let summary = build_summary_text(
            &[
                text_message(Role::User, "Initial task."),
                text_message(Role::Assistant, "Investigated the codebase."),
                text_message(Role::User, "Fix the streaming stall and keep going."),
            ],
            CompactionReason::ContextPressure,
        );

        assert!(summary.contains("Latest carried-forward user request"));
        assert!(summary.contains("Fix the streaming stall and keep going."));
    }

    #[test]
    fn compaction_preserves_pinned_system_context() {
        let manager = CompactionManager::with_budgets(1, 30, usize::MAX);
        let mut conversation = vec![text_message(
            Role::System,
            "[PROJECT INSTRUCTIONS - /repo/AGENTS.md]\nAlways preserve this context.",
        )];
        conversation.push(text_message(Role::User, "Implement end-to-end fixes."));

        for idx in 0..10 {
            conversation.push(text_message(
                Role::Assistant,
                &format!("Step {idx}: {}", "detail ".repeat(60)),
            ));
            conversation.push(text_message(
                Role::User,
                &format!("Continue with step {idx}. {}", "context ".repeat(50)),
            ));
        }

        let outcome = manager
            .compact_conversation(&conversation, CompactionReason::ContextPressure)
            .expect("expected compaction output");

        let preserved = outcome.conversation.iter().any(|message| {
            message.role == Role::System
                && message.content.iter().any(|content| {
                    matches!(
                        content,
                        Content::Text { text } if text.contains("[PROJECT INSTRUCTIONS")
                    )
                })
        });

        assert!(
            preserved,
            "Pinned system instructions must survive compaction"
        );
    }
}
