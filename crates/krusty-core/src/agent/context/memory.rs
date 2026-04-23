use std::path::Path;

use crate::storage::{is_current_snapshot, MemoryStore, MemoryType};

use super::{open_context_database, truncate_utf8};

/// Maximum number of memories injected per type (most recent first).
const MAX_MEMORIES_PER_TYPE: usize = 15;
/// Maximum character length for a single memory's content in the injection.
pub(super) const MAX_MEMORY_CONTENT_CHARS: usize = 300;
/// Approximate upper bound on total memory context output size.
const MAX_MEMORY_CONTEXT_BYTES: usize = 8 * 1024;

/// Build persistent memory context from the agent memory store.
///
/// Returns an empty string when no memories exist, keeping the system
/// prompt lean for fresh sessions. Caps per-type count, individual
/// content length, and total output size to prevent memory injection
/// from consuming too much context budget.
pub(super) fn build_memory_context(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> String {
    let Some(db) = open_context_database(db_path, "building memory context") else {
        return String::new();
    };
    let store = MemoryStore::new(db);
    let memories = store
        .list(project_dir, user_id)
        .into_iter()
        .filter(|memory| !is_current_snapshot(memory))
        .collect::<Vec<_>>();
    if memories.is_empty() {
        return String::new();
    }

    let mut sections: Vec<String> = Vec::new();
    sections.push("[PERSISTENT MEMORY]".to_string());
    sections.push(
        "These memories persist across sessions. Use them as context but verify against current state before acting.".to_string(),
    );

    let mut total_len: usize = sections.iter().map(|s| s.len()).sum();

    for memory_type in &[
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ] {
        let typed: Vec<_> = memories
            .iter()
            .filter(|m| m.memory_type == *memory_type)
            .take(MAX_MEMORIES_PER_TYPE)
            .collect();
        if typed.is_empty() {
            continue;
        }

        let header = match memory_type {
            MemoryType::User => "## User Context",
            MemoryType::Feedback => "## Feedback & Guidance",
            MemoryType::Project => "## Project Context",
            MemoryType::Reference => "## External References",
        };
        sections.push(header.to_string());
        total_len += header.len();

        for m in typed {
            let content = truncate_utf8(&m.content, MAX_MEMORY_CONTENT_CHARS);
            let line = format!("- **{}**: {}", m.title, content);
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

    sections.push("[/PERSISTENT MEMORY]".to_string());
    sections.join("\n")
}

pub(super) fn format_memory_kind(memory_type: MemoryType) -> &'static str {
    match memory_type {
        MemoryType::User => "User",
        MemoryType::Feedback => "Feedback",
        MemoryType::Project => "Project",
        MemoryType::Reference => "Reference",
    }
}
