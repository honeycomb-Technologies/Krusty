//! Compaction summary formatting and incremental merge support.

use crate::agent::summarizer::SummarizationResult;
use crate::storage::RankedFile;

use super::{COMPACTION_BOUNDARY_PREFIX, COMPACTION_SUMMARY_PREFIX};

#[derive(Debug, Clone)]
pub(crate) struct CompactionSummaryInput {
    pub summary: SummarizationResult,
    pub direction: Option<String>,
    pub preservation_hints: Option<String>,
    pub ranked_files: Vec<RankedFile>,
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub checkpoint_id: String,
    pub compaction_count: u32,
    pub latest_user_objective: Option<String>,
    pub previous_summary: Option<String>,
}

pub(crate) fn format_boundary_message(
    trigger: &str,
    tokens_before: usize,
    tokens_after: usize,
    first_kept_original_message_id: i64,
    checkpoint_id: &str,
    compaction_count: u32,
) -> String {
    serde_json::json!({
        "type": "compact_boundary",
        "trigger": trigger,
        "tokens_before": tokens_before,
        "tokens_after": tokens_after,
        "first_kept_original_message_id": first_kept_original_message_id,
        "checkpoint_id": checkpoint_id,
        "compaction_count": compaction_count,
    })
    .to_string()
}

pub(crate) fn format_summary_message(input: &CompactionSummaryInput) -> String {
    let mut msg = String::from(COMPACTION_SUMMARY_PREFIX);

    msg.push_str(
        "This session is being continued from a previous conversation that was compacted to stay within the context window.\n\n",
    );

    if let Some(direction) = &input.direction {
        msg.push_str("## Priority Direction\n\n");
        msg.push_str(direction);
        msg.push_str("\n\n");
    }

    if let Some(objective) = &input.latest_user_objective {
        msg.push_str("## Latest User Objective\n\n");
        msg.push_str(objective);
        msg.push_str("\n\n");
    }

    if let Some(previous) = &input.previous_summary {
        msg.push_str("## Prior Compaction Context\n\n");
        msg.push_str(previous);
        msg.push_str("\n\n");
    }

    msg.push_str("## Work Summary\n\n");
    msg.push_str(&input.summary.work_summary);
    msg.push_str("\n\n");

    if !input.summary.key_decisions.is_empty() {
        msg.push_str("## Key Decisions\n\n");
        for decision in &input.summary.key_decisions {
            msg.push_str(&format!("- {}\n", decision));
        }
        msg.push('\n');
    }

    if !input.summary.pending_tasks.is_empty() {
        msg.push_str("## Pending Tasks\n\n");
        for (index, task) in input.summary.pending_tasks.iter().enumerate() {
            msg.push_str(&format!("{}. {}\n", index + 1, task));
        }
        msg.push('\n');
    }

    if !input.read_files.is_empty() {
        msg.push_str("<read-files>\n");
        for path in &input.read_files {
            msg.push_str(path);
            msg.push('\n');
        }
        msg.push_str("</read-files>\n\n");
    }

    if !input.modified_files.is_empty() {
        msg.push_str("<modified-files>\n");
        for path in &input.modified_files {
            msg.push_str(path);
            msg.push('\n');
        }
        msg.push_str("</modified-files>\n\n");
    }

    if !input.ranked_files.is_empty() {
        msg.push_str("## Important Files\n\n");
        for (index, file) in input.ranked_files.iter().take(10).enumerate() {
            msg.push_str(&format!("{}. `{}`\n", index + 1, file.path));
        }
        msg.push('\n');
    }

    if let Some(hints) = &input.preservation_hints {
        msg.push_str("## Preservation Notes\n\n");
        msg.push_str(hints);
        msg.push_str("\n\n");
    }

    msg.push_str("## Recovery\n\n");
    msg.push_str(&format!(
        "Checkpoint `{}` (compaction #{}) archived pre-compact history in compaction segments. Use `search_compaction_segments` if exact prior details are needed.\n",
        input.checkpoint_id, input.compaction_count
    ));

    msg
}

pub(crate) fn boundary_user_text(payload_json: &str) -> String {
    format!("{COMPACTION_BOUNDARY_PREFIX}\n{payload_json}")
}

pub(crate) fn extract_previous_summary(
    messages: &[crate::ai::types::ModelMessage],
) -> Option<String> {
    use crate::ai::types::{Content, ModelMessage};

    messages.iter().rev().find_map(|message: &ModelMessage| {
        message.content.iter().find_map(|content| {
            if let Content::Text { text } = content {
                if text.starts_with(COMPACTION_SUMMARY_PREFIX) {
                    Some(text.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
    })
}

pub(crate) fn extract_file_operations(
    messages: &[crate::ai::types::ModelMessage],
) -> (Vec<String>, Vec<String>) {
    use crate::ai::types::Content;
    use std::collections::BTreeSet;

    let mut read_files = BTreeSet::new();
    let mut modified_files = BTreeSet::new();

    for message in messages {
        if message.role != crate::ai::types::Role::Assistant {
            continue;
        }
        for content in &message.content {
            if let Content::ToolUse { name, input, .. } = content {
                match name.as_str() {
                    "read" => {
                        if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                            read_files.insert(path.to_string());
                        }
                    }
                    "write" | "edit" | "multiedit" | "apply_patch" => {
                        if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                            modified_files.insert(path.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    (
        read_files.into_iter().collect(),
        modified_files.into_iter().collect(),
    )
}
