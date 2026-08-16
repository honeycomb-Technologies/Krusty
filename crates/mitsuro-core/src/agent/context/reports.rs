use std::collections::HashSet;
use std::path::Path;

use tracing::warn;

use crate::agent::context_ledger::ContextLedger;
use crate::ai::types::ModelMessage;
use crate::storage::{
    is_compaction_flush_memory, is_current_snapshot, refresh_current_snapshot, AutonomousTaskStore,
    MemoryNamespace, MemoryStore, Report, ReportStore, TaskStatus,
};

use super::memory::{format_memory_kind, MAX_MEMORY_CONTENT_CHARS};
use super::{open_context_database, truncate_utf8, truncate_utf8_bytes};

/// Maximum number of memories included in the Hive-specific knowledge block.
const MAX_HIVE_MEMORY_ITEMS: usize = 8;
/// Maximum number of reports included in the Hive-specific knowledge block.
const MAX_HIVE_REPORT_ITEMS: usize = 5;
/// Snapshot and aggregate limits keep long-lived Hive state from becoming an
/// unbounded system-prompt layer.
const MAX_HIVE_SNAPSHOT_BYTES: usize = 8 * 1024;
const MAX_HIVE_KNOWLEDGE_BYTES: usize = 16 * 1024;
const MAX_KNOWLEDGE_TITLE_CHARS: usize = 160;
/// Maximum number of query terms used when ranking relevant reports.
const MAX_REPORT_QUERY_TERMS: usize = 10;
/// Maximum number of keywords extracted from one text signal.
const MAX_REPORT_SIGNAL_KEYWORDS: usize = 6;
/// Common low-signal terms that should not drive report relevance.
const REPORT_QUERY_STOPWORDS: &[&str] = &[
    "about", "after", "again", "agent", "always", "because", "before", "being", "between", "could",
    "every", "finish", "first", "found", "from", "have", "into", "just", "make", "more", "need",
    "over", "please", "report", "should", "some", "that", "their", "them", "there", "these",
    "they", "this", "through", "what", "when", "with", "work",
];

/// Build context for recent reports in this project.
pub(super) fn build_report_context(
    db_path: &Path,
    project_dir: Option<&str>,
    conversation: &[ModelMessage],
) -> String {
    let Some(db) = open_context_database(db_path, "building report context") else {
        return String::new();
    };
    let store = ReportStore::new(db);
    let reports = match store.list_reports(project_dir) {
        Ok(r) => r,
        Err(error) => {
            warn!(project_dir = ?project_dir, error = %error, "Failed to load reports for context");
            return String::new();
        }
    };
    if reports.is_empty() {
        return String::new();
    }

    let selection = select_reports_for_context(
        &reports,
        &build_report_relevance_terms(conversation, db_path, None),
        5,
    );
    if selection.reports.is_empty() {
        return String::new();
    }

    let mut lines = vec!["[RELEVANT REPORTS]".to_string()];
    for report in selection.reports {
        let summary = truncate_utf8(&report.summary, 200);
        lines.push(format!(
            "- id={} | \"{}\" ({}): {}",
            report.id,
            truncate_utf8(&report.title, MAX_KNOWLEDGE_TITLE_CHARS),
            report.created_at,
            summary
        ));
    }
    lines.push(
        "Read full content with `tool_search(action: \"execute\", tool: \"report\", arguments: {\"action\": \"read\", \"report_id\": \"...\"})`."
            .to_string(),
    );

    lines.join("\n")
}

