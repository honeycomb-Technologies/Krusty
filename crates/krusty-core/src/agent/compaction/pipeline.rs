//! Compaction pipeline orchestration.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

use crate::agent::build_project_context;
use crate::agent::context_ledger::ContextLedger;
use crate::agent::summarizer::{generate_summary, SummarizationResult};
use crate::ai::client::AiClient;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::plan::{PlanFile, PlanManager};
use crate::storage::{
    CompactionStore, Database, FileActivityTracker, MessageStore, RankedFile, StoredMessageRecord,
};

use super::apply::build_compacted_conversation;
use super::budget::{estimate_tokens, CompactionManager};
use super::cut_point::{
    find_aggressive_cut_point, find_cut_point, find_last_compaction_index, IndexedMessage,
};
use super::microcompact::microcompact_messages;
use super::summarize::{extract_file_operations, extract_previous_summary, CompactionSummaryInput};

const PINCH_RANKED_FILE_LIMIT: usize = 20;
const PINCH_SUMMARY_FILE_CONTENT_LIMIT: usize = 10;

#[derive(Debug, Clone)]
pub enum CompactionTrigger {
    Auto,
    Manual {
        preservation_hints: Option<String>,
        direction: Option<String>,
    },
    Reactive,
    Overflow,
}

impl CompactionTrigger {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual { .. } => "manual",
            Self::Reactive => "reactive",
            Self::Overflow => "overflow",
        }
    }
}

pub struct CompactionRequest<'a> {
    pub db_path: &'a Path,
    pub session_id: &'a str,
    pub conversation: &'a [ModelMessage],
    pub working_dir: &'a Path,
    pub ai_client: Option<&'a AiClient>,
    pub model: Option<&'a str>,
    pub trigger: CompactionTrigger,
    pub compaction_manager: CompactionManager,
    /// Optional request-size estimate from the caller after runtime context injection.
    pub triggering_token_estimate: Option<usize>,
    pub last_usage_prompt_tokens: Option<usize>,
    pub messages_after_usage: usize,
    /// When set, skips the summarization LLM call (manual `/pinch` after preview).
    pub summary_override: Option<SummarizationResult>,
    pub project_dir: Option<&'a str>,
    pub user_id: Option<&'a str>,
}

pub struct CompactionResult {
    pub compacted_conversation: Vec<ModelMessage>,
    pub estimated_tokens_before: usize,
    pub estimated_tokens_after: usize,
    pub replaced_messages: usize,
    pub checkpoint_id: String,
    pub compaction_count: u32,
    pub summary: SummarizationResult,
}

