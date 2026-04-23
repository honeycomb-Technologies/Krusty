use std::collections::HashSet;
use std::path::Path;

use tracing::warn;

use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::{
    is_current_snapshot, refresh_current_snapshot, AutonomousTaskStore, MemoryStore, Report,
    ReportStore, TaskStatus,
};

use super::memory::{format_memory_kind, MAX_MEMORY_CONTENT_CHARS};
use super::{open_context_database, truncate_utf8};

/// Maximum number of memories included in the Mako-specific knowledge block.
const MAX_MAKO_MEMORY_ITEMS: usize = 8;
/// Maximum number of reports included in the Mako-specific knowledge block.
const MAX_MAKO_REPORT_ITEMS: usize = 5;
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

    let mut lines = vec![if selection.has_relevant_matches {
        "[RELEVANT REPORTS]".to_string()
    } else {
        "[RECENT REPORTS]".to_string()
    }];
    for report in selection.reports {
        let summary = truncate_utf8(&report.summary, 200);
        lines.push(format!(
            "- \"{}\" ({}): {}",
            report.title, report.created_at, summary
        ));
    }
    lines.push("Use `ReadReport` tool to access full content.".to_string());

    lines.join("\n")
}

pub(super) fn build_mako_knowledge_context(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    session_id: &str,
    conversation: &[ModelMessage],
) -> String {
    if let Err(error) = refresh_current_snapshot(db_path, project_dir, user_id) {
        warn!(project_dir = ?project_dir, error = %error, "Failed to refresh Mako snapshot context");
    }

    let mut memories =
        if let Some(memory_db) = open_context_database(db_path, "building mako memory context") {
            let memory_store = MemoryStore::new(memory_db);
            memory_store.list(project_dir, user_id)
        } else {
            Vec::new()
        };
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
        open_context_database(db_path, "building mako report context")
    {
        let report_store = ReportStore::new(report_db);
        match report_store.list_reports(project_dir) {
            Ok(reports) => reports,
            Err(error) => {
                warn!(project_dir = ?project_dir, error = %error, "Failed to load Mako reports for context");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    if memories.is_empty() && reports.is_empty() {
        return String::new();
    }

    let report_selection = select_reports_for_context(
        &reports,
        &build_report_relevance_terms(conversation, db_path, Some(session_id)),
        MAX_MAKO_REPORT_ITEMS,
    );

    let current_snapshot = memories.iter().find(|memory| is_current_snapshot(memory));
    let carry_forward_memories = memories
        .iter()
        .filter(|memory| !is_current_snapshot(memory))
        .collect::<Vec<_>>();
    let mut sections = vec![
        "[MAKO KNOWLEDGE]".to_string(),
        "Carry forward durable facts from memory and recent outcomes from reports. Prefer promoted memory for stable decisions, and use `ReadReport` when full report detail matters.".to_string(),
    ];

    if let Some(snapshot) = current_snapshot {
        sections.push("## Current Snapshot".to_string());
        sections.push(snapshot.content.clone());
    }

    if !carry_forward_memories.is_empty() {
        sections.push("## Carry Forward".to_string());
        for memory in carry_forward_memories.iter().take(MAX_MAKO_MEMORY_ITEMS) {
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
                memory.title,
                content
            ));
        }
    }

    if !report_selection.reports.is_empty() {
        sections.push(if report_selection.has_relevant_matches {
            "## Relevant Reports".to_string()
        } else {
            "## Recent Reports".to_string()
        });
        for report in report_selection.reports {
            let summary = truncate_utf8(&report.summary, 200);
            sections.push(format!(
                "- \"{}\" ({}): {}",
                report.title, report.created_at, summary
            ));
        }
    }

    sections.push("[/MAKO KNOWLEDGE]".to_string());
    sections.join("\n")
}

struct ReportContextSelection<'a> {
    reports: Vec<&'a Report>,
    has_relevant_matches: bool,
}

fn select_reports_for_context<'a>(
    reports: &'a [Report],
    query_terms: &[String],
    limit: usize,
) -> ReportContextSelection<'a> {
    if reports.is_empty() || limit == 0 {
        return ReportContextSelection {
            reports: Vec::new(),
            has_relevant_matches: false,
        };
    }

    let mut scored = reports
        .iter()
        .enumerate()
        .map(|(index, report)| (index, score_report_for_context(report, query_terms), report))
        .collect::<Vec<_>>();

    let has_relevant_matches = scored.iter().any(|(_, score, _)| *score > 0);
    if !has_relevant_matches {
        return ReportContextSelection {
            reports: reports.iter().take(limit).collect(),
            has_relevant_matches: false,
        };
    }

    scored.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut selected = Vec::new();
    let mut selected_ids = HashSet::new();
    for (_, _, report) in scored.iter().filter(|(_, score, _)| *score > 0) {
        if selected.len() >= limit {
            break;
        }
        if selected_ids.insert(report.id.as_str()) {
            selected.push(*report);
        }
    }

    if selected.len() < limit {
        for report in reports {
            if selected.len() >= limit {
                break;
            }
            if selected_ids.insert(report.id.as_str()) {
                selected.push(report);
            }
        }
    }

    ReportContextSelection {
        reports: selected,
        has_relevant_matches: true,
    }
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
    conversation.iter().rev().find_map(|message| {
        if message.role != Role::User {
            return None;
        }
        first_text_content(&message.content)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn first_text_content(content: &[Content]) -> Option<&str> {
    content.iter().find_map(|item| {
        if let Content::Text { text } = item {
            Some(text.as_str())
        } else {
            None
        }
    })
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