pub(super) fn build_hive_knowledge_context(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    hive_memory_namespace: Option<&str>,
    session_id: &str,
    conversation: &[ModelMessage],
) -> String {
    // The materialized snapshot is owner/project scoped rather than crew or
    // Worker scoped. Only the primary Hive presence may consume it; named crew
    // members and Workers receive their own explicit memory namespace below.
    let generated_snapshot = if hive_memory_namespace.is_none() {
        match refresh_current_snapshot(db_path, project_dir, user_id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warn!(project_dir = ?project_dir, error = %error, "Failed to refresh Hive snapshot context");
                None
            }
        }
    } else {
        None
    };

    let mut memories =
        if let Some(memory_db) = open_context_database(db_path, "building hive memory context") {
            let memory_store = MemoryStore::new(memory_db);
            memory_store.list_for_exact_owner(project_dir, user_id)
        } else {
            Vec::new()
        };
    // A named presence (legacy crew slug or a Worker's memory namespace) sees
    // Shared plus exactly its own namespace; the primary companion sees
    // Shared plus the Hive namespace.
    memories.retain(|memory| match hive_memory_namespace {
        Some(namespace_id) => {
            memory.namespace == MemoryNamespace::Shared
                || (memory.namespace == MemoryNamespace::Crew
                    && memory.namespace_id.as_deref() == Some(namespace_id))
        }
        None => {
            memory.namespace == MemoryNamespace::Shared || memory.namespace == MemoryNamespace::Hive
        }
    });
    if let Some(project_dir) = project_dir {
        memories.sort_by(|left, right| {
            let left_project_match = left.project_dir.as_deref() == Some(project_dir);
            let right_project_match = right.project_dir.as_deref() == Some(project_dir);

            right_project_match
                .cmp(&left_project_match)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
    }

    let reports = if let Some(report_db) =
        open_context_database(db_path, "building hive report context")
    {
        let report_store = ReportStore::new(report_db);
        match report_store.list_reports_for_exact_owner(project_dir, user_id) {
            Ok(reports) => reports,
            Err(error) => {
                warn!(project_dir = ?project_dir, error = %error, "Failed to load Hive reports for context");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    if memories.is_empty() && reports.is_empty() && generated_snapshot.is_none() {
        return String::new();
    }

    let report_selection = select_reports_for_context(
        &reports,
        &build_report_relevance_terms(conversation, db_path, Some(session_id)),
        MAX_HIVE_REPORT_ITEMS,
    );

    // `knowledge_snapshots` is the canonical generated-state store. The
    // memory lookup remains only as a compatibility fallback for pre-migration
    // snapshot rows and is never preferred over the exact-owner materialized
    // snapshot returned above.
    let legacy_current_snapshot = memories.iter().find(|memory| is_current_snapshot(memory));
    let carry_forward_memories = memories
        .iter()
        .filter(|memory| !is_current_snapshot(memory))
        .filter(|memory| !is_compaction_flush_memory(memory))
        .collect::<Vec<_>>();
    let mut sections = vec![
        "[HIVE KNOWLEDGE]".to_string(),
        "Carry forward durable facts from memory and recent outcomes from reports. Prefer promoted memory for stable decisions; use deferred `report` execution through `tool_search` when full detail matters.".to_string(),
    ];

    if let Some(snapshot_content) = generated_snapshot
        .as_ref()
        .map(|snapshot| snapshot.content.as_str())
        .or_else(|| legacy_current_snapshot.map(|snapshot| snapshot.content.as_str()))
    {
        sections.push("## Current Snapshot".to_string());
        sections.push(truncate_utf8_bytes(
            snapshot_content,
            MAX_HIVE_SNAPSHOT_BYTES,
        ));
    }

    if !carry_forward_memories.is_empty() {
        sections.push("## Carry Forward".to_string());
        for memory in carry_forward_memories.iter().take(MAX_HIVE_MEMORY_ITEMS) {
            let scope = if project_dir.is_some() && memory.project_dir.as_deref() == project_dir {
                "project"
            } else {
                "global"
            };
            let content = truncate_utf8(&memory.content, MAX_MEMORY_CONTENT_CHARS);
            sections.push(format!(
                "- [{} | {}] {}: {}",
                format_memory_kind(memory.memory_type),
                scope,
                truncate_utf8(&memory.title, MAX_KNOWLEDGE_TITLE_CHARS),
                content
            ));
        }
    }

    if !report_selection.reports.is_empty() {
        sections.push("## Relevant Reports".to_string());
        for report in report_selection.reports {
            let summary = truncate_utf8(&report.summary, 200);
            sections.push(format!(
                "- id={} | \"{}\" ({}): {}",
                report.id,
                truncate_utf8(&report.title, MAX_KNOWLEDGE_TITLE_CHARS),
                report.created_at,
                summary
            ));
        }
    }

    sections.push("[/HIVE KNOWLEDGE]".to_string());
    let context = sections.join("\n");
    if context.len() <= MAX_HIVE_KNOWLEDGE_BYTES {
        return context;
    }

    const END_MARKER: &str = "\n[HIVE KNOWLEDGE TRUNCATED AT REQUEST BUDGET]\n[/HIVE KNOWLEDGE]";
    let mut bounded = truncate_utf8_bytes(
        &context,
        MAX_HIVE_KNOWLEDGE_BYTES.saturating_sub(END_MARKER.len()),
    );
    bounded.push_str(END_MARKER);
    bounded
}

struct ReportContextSelection<'a> {
    reports: Vec<&'a Report>,
}

fn select_reports_for_context<'a>(
    reports: &'a [Report],
    query_terms: &[String],
    limit: usize,
) -> ReportContextSelection<'a> {
    if reports.is_empty() || limit == 0 {
        return ReportContextSelection {
            reports: Vec::new(),
        };
    }

    let mut scored = reports
        .iter()
        .enumerate()
        .map(|(index, report)| (index, score_report_for_context(report, query_terms), report))
        .collect::<Vec<_>>();

    if !scored.iter().any(|(_, score, _)| *score > 0) {
        return ReportContextSelection {
            reports: Vec::new(),
        };
    }

    scored.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut selected = Vec::new();
    for (_, _, report) in scored.iter().filter(|(_, score, _)| *score > 0) {
        if selected.len() >= limit {
            break;
        }
        selected.push(*report);
    }

    ReportContextSelection { reports: selected }
}

fn score_report_for_context(report: &Report, query_terms: &[String]) -> usize {
    if query_terms.is_empty() {
        return 0;
    }

    let title = report.title.to_lowercase();
    let summary = report.summary.to_lowercase();
    let tags = report
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<Vec<_>>();
    let sources = report
        .sources
        .iter()
        .map(|source| source.to_lowercase())
        .collect::<Vec<_>>();

    query_terms.iter().fold(0, |score, term| {
        let normalized = term.trim().to_lowercase();
        if normalized.is_empty() {
            return score;
        }

        let mut term_score = 0;
        if title.contains(&normalized) {
            term_score += 6;
        }
        if summary.contains(&normalized) {
            term_score += 4;
        }
        if tags.iter().any(|tag| tag.contains(&normalized)) {
            term_score += 5;
        }
        if sources.iter().any(|source| source.contains(&normalized)) {
            term_score += 3;
        }

        score + term_score
    })
}

fn build_report_relevance_terms(
    conversation: &[ModelMessage],
    db_path: &Path,
    session_id: Option<&str>,
) -> Vec<String> {
    let mut terms = Vec::new();

    if let Some(objective) = latest_user_objective(conversation) {
        terms.push(objective.clone());
        terms.extend(extract_report_keywords(&objective));
    }

    if let Some(session_id) = session_id {
        terms.extend(load_active_task_subjects(db_path, session_id));
    }

    let mut seen = HashSet::new();
    terms.retain(|term| {
        let normalized = term.trim().to_lowercase();
        !normalized.is_empty() && seen.insert(normalized)
    });
    terms.truncate(MAX_REPORT_QUERY_TERMS);
    terms
}

fn latest_user_objective(conversation: &[ModelMessage]) -> Option<String> {
    ContextLedger::from_conversation(conversation).latest_user_objective
}

fn extract_report_keywords(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|part| {
            let normalized = part.trim().to_lowercase();
            if normalized.len() < 4 || REPORT_QUERY_STOPWORDS.contains(&normalized.as_str()) {
                return None;
            }
            if seen.insert(normalized.clone()) {
                Some(normalized)
            } else {
                None
            }
        })
        .take(MAX_REPORT_SIGNAL_KEYWORDS)
        .collect()
}

fn load_active_task_subjects(db_path: &Path, session_id: &str) -> Vec<String> {
    let Some(db) = open_context_database(db_path, "building report relevance task context") else {
        return Vec::new();
    };
    let store = AutonomousTaskStore::new(db);
    let tasks = match store.list_tasks(session_id) {
        Ok(tasks) => tasks,
        Err(error) => {
            warn!(session_id, error = %error, "Failed to load autonomous tasks for report relevance");
            return Vec::new();
        }
    };

    let mut subjects = Vec::new();
    for task in tasks
        .into_iter()
        .filter(|task| matches!(task.status, TaskStatus::Pending | TaskStatus::InProgress))
        .take(3)
    {
        if !task.subject.trim().is_empty() {
            subjects.push(task.subject.clone());
            subjects.extend(extract_report_keywords(&task.subject));
        }
    }
    subjects
}