pub async fn run_compaction_pipeline(request: CompactionRequest<'_>) -> Result<CompactionResult> {
    if request.conversation.is_empty() {
        bail!("Cannot compact session with no messages");
    }

    let micro = microcompact_messages(request.conversation);
    let working_conversation = if micro.changed {
        micro.messages
    } else {
        request.conversation.to_vec()
    };

    let raw_conversation_tokens = estimate_tokens(&working_conversation);
    let estimated_tokens_before = request.triggering_token_estimate.unwrap_or_else(|| {
        super::budget::estimate_with_usage(
            &working_conversation,
            request.last_usage_prompt_tokens,
            request.messages_after_usage,
        )
    });

    if matches!(request.trigger, CompactionTrigger::Auto)
        && !request
            .compaction_manager
            .should_compact(estimated_tokens_before)
    {
        bail!("Compaction trigger threshold not reached");
    }

    let indexed_messages = load_indexed_messages(request.db_path, request.session_id)?;
    if indexed_messages.is_empty() {
        bail!("Cannot compact session with no persisted messages");
    }

    let compaction_window_start = find_last_compaction_index(&working_conversation);
    let ranked_files = ranked_files_for_session(request.db_path, request.session_id);
    let file_contents = load_key_file_contents(
        request.working_dir,
        &ranked_files,
        PINCH_SUMMARY_FILE_CONTENT_LIMIT,
    );
    let project_context = load_project_context(request.working_dir);
    let active_plan = load_active_plan(request.db_path, request.session_id);
    let latest_user_objective =
        ContextLedger::from_conversation(&working_conversation).latest_user_objective;
    let previous_summary = extract_previous_summary(&working_conversation);

    let (preservation_hints, direction) = match &request.trigger {
        CompactionTrigger::Manual {
            preservation_hints,
            direction,
        } => (preservation_hints.clone(), direction.clone()),
        _ => (None, None),
    };

    let compaction_count = {
        let db = Database::new(request.db_path)?;
        CompactionStore::new(&db)
            .count_checkpoints(request.session_id)?
            .saturating_add(1)
    };

    let mut cut = None;
    for attempt in 0..4 {
        let keep_recent_tokens = request.compaction_manager.keep_recent_tokens_for_attempt(
            estimated_tokens_before,
            raw_conversation_tokens,
            attempt,
        );
        if let Some(candidate) = find_cut_point(
            &indexed_messages,
            compaction_window_start,
            keep_recent_tokens,
        ) {
            cut = Some(candidate);
            break;
        }
    }
    let cut = cut.or_else(|| find_aggressive_cut_point(&indexed_messages, compaction_window_start));
    let cut = cut.ok_or_else(|| anyhow::anyhow!("No valid compaction cut point found"))?;

    let summarize_messages: Vec<ModelMessage> = cut
        .messages_to_summarize
        .iter()
        .map(|indexed| indexed.message.clone())
        .collect();

    let summary = if let Some(summary) = request.summary_override.clone() {
        summary
    } else {
        summarize_for_compaction(
            request.ai_client,
            &summarize_messages,
            preservation_hints.as_deref(),
            &ranked_files,
            &file_contents,
            project_context.as_deref(),
            request.model,
        )
        .await
    };

    let (mut read_files, mut modified_files) = extract_file_operations(&summarize_messages);
    if let Some(previous) = &previous_summary {
        for path in extract_paths_from_tag(previous, "read-files") {
            if !read_files.contains(&path) {
                read_files.push(path);
            }
        }
        for path in extract_paths_from_tag(previous, "modified-files") {
            if !modified_files.contains(&path) {
                modified_files.push(path);
            }
        }
    }

    let pre_compact_ids: Vec<i64> = cut
        .messages_to_summarize
        .iter()
        .map(|message| message.id)
        .collect();
    // Keep exact pre-compact transcript content only in compaction_segments.
    // Checkpoints retain metadata and message ids, not a second full-history copy.
    let compacted_history_json = "[]";
    let segment_markdown = build_segment_markdown(&cut.messages_to_summarize);
    let segment_token_estimate = estimate_tokens(&summarize_messages);
    let checkpoint_id = Uuid::new_v4().to_string();

    let summary_input = CompactionSummaryInput {
        summary: summary.clone(),
        direction,
        preservation_hints,
        ranked_files: ranked_files.clone(),
        read_files,
        modified_files,
        checkpoint_id: checkpoint_id.clone(),
        compaction_count,
        latest_user_objective: latest_user_objective.clone(),
        previous_summary,
    };

    let mut compacted_conversation = build_compacted_conversation(
        &summary_input,
        request.trigger.as_str(),
        estimated_tokens_before,
        0,
        cut.first_kept_message_id,
        &cut.kept_messages,
    );

    if let Some(plan_markdown) = active_plan.as_ref().map(PlanFile::to_markdown) {
        append_plan_context(&mut compacted_conversation, &plan_markdown);
    }

    let estimated_tokens_after = estimate_tokens(&compacted_conversation);
    if let Some(boundary) = compacted_conversation.first_mut() {
        if let Some(text) = boundary.content.first_mut().and_then(|content| {
            if let crate::ai::types::Content::Text { text } = content {
                Some(text)
            } else {
                None
            }
        }) {
            if text.contains("\"tokens_after\":0") {
                *text = text.replace(
                    "\"tokens_after\":0",
                    &format!("\"tokens_after\":{estimated_tokens_after}"),
                );
            }
        }
    }

    persist_compaction_atomically(
        request.db_path,
        request.session_id,
        &checkpoint_id,
        cut.first_kept_index,
        &pre_compact_ids,
        compacted_history_json,
        latest_user_objective.as_deref(),
        &summary.important_files,
        pre_compact_ids.first().copied().unwrap_or(0),
        pre_compact_ids.last().copied().unwrap_or(0),
        &segment_markdown,
        segment_token_estimate,
        &compacted_conversation,
    )?;

    let replaced_messages = cut.messages_to_summarize.len();

    Ok(CompactionResult {
        compacted_conversation,
        estimated_tokens_before,
        estimated_tokens_after,
        replaced_messages,
        checkpoint_id,
        compaction_count,
        summary,
    })
}

