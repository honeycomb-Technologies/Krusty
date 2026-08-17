use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::agent::context_ledger::ContextLedger;
use crate::ai::types::ModelMessage;
use crate::storage::{
    is_compaction_flush_memory, is_current_snapshot, AgentMemory, MemoryStore, MemoryType,
};

use super::{open_context_database, truncate_utf8, truncate_utf8_bytes};

/// Maximum number of memories injected per type (highest relevance first,
/// preserving store recency for equal scores).
const MAX_MEMORIES_PER_TYPE: usize = 3;
/// Maximum character length for a single memory preview in the injection.
pub(super) const MAX_MEMORY_CONTENT_CHARS: usize = 180;
/// Approximate upper bound on total memory context output size.
const MAX_MEMORY_CONTEXT_BYTES: usize = 2 * 1024;

/// Build persistent memory context from the agent memory store.
///
/// Returns an empty string when no relevant memories exist, keeping the
/// system prompt lean for fresh or unrelated turns. Memories are only injected
/// when their title/content appears relevant to the latest user request. Raw
/// compaction flush memories are never injected; exact archived history belongs
/// in compaction segments and `search_compaction_segments`.
pub(super) fn build_memory_context(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    conversation: &[ModelMessage],
) -> String {
    let Some(db) = open_context_database(db_path, "building memory context") else {
        return String::new();
    };
    let store = MemoryStore::new(db);
    let query_terms = relevance_terms(conversation);
    let memories = store
        .list(project_dir, user_id)
        .into_iter()
        .filter(|memory| !is_current_snapshot(memory))
        .filter(|memory| !is_compaction_flush_memory(memory))
        .collect::<Vec<_>>();
    if memories.is_empty() {
        return String::new();
    }

    let mut sections: Vec<String> = Vec::new();
    sections.push("[PERSISTENT MEMORY]".to_string());
    sections.push(
        "Relevant durable memory is shown as short previews. Use it only when it helps the current request; do not call the memory tool for generic or unrelated questions unless the user asks about stored memory. Verify remembered project facts against current state before acting."
            .to_string(),
    );

    let mut total_len: usize = sections.iter().map(|s| s.len()).sum();
    let mut injected_any = false;

    for memory_type in &[
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ] {
        let mut typed = memories
            .iter()
            .filter(|memory| memory.memory_type == *memory_type)
            .filter_map(|memory| {
                let score = memory_relevance_score(memory, &query_terms);
                (score > 0).then_some((score, memory))
            })
            .collect::<Vec<_>>();
        typed.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        typed.truncate(MAX_MEMORIES_PER_TYPE);
        if typed.is_empty() {
            continue;
        }

        let header = match memory_type {
            MemoryType::User => "## User Context",
            MemoryType::Feedback => "## Feedback & Guidance",
            MemoryType::Project => "## Relevant Project Context",
            MemoryType::Reference => "## Relevant External References",
        };
        sections.push(header.to_string());
        total_len += header.len();
        injected_any = true;

        for (_, memory) in typed {
            let content = memory_preview(&memory.content);
            let line = format!("- **{}**: {}", memory.title, content);
            total_len += line.len() + 1;
            if total_len > MAX_MEMORY_CONTEXT_BYTES {
                break;
            }
            sections.push(line);
        }

        if total_len > MAX_MEMORY_CONTEXT_BYTES {
            break;
        }
    }

    if !injected_any {
        return String::new();
    }

    sections.push("[/PERSISTENT MEMORY]".to_string());
    let context = sections.join("\n");
    if context.len() <= MAX_MEMORY_CONTEXT_BYTES {
        return context;
    }

    const END_MARKER: &str =
        "\n[PERSISTENT MEMORY TRUNCATED AT REQUEST BUDGET]\n[/PERSISTENT MEMORY]";
    let mut bounded = truncate_utf8_bytes(
        &context,
        MAX_MEMORY_CONTEXT_BYTES.saturating_sub(END_MARKER.len()),
    );
    bounded.push_str(END_MARKER);
    bounded
}

fn memory_relevance_score(memory: &AgentMemory, query_terms: &BTreeSet<String>) -> f32 {
    if query_terms.is_empty() {
        return 0.0;
    }

    let title_terms = tokenize(&memory.title);
    let content_terms = tokenize(&memory.content);
    let matches = query_terms
        .iter()
        .filter(|term| title_terms.contains(*term) || content_terms.contains(*term))
        .collect::<Vec<_>>();
    if matches.is_empty() || (matches.len() == 1 && is_generic_relevance_term(matches[0].as_str()))
    {
        return 0.0;
    }

    let overlap = matches.iter().fold(0.0_f32, |score, term| {
        score
            + f32::from(content_terms.contains(*term))
            + 4.0 * f32::from(title_terms.contains(*term))
            + f32::from(term.chars().count() >= 8)
    });
    if overlap <= 0.0 {
        return 0.0;
    }
    let recency = recency_multiplier(&memory.updated_at);
    let confidence = (memory.confidence as f32).clamp(0.05, 1.0);
    overlap * recency * (0.5 + 0.5 * confidence)
}

fn recency_multiplier(updated_at: &str) -> f32 {
    let Some(updated) = DateTime::parse_from_rfc3339(updated_at)
        .ok()
        .map(|value| value.with_timezone(&Utc))
    else {
        return 0.75;
    };
    let age_days = (Utc::now() - updated).num_days().max(0) as f32;
    (1.0 - (age_days / 60.0) * 0.5).clamp(0.5, 1.0)
}

fn relevance_terms(conversation: &[ModelMessage]) -> BTreeSet<String> {
    ContextLedger::from_conversation(conversation)
        .latest_user_objective
        .map_or_else(BTreeSet::new, |objective| tokenize(&objective))
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| token.chars().count() >= 3)
        .map(str::to_lowercase)
        .filter(|token| !is_stopword(token))
        .collect()
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "all"
            | "and"
            | "are"
            | "about"
            | "after"
            | "again"
            | "also"
            | "before"
            | "being"
            | "can"
            | "change"
            | "changes"
            | "code"
            | "coding"
            | "could"
            | "current"
            | "did"
            | "does"
            | "doing"
            | "done"
            | "fix"
            | "for"
            | "from"
            | "has"
            | "have"
            | "issue"
            | "its"
            | "hello"
            | "how"
            | "make"
            | "memory"
            | "more"
            | "need"
            | "new"
            | "not"
            | "now"
            | "old"
            | "please"
            | "problem"
            | "project"
            | "system"
            | "test"
            | "tests"
            | "some"
            | "tell"
            | "that"
            | "the"
            | "their"
            | "there"
            | "these"
            | "thing"
            | "this"
            | "those"
            | "then"
            | "they"
            | "use"
            | "using"
            | "want"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "will"
            | "with"
            | "work"
            | "would"
            | "you"
            | "your"
    )
}

fn is_generic_relevance_term(token: &str) -> bool {
    matches!(
        token,
        "agent"
            | "context"
            | "feature"
            | "file"
            | "files"
            | "harness"
            | "model"
            | "request"
            | "server"
            | "tool"
            | "tools"
            | "update"
    )
}

fn memory_preview(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_utf8(&compact, MAX_MEMORY_CONTENT_CHARS)
}

pub(super) fn format_memory_kind(memory_type: MemoryType) -> &'static str {
    match memory_type {
        MemoryType::User => "User",
        MemoryType::Feedback => "Feedback",
        MemoryType::Project => "Project",
        MemoryType::Reference => "Reference",
    }
}
