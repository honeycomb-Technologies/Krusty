use std::collections::BTreeSet;
use std::path::Path;

use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::{
    is_compaction_flush_memory, is_current_snapshot, AgentMemory, MemoryStore, MemoryType,
};

use super::{open_context_database, truncate_utf8};

/// Maximum number of memories injected per type (most recent first).
const MAX_MEMORIES_PER_TYPE: usize = 4;
/// Maximum character length for a single memory preview in the injection.
pub(super) const MAX_MEMORY_CONTENT_CHARS: usize = 180;
/// Approximate upper bound on total memory context output size.
const MAX_MEMORY_CONTEXT_BYTES: usize = 3 * 1024;
const MAX_QUERY_MESSAGES: usize = 1;

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
        let typed = memories
            .iter()
            .filter(|memory| memory.memory_type == *memory_type)
            .filter(|memory| should_inject_memory(memory, *memory_type, &query_terms))
            .take(MAX_MEMORIES_PER_TYPE)
            .collect::<Vec<_>>();
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

        for memory in typed {
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
    sections.join("\n")
}

fn should_inject_memory(
    memory: &AgentMemory,
    _memory_type: MemoryType,
    query_terms: &BTreeSet<String>,
) -> bool {
    memory_matches_query(memory, query_terms)
}

fn memory_matches_query(memory: &AgentMemory, query_terms: &BTreeSet<String>) -> bool {
    if query_terms.is_empty() {
        return false;
    }

    let haystack_terms = tokenize(&format!("{} {}", memory.title, memory.content));
    query_terms.iter().any(|term| haystack_terms.contains(term))
}

fn relevance_terms(conversation: &[ModelMessage]) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for text in conversation
        .iter()
        .rev()
        .filter(|message| message.role == Role::User)
        .filter_map(first_text)
        .take(MAX_QUERY_MESSAGES)
    {
        terms.extend(tokenize(text));
    }
    terms
}

fn first_text(message: &ModelMessage) -> Option<&str> {
    message.content.iter().find_map(|content| match content {
        Content::Text { text } => Some(text.as_str()),
        _ => None,
    })
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
        "about"
            | "after"
            | "again"
            | "also"
            | "before"
            | "being"
            | "could"
            | "current"
            | "hello"
            | "more"
            | "need"
            | "please"
            | "some"
            | "tell"
            | "that"
            | "their"
            | "there"
            | "these"
            | "thing"
            | "this"
            | "those"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "with"
            | "would"
            | "your"
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