#[allow(clippy::too_many_arguments)]
fn persist_compaction_atomically(
    db_path: &Path,
    session_id: &str,
    checkpoint_id: &str,
    prompt_index_at_compaction: usize,
    pre_compact_message_ids: &[i64],
    compacted_history_json: &str,
    original_user_info: Option<&str>,
    reread_file_paths: &[String],
    message_id_start: i64,
    message_id_end: i64,
    segment_markdown: &str,
    segment_token_estimate: usize,
    compacted_conversation: &[ModelMessage],
) -> Result<()> {
    let db = Database::new(db_path)?;
    let tx = db.conn().unchecked_transaction()?;
    let now = Utc::now().to_rfc3339();
    let pre_compact_message_ids_json = serde_json::to_string(pre_compact_message_ids)?;
    let reread_file_paths_json = serde_json::to_string(reread_file_paths)?;

    tx.execute(
        "INSERT INTO compaction_checkpoints (
            id, session_id, prompt_index_at_compaction,
            pre_compact_message_ids_json, compacted_history_json,
            original_user_info, reread_file_paths_json,
            schema_version, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            checkpoint_id,
            session_id,
            prompt_index_at_compaction,
            pre_compact_message_ids_json,
            compacted_history_json,
            original_user_info,
            reread_file_paths_json,
            1_i32,
            now,
        ],
    )
    .context("save compaction checkpoint")?;

    let segment_id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO compaction_segments (
            id, session_id, checkpoint_id,
            message_id_start, message_id_end,
            segment_markdown, token_estimate, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            segment_id,
            session_id,
            checkpoint_id,
            message_id_start,
            message_id_end,
            segment_markdown,
            segment_token_estimate,
            now,
        ],
    )
    .context("save compaction segment")?;

    tx.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])
        .context("delete pre-compaction messages")?;

    for message in compacted_conversation {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            _ => continue,
        };
        let content_json = serde_json::to_string(&message.content)?;
        tx.execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content_json, now],
        )
        .context("insert compacted message")?;
    }

    tx.execute(
        "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
        params![now, session_id],
    )
    .context("touch compacted session")?;
    tx.commit().context("commit compaction transaction")?;

    Ok(())
}

async fn summarize_for_compaction(
    ai_client: Option<&AiClient>,
    conversation: &[ModelMessage],
    preservation_hints: Option<&str>,
    ranked_files: &[RankedFile],
    file_contents: &[(String, String)],
    project_context: Option<&str>,
    model: Option<&str>,
) -> SummarizationResult {
    if let Some(client) = ai_client {
        match generate_summary(
            client,
            conversation,
            preservation_hints,
            ranked_files,
            file_contents,
            project_context,
            model,
        )
        .await
        {
            Ok(summary) if !summary.work_summary.trim().is_empty() => return summary,
            Ok(_) => tracing::warn!(
                "Compaction summarizer returned an empty summary; using deterministic fallback"
            ),
            Err(error) => {
                tracing::warn!(%error, "Compaction summarizer failed; using deterministic fallback")
            }
        }
    }

    deterministic_summary(conversation, preservation_hints, ranked_files)
}

fn deterministic_summary(
    conversation: &[ModelMessage],
    preservation_hints: Option<&str>,
    ranked_files: &[RankedFile],
) -> SummarizationResult {
    let mut excerpts = Vec::new();
    for message in conversation.iter().rev() {
        let role = match message.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            _ => continue,
        };
        let Some(text) = first_text(&message.content) else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        excerpts.push(format!("- {role}: {}", truncate_chars(trimmed, 700)));
        if excerpts.len() >= 8 {
            break;
        }
    }
    excerpts.reverse();

    let (read_files, modified_files) = extract_file_operations(conversation);
    let mut important_files: Vec<String> = ranked_files
        .iter()
        .map(|file| file.path.clone())
        .take(10)
        .collect();
    for path in read_files.iter().chain(modified_files.iter()) {
        if !important_files.contains(path) {
            important_files.push(path.clone());
        }
        if important_files.len() >= 10 {
            break;
        }
    }

    let mut work_summary = String::from(
        "A deterministic compaction summary was generated because AI summarization was unavailable or unusable. The archived checkpoint stores the full compacted transcript segment; use search_compaction_segments if exact details are needed.\n\n",
    );
    if let Some(hints) = preservation_hints.filter(|value| !value.trim().is_empty()) {
        work_summary.push_str("Preservation hints supplied by the user:\n");
        work_summary.push_str(&truncate_chars(hints.trim(), 1_000));
        work_summary.push_str("\n\n");
    }
    if excerpts.is_empty() {
        work_summary.push_str("The compacted segment contained tool-only or non-text messages.");
    } else {
        work_summary.push_str("Recent compacted transcript excerpts:\n");
        work_summary.push_str(&excerpts.join("\n"));
    }

    let pending_tasks = latest_user_text(conversation).into_iter().collect();

    SummarizationResult {
        work_summary,
        key_decisions: Vec::new(),
        pending_tasks,
        important_files,
    }
}

fn first_text(content: &[Content]) -> Option<&str> {
    content.iter().find_map(|content| match content {
        Content::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

fn latest_user_text(conversation: &[ModelMessage]) -> Option<String> {
    conversation.iter().rev().find_map(|message| {
        if message.role != Role::User {
            return None;
        }
        first_text(&message.content)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| truncate_chars(text, 500))
    })
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars).collect();
    truncated.push('…');
    truncated
}

fn load_indexed_messages(db_path: &Path, session_id: &str) -> Result<Vec<IndexedMessage>> {
    let db = Database::new(db_path)?;
    let store = MessageStore::new(&db);
    let records = store.load_session_message_records(session_id)?;
    let mut indexed = Vec::new();

    for record in records {
        match indexed_from_record(record) {
            Ok(Some(message)) => indexed.push(message),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }

    Ok(indexed)
}

fn indexed_from_record(record: StoredMessageRecord) -> Result<Option<IndexedMessage>> {
    let role = match record.role.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => return Ok(None),
    };
    let content: Vec<Content> = serde_json::from_str(&record.content_json).with_context(|| {
        format!(
            "failed to parse message {} while preparing compaction",
            record.id
        )
    })?;
    Ok(Some(IndexedMessage {
        id: record.id,
        message: ModelMessage { role, content },
    }))
}

fn ranked_files_for_session(db_path: &Path, session_id: &str) -> Vec<RankedFile> {
    let Ok(db) = Database::new(db_path) else {
        return Vec::new();
    };
    let tracker = FileActivityTracker::new(&db, session_id.to_string());
    tracker
        .get_ranked_files(PINCH_RANKED_FILE_LIMIT)
        .unwrap_or_default()
}

fn load_key_file_contents(
    working_dir: &Path,
    ranked_files: &[RankedFile],
    limit: usize,
) -> Vec<(String, String)> {
    ranked_files
        .iter()
        .take(limit)
        .filter_map(|file| {
            let path = if Path::new(&file.path).is_absolute() {
                PathBuf::from(&file.path)
            } else {
                working_dir.join(&file.path)
            };
            std::fs::read_to_string(path)
                .ok()
                .map(|content| (file.path.clone(), content))
        })
        .collect()
}

fn load_project_context(working_dir: &Path) -> Option<String> {
    let context = build_project_context(working_dir);
    (!context.trim().is_empty()).then_some(context)
}

fn load_active_plan(db_path: &Path, session_id: &str) -> Option<PlanFile> {
    PlanManager::new(db_path.to_path_buf())
        .ok()
        .and_then(|manager| manager.get_active_plan(session_id).ok().flatten())
}

fn build_segment_markdown(messages: &[IndexedMessage]) -> String {
    let mut segment = String::new();
    for indexed in messages {
        segment.push_str(&format!(
            "[message:{}:{:?}]\n",
            indexed.id, indexed.message.role
        ));
        for content in &indexed.message.content {
            match content {
                crate::ai::types::Content::Text { text } => {
                    segment.push_str(text);
                    segment.push('\n');
                }
                crate::ai::types::Content::ToolUse { name, input, .. } => {
                    segment.push_str(&format!("[tool_use:{name}] {input}\n"));
                }
                crate::ai::types::Content::ToolResult { output, .. } => {
                    segment.push_str(&format!("[tool_result] {output}\n"));
                }
                crate::ai::types::Content::Thinking { thinking, .. } => {
                    segment.push_str(&format!("[thinking] {thinking}\n"));
                }
                _ => {}
            }
        }
        segment.push_str("\n---\n");
    }
    segment
}

fn append_plan_context(conversation: &mut [ModelMessage], plan_markdown: &str) {
    if plan_markdown.trim().is_empty() {
        return;
    }
    let text = format!(
        "## Active Plan (post-compaction)\n\n{plan_markdown}\n\nContinue from the active plan state above."
    );
    if let Some(summary) = conversation.get_mut(1) {
        if let Some(crate::ai::types::Content::Text { text: existing }) =
            summary.content.first_mut()
        {
            existing.push_str("\n\n");
            existing.push_str(&text);
        }
    }
}

fn extract_paths_from_tag(previous_summary: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let Some(start) = previous_summary.find(&open) else {
        return Vec::new();
    };
    let Some(end) = previous_summary.find(&close) else {
        return Vec::new();
    };
    previous_summary[start + open.len()..end]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}
