//! Context injection for the agentic loop.
//!
//! Builds plan, skills, and project context strings that get injected as
//! system messages at the head of the conversation before each AI call.
//! This ensures the AI is always aware of the active plan, available skills,
//! and project-specific instructions.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use tokio::sync::RwLock;
use tracing::warn;

use crate::ai::types::{Content, ModelMessage, Role};
use crate::paths;
use crate::plan::PlanManager;
use crate::skills::SkillsManager;
use crate::storage::{
    is_current_snapshot, refresh_current_snapshot, AutonomousTaskStore, Database,
    DelegatedRunStore, MakoHomeProfile, MemoryStore, MemoryType, ProjectSettings, ReportStore,
    TaskStatus, WorkMode,
};

/// Instruction files to search for in the working directory (priority order).
const PROJECT_FILES: &[&str] = &[
    "KRAB.md",
    "krab.md",
    "AGENTS.md",
    "agents.md",
    "CLAUDE.md",
    "claude.md",
    ".cursorrules",
    ".windsurfrules",
    ".clinerules",
    ".github/copilot-instructions.md",
    "JULES.md",
    "gemini.md",
];
const MAKO_FILES: &[&str] = &["MAKO.md", "mako.md"];

/// Build a conversation clone with context system messages prepended.
///
/// Injects plan, skills, and project context in the same order as the TUI:
/// project → plan → skills → original conversation.
pub fn inject_context(
    conversation: &[ModelMessage],
    db_path: &Path,
    session_id: &str,
    working_dir: &Path,
    project_dir: Option<&Path>,
    work_mode: WorkMode,
    skills_manager: &RwLock<SkillsManager>,
    model_id: Option<&str>,
    session_type: Option<&str>,
) -> Vec<ModelMessage> {
    let is_chat = session_type == Some("chat");

    // Chat sessions get minimal context — no workspace, tools, plans, or project data.
    if is_chat {
        let mut injected = Vec::with_capacity(conversation.len() + 2);
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: "You are Krusty, a helpful conversational assistant. This is a chat session — you are having a natural conversation with the user.\n\nIMPORTANT: You do NOT have access to any tools in this session. Do not mention, list, or describe any tools. You cannot read files, run commands, or edit code. If the user asks about tools, explain that this is a chat-only session and suggest they switch to Code mode for coding tasks.\n\nBe friendly, helpful, and conversational.".to_string(),
            }],
        });
        let memory_ctx = build_memory_context(db_path, None, None);
        if !memory_ctx.is_empty() {
            injected.push(ModelMessage {
                role: Role::System,
                content: vec![Content::Text { text: memory_ctx }],
            });
        }
        injected.extend_from_slice(conversation);
        return injected;
    }

    let workspace_ctx = build_workspace_context(working_dir, project_dir);
    let env_ctx = build_environment_context(working_dir, model_id);
    let context_project_dir = project_dir.map(|p| p.to_string_lossy().to_string());
    let is_mako = session_type == Some("mako");
    let memory_ctx = if is_mako {
        String::new()
    } else {
        build_memory_context(
            db_path,
            context_project_dir.as_deref(),
            None, // user_id — single-tenant for now
        )
    };
    let plan_ctx = build_plan_context(db_path, session_id, work_mode);
    let delegated_ctx = build_delegated_context(db_path, session_id);
    let task_ctx = build_autonomous_task_context(db_path, session_id);
    let report_ctx = if is_mako {
        String::new()
    } else {
        build_report_context(db_path, context_project_dir.as_deref(), conversation)
    };
    let mako_knowledge_ctx = if is_mako {
        build_mako_knowledge_context(
            db_path,
            context_project_dir.as_deref(),
            None,
            session_id,
            conversation,
        )
    } else {
        String::new()
    };
    let skills_ctx = build_skills_context(skills_manager, project_dir.is_some());
    let project_ctx = project_dir.map(build_project_context).unwrap_or_default();
    let mako_ctx_sections = if is_mako {
        build_mako_context_sections(project_dir.unwrap_or(working_dir))
    } else {
        Vec::new()
    };
    let project_settings = project_dir.map(ProjectSettings::load).unwrap_or_default();

    let mut injected = Vec::with_capacity(conversation.len() + 7);

    if !workspace_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: workspace_ctx,
            }],
        });
    }
    if !env_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: env_ctx }],
        });
    }
    if !memory_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: memory_ctx }],
        });
    }
    if !mako_knowledge_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: mako_knowledge_ctx,
            }],
        });
    }
    if !project_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: project_ctx }],
        });
    }
    for text in mako_ctx_sections {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text }],
        });
    }
    if let Some(ref append) = project_settings.system_prompt_append {
        if !append.is_empty() {
            injected.push(ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: format!("[PROJECT SETTINGS]\n{}", append),
                }],
            });
        }
    }
    if !plan_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: plan_ctx }],
        });
    }
    if !delegated_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: delegated_ctx,
            }],
        });
    }
    if !task_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: task_ctx }],
        });
    }
    if !report_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: report_ctx }],
        });
    }
    if !skills_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: skills_ctx }],
        });
    }

    injected.extend_from_slice(conversation);
    injected
}

/// Maximum number of memories injected per type (most recent first).
const MAX_MEMORIES_PER_TYPE: usize = 15;
/// Maximum character length for a single memory's content in the injection.
const MAX_MEMORY_CONTENT_CHARS: usize = 300;
/// Approximate upper bound on total memory context output size.
const MAX_MEMORY_CONTEXT_BYTES: usize = 8 * 1024;
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

/// Truncate a string to at most `max_chars` characters on a valid UTF-8
/// boundary, appending "..." when truncation occurs.
fn truncate_utf8(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}...", truncated)
}

fn open_context_database(db_path: &Path, context: &'static str) -> Option<Database> {
    match Database::new(db_path) {
        Ok(db) => Some(db),
        Err(error) => {
            warn!(context, db_path = %db_path.display(), error = %error, "Failed to open context database");
            None
        }
    }
}

fn plan_mode_default_context() -> String {
    "[PLAN MODE ACTIVE]\n\n\
     You are in PLAN MODE. The user wants a plan before implementing.\n\
     - You can READ files, search code, and explore the codebase\n\
     - You CANNOT write, edit, or create files\n\
     - Use the AskUserQuestion tool for clarifications\n\n\
     When creating a plan, use this format:\n\
     ```\n\
     # Plan: [Title]\n\n\
     ## Phase 1: [Phase Name]\n\n\
     - [ ] Task description\n\
       > Context: Implementation details\n\
     ```"
    .to_string()
}

/// Build persistent memory context from the agent memory store.
///
/// Returns an empty string when no memories exist, keeping the system
/// prompt lean for fresh sessions.  Caps per-type count, individual
/// content length, and total output size to prevent memory injection
/// from consuming too much context budget.
fn build_memory_context(
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
            total_len += line.len() + 1; // +1 for newline join
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

fn build_delegated_context(db_path: &Path, session_id: &str) -> String {
    let Some(db) = open_context_database(db_path, "building delegated context") else {
        return String::new();
    };
    let store = DelegatedRunStore::new(db);
    let recent = match store.list_runs_for_session(session_id, 3) {
        Ok(runs) => runs,
        Err(error) => {
            warn!(session_id = %session_id, error = %error, "Failed to load delegated runs for context");
            return String::new();
        }
    };
    if recent.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        "[RECENT DELEGATED RUNS]".to_string(),
        String::new(),
        "Recent delegated investigations exist for this session.".to_string(),
        "- If the user explicitly asks to use `explore` or continue/refine a prior architectural audit, prefer calling `explore` again over manual `glob`/`list`/`read` probing.".to_string(),
        "- If a matching delegated scope already exists below, let the `explore` tool resume or deepen that work instead of remapping the directory from scratch.".to_string(),
        "- Only fall back to manual probing if the user forbids delegation or the explore result was clearly unusable.".to_string(),
        String::new(),
        "Most recent delegated runs:".to_string(),
    ];

    for run in recent {
        let scopes = if run.target_scope.is_empty() {
            "(no scope)".to_string()
        } else {
            run.target_scope
                .iter()
                .map(|scope| scope.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let role = match run.role {
            crate::storage::DelegatedRunRole::Explore => "explore",
            crate::storage::DelegatedRunRole::Build => "build",
            crate::storage::DelegatedRunRole::Planner => "planner",
            crate::storage::DelegatedRunRole::Verifier => "verifier",
        };
        let review = run
            .human_review
            .as_deref()
            .unwrap_or("No finalized review was recorded.")
            .lines()
            .next()
            .unwrap_or("No finalized review was recorded.");
        lines.push(format!(
            "- {} run {} on [{}]: stage={:?}, resumable={}, semantic_review=\"{}\"",
            role, run.delegated_run_id, scopes, run.stage, run.resumable, review
        ));
    }

    lines.join("\n")
}

/// Build context for autonomous tasks in this session.
fn build_autonomous_task_context(db_path: &Path, session_id: &str) -> String {
    let Some(db) = open_context_database(db_path, "building autonomous task context") else {
        return String::new();
    };
    let store = AutonomousTaskStore::new(db);
    let tasks = match store.list_tasks(session_id) {
        Ok(t) => t,
        Err(error) => {
            warn!(session_id = %session_id, error = %error, "Failed to load autonomous tasks for context");
            return String::new();
        }
    };
    if tasks.is_empty() {
        return String::new();
    }

    let mut lines = vec!["[AUTONOMOUS TASKS]".to_string()];

    let pending: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending)
        .collect();
    let in_progress: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::InProgress)
        .collect();
    let completed: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .collect();

    if !pending.is_empty() {
        lines.push("Pending:".to_string());
        for t in &pending {
            lines.push(format!("  - {}: {}", t.id, t.subject));
        }
    }
    if !in_progress.is_empty() {
        lines.push("In Progress:".to_string());
        for t in &in_progress {
            let owner = t
                .owner
                .as_deref()
                .map(|o| format!(" (owner: {})", o))
                .unwrap_or_default();
            lines.push(format!("  - {}: {}{}", t.id, t.subject, owner));
        }
    }
    if !completed.is_empty() {
        lines.push("Completed:".to_string());
        for t in &completed {
            lines.push(format!("  - {}: {}", t.id, t.subject));
        }
    }

    lines.join("\n")
}

/// Build context for recent reports in this project.
fn build_report_context(
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

fn format_memory_kind(memory_type: MemoryType) -> &'static str {
    match memory_type {
        MemoryType::User => "User",
        MemoryType::Feedback => "Feedback",
        MemoryType::Project => "Project",
        MemoryType::Reference => "Reference",
    }
}

fn build_mako_knowledge_context(
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
    reports: Vec<&'a crate::storage::Report>,
    has_relevant_matches: bool,
}

fn select_reports_for_context<'a>(
    reports: &'a [crate::storage::Report],
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

fn score_report_for_context(report: &crate::storage::Report, query_terms: &[String]) -> usize {
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

/// Build combined workspace + project context for a subagent system prompt.
pub fn build_subagent_project_context(working_dir: &Path, project_dir: Option<&Path>) -> String {
    let workspace = build_workspace_context(working_dir, project_dir);
    let project = project_dir.map(build_project_context).unwrap_or_default();
    let project_settings = project_dir.map(ProjectSettings::load).unwrap_or_default();

    let mut ctx = String::new();
    if !workspace.is_empty() {
        ctx.push_str(&workspace);
    }
    if !project.is_empty() {
        if !ctx.is_empty() {
            ctx.push_str("\n\n");
        }
        ctx.push_str(&project);
    }
    if let Some(ref append) = project_settings.system_prompt_append {
        if !append.is_empty() {
            if !ctx.is_empty() {
                ctx.push_str("\n\n");
            }
            ctx.push_str(&format!("[PROJECT SETTINGS]\n{}", append));
        }
    }
    ctx
}

fn build_workspace_context(working_dir: &Path, project_dir: Option<&Path>) -> String {
    let execution_dir = working_dir.display();

    if let Some(project_dir) = project_dir {
        return format!(
            "[WORKSPACE MODE: PROJECT]\n\n\
             Execution directory: {}\n\
             Project directory: {}\n\
             - Treat the project directory above as the canonical repository root for this session\n\
             - Prefer absolute paths rooted in that project directory when referring to files\n\
             - Do not invent alternate workspace roots or mirror paths if tools already revealed the real filesystem layout",
            execution_dir,
            project_dir.display()
        );
    }

    format!(
        "[WORKSPACE MODE: NEUTRAL]\n\n\
     Execution directory: {}\n\
     Project directory: none selected\n\n\
     No project directory is currently selected for this session.\n\
     - Do not assume the user wants help with any repository just because files are accessible\n\
     - Do not describe yourself as working on a named project unless the user explicitly selects or creates one\n\
     - Use the execution directory above when running tools, and treat paths returned by tools as canonical\n\
     - Do not invent alternate workspace roots, mount points, or mirrored directories when tools already returned absolute paths\n\
     - Read-only system inspection, brainstorming, and machine-level tasks are valid in this mode\n\
     - If the user chooses or creates a project directory, call `set_workspace_context` so future turns pivot into project-aware behavior",
        execution_dir
    )
}

/// Build plan context from the active plan for this session.
pub fn build_plan_context(db_path: &Path, session_id: &str, work_mode: WorkMode) -> String {
    let plan_manager = match PlanManager::new(db_path.to_path_buf()) {
        Ok(pm) => pm,
        Err(error) => {
            warn!(session_id = %session_id, db_path = %db_path.display(), error = %error, "Failed to open plan manager for context");
            return if work_mode == WorkMode::Plan {
                plan_mode_default_context()
            } else {
                String::new()
            };
        }
    };

    let plan = match plan_manager.get_active_plan(session_id) {
        Ok(Some(p)) => p,
        Err(error) => {
            warn!(session_id = %session_id, error = %error, "Failed to load active plan for context");
            return if work_mode == WorkMode::Plan {
                plan_mode_default_context()
            } else {
                String::new()
            };
        }
        _ => {
            return if work_mode == WorkMode::Plan {
                plan_mode_default_context()
            } else {
                String::new()
            };
        }
    };

    let (completed, total) = plan.progress();
    let markdown = plan.to_context();

    if work_mode == WorkMode::Plan {
        let title = plan.title.replace(['`', '"'], "'");
        format!(
            "[PLAN MODE ACTIVE - Plan: \"{}\"]\n\n\
             Progress: {}/{} tasks completed\n\n\
             ## Current Plan\n\n{}\n\n---\n\n\
             In plan mode you can READ but CANNOT write/edit files.",
            title, completed, total, markdown
        )
    } else {
        let ready_tasks = plan.get_ready_tasks();
        let blocked_tasks = plan.get_blocked_tasks();

        let ready_list = if ready_tasks.is_empty() {
            "  (none)".to_string()
        } else {
            ready_tasks
                .iter()
                .map(|t| format!("  - Task {}: {}", t.id, t.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let blocked_list = if blocked_tasks.is_empty() {
            "  (none)".to_string()
        } else {
            blocked_tasks
                .iter()
                .map(|t| {
                    format!(
                        "  - Task {}: {} (waiting on: {})",
                        t.id,
                        t.description,
                        t.blocked_by.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let title = plan.title.replace(['`', '"'], "'");
        format!(
            "[ACTIVE PLAN - \"{}\"]\n\n\
             Progress: {}/{} tasks completed\n\n\
             ## Ready to Work\n{}\n\n\
             ## Blocked Tasks\n{}\n\n\
             ## Current Plan\n\n{}\n\n---\n\n\
             ## Task Workflow Protocol\n\n\
             1. PICK ONE ready task\n\
             2. `task_start(task_id)` - marks as in-progress\n\
             3. DO THE WORK\n\
             4. `task_complete(task_id, result)` - with specific result\n\
             5. Move to next task\n\n\
             Rules: One task at a time. Always start before completing. \
             Use `add_subtask` for complex tasks. Check Ready list for unblocked tasks.",
            title, completed, total, ready_list, blocked_list, markdown
        )
    }
}

/// Build skills context listing available skills.
pub fn build_skills_context(
    skills_manager: &RwLock<SkillsManager>,
    include_project_skills: bool,
) -> String {
    let mut guard = match skills_manager.try_write() {
        Ok(g) => g,
        Err(_) => {
            warn!(
                include_project_skills,
                "Skipping skills context because the skills manager is busy"
            );
            return String::new();
        }
    };

    let skills = if include_project_skills {
        guard.list_skills()
    } else {
        guard.list_global_skills()
    };
    if skills.is_empty() {
        return String::new();
    }

    let mut context =
        String::from("[AVAILABLE SKILLS]\n\nUse the `skill` tool to invoke a skill.\n\n");
    for info in skills {
        context.push_str(&format!("- **{}**: {}\n", info.name, info.description));
        if !info.tags.is_empty() {
            context.push_str(&format!("  Tags: {}\n", info.tags.join(", ")));
        }
    }
    context.push_str("\nTo use: `skill(skill: \"name\")`\n");
    context
}

/// Build environment context with runtime information.
///
/// Gathers working directory, git status, platform, shell, date, and model
/// information. Git commands that fail are skipped with warnings.
fn run_git_context_command(working_dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    match Command::new("git")
        .args(args)
        .current_dir(working_dir)
        .output()
    {
        Ok(output) if output.status.success() => Some(output),
        Ok(output) => {
            warn!(
                working_dir = %working_dir.display(),
                ?args,
                status = ?output.status,
                "Git probe failed while building environment context"
            );
            None
        }
        Err(error) => {
            warn!(
                working_dir = %working_dir.display(),
                ?args,
                error = %error,
                "Failed to run git probe while building environment context"
            );
            None
        }
    }
}

fn summarize_git_status(status_text: &str) -> Option<String> {
    let mut modified = 0usize;
    let mut staged = 0usize;
    let mut untracked = 0usize;

    for line in status_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("??") {
            untracked += 1;
        } else if trimmed.starts_with('A') {
            staged += 1;
        } else if trimmed.starts_with('M') || trimmed.contains('M') {
            modified += 1;
        }
    }

    let mut parts = Vec::new();
    if modified > 0 {
        parts.push(format!("{} modified", modified));
    }
    if staged > 0 {
        parts.push(format!("{} staged", staged));
    }
    if untracked > 0 {
        parts.push(format!("{} untracked", untracked));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn build_environment_context(working_dir: &Path, model_id: Option<&str>) -> String {
    let mut lines = vec![
        "[ENVIRONMENT]".to_string(),
        format!("Working directory: {}", working_dir.display()),
    ];

    let is_git_repo = working_dir.join(".git").exists();
    lines.push(format!(
        "Git repository: {}",
        if is_git_repo { "yes" } else { "no" }
    ));

    if is_git_repo {
        if let Some(output) =
            run_git_context_command(working_dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() {
                lines.push(format!("Git branch: {}", branch));
            }
        }

        if let Some(output) = run_git_context_command(working_dir, &["status", "--short"]) {
            let status_text = String::from_utf8_lossy(&output.stdout);
            if let Some(summary) = summarize_git_status(&status_text) {
                lines.push(format!("Git status: {}", summary));
            }
        }
    }

    lines.push(format!("Platform: {}", std::env::consts::OS));

    if let Ok(shell) = std::env::var("SHELL") {
        let shell_name = shell.rsplit('/').next().unwrap_or(&shell);
        lines.push(format!("Shell: {}", shell_name));
    }

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    lines.push(format!("Date: {}", date));

    if let Some(model) = model_id {
        lines.push(format!("Model: {}", model));
    }

    lines.join("\n")
}

/// Build project context from instruction files in the working directory.
///
/// Searches from the project root down to the working directory and
/// concatenates the closest instruction file from each directory.
pub fn build_project_context(working_dir: &Path) -> String {
    let instruction_files = discover_instruction_files(working_dir);
    if instruction_files.is_empty() {
        return String::new();
    }

    let mut sections = Vec::new();
    for path in instruction_files {
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                warn!(path = %path.display(), error = %error, "Failed to read project instruction file");
                continue;
            }
        };
        if content.trim().is_empty() {
            continue;
        }

        let label = path
            .strip_prefix(working_dir)
            .ok()
            .map(|p| p.display().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| path.display().to_string());

        sections.push(format!(
            "[PROJECT INSTRUCTIONS - {}]\n\n{}\n\n[END PROJECT INSTRUCTIONS]",
            label, content
        ));
    }

    sections.join("\n\n")
}

fn build_mako_context_sections(project_root: &Path) -> Vec<String> {
    let mako_home = paths::mako_dir();
    build_mako_context_sections_with_home(project_root, &mako_home)
}

fn build_mako_context_sections_with_home(project_root: &Path, mako_home: &Path) -> Vec<String> {
    let mut sections = MakoHomeProfile::load_from(mako_home)
        .context_layers()
        .into_iter()
        .map(|layer| {
            format!(
                "[MAKO {} - {}]\n\n{}\n\n[END MAKO {}]",
                layer.kind, layer.document.file_name, layer.document.content, layer.kind
            )
        })
        .collect::<Vec<_>>();

    if let Some(path) = discover_named_file(project_root, MAKO_FILES) {
        if let Some(content) = load_mako_context_file(&path, "Mako project overlay") {
            let label = display_context_file_name(&path, "MAKO.md");
            sections.push(format!(
                "[MAKO PROJECT OVERLAY - {}]\n\n{}\n\n[END MAKO PROJECT OVERLAY]",
                label, content
            ));
        }
    }

    sections
}

fn load_mako_context_file(path: &Path, context: &'static str) -> Option<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            warn!(context, path = %path.display(), error = %error, "Failed to read Mako context file");
            return None;
        }
    };

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn display_context_file_name(path: &Path, fallback: &str) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(fallback)
        .to_string()
}

fn discover_instruction_files(working_dir: &Path) -> Vec<PathBuf> {
    let start = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    let root = discover_project_root(&start);
    let mut dirs = Vec::new();
    let mut current = start.as_path();

    loop {
        dirs.push(current.to_path_buf());
        if current == root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }

    dirs.reverse();

    let mut files = Vec::new();
    for dir in dirs {
        if let Some(path) = PROJECT_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
        {
            files.push(path);
        }
    }

    files
}

fn discover_named_file(base_dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|name| base_dir.join(name))
        .find(|path| path.is_file())
}

fn discover_project_root(working_dir: &Path) -> &Path {
    for ancestor in working_dir.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor;
        }
    }
    working_dir
}

#[cfg(test)]
mod tests {
    use super::{
        build_mako_context_sections, build_mako_context_sections_with_home, build_plan_context,
        build_project_context, build_skills_context, inject_context, summarize_git_status,
    };
    use std::fs;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    use crate::agent::DelegatedRunStage;
    use crate::ai::types::{Content, ModelMessage, Role};
    use crate::paths;
    use crate::skills::SkillsManager;
    use crate::storage::reports::CreateReportInput;
    use crate::storage::{
        AutonomousTaskStore, Database, DelegatedRunRole, DelegatedRunScope, DelegatedRunStartInput,
        DelegatedRunStore, MemoryStore, MemoryType, ReportStore, SessionManager, WorkMode,
    };

    #[test]
    fn project_context_loads_hierarchical_instruction_files() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        let nested = repo.join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("AGENTS.md"), "root instructions").unwrap();
        fs::write(repo.join("a").join("CLAUDE.md"), "nested instructions").unwrap();

        let context = build_project_context(&nested);

        assert!(context.contains("root instructions"));
        assert!(context.contains("nested instructions"));
    }

    #[test]
    fn build_mako_context_loads_global_home_files_and_project_overlay() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let mako_home = temp.path().join("mako-home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&mako_home).unwrap();

        fs::write(mako_home.join(paths::MAKO_SOUL_FILE), "Keep moving.").unwrap();
        fs::write(mako_home.join(paths::MAKO_IDENTITY_FILE), "Name: Mako").unwrap();
        fs::write(
            mako_home.join(paths::MAKO_HEARTBEAT_FILE),
            "Check queued work.",
        )
        .unwrap();
        fs::write(repo.join("MAKO.md"), "Project-specific operating notes.").unwrap();

        let context = build_mako_context_sections_with_home(&repo, &mako_home).join("\n\n");

        assert!(context.contains("[MAKO SOUL - MAKO_SOUL.md]"));
        assert!(context.contains("Keep moving."));
        assert!(context.contains("[MAKO IDENTITY - MAKO_IDENTITY.md]"));
        assert!(context.contains("Name: Mako"));
        assert!(context.contains("[MAKO HEARTBEAT - MAKO_HEARTBEAT.md]"));
        assert!(context.contains("Check queued work."));
        assert!(context.contains("[MAKO PROJECT OVERLAY - MAKO.md]"));
        assert!(context.contains("Project-specific operating notes."));
    }

    #[test]
    fn build_mako_context_falls_back_to_project_overlay_when_global_home_is_empty() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let mako_home = temp.path().join("mako-home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&mako_home).unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let context = build_mako_context_sections_with_home(&repo, &mako_home).join("\n\n");

        assert!(context.contains("[MAKO PROJECT OVERLAY - MAKO.md]"));
        assert!(context.contains("Always Swimming."));
    }

    #[test]
    fn build_mako_context_accepts_legacy_generic_home_file_names() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let mako_home = temp.path().join("mako-home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&mako_home).unwrap();

        fs::write(mako_home.join("SOUL.md"), "Legacy soul.").unwrap();
        fs::write(mako_home.join("IDENTITY.md"), "Legacy identity.").unwrap();

        let context = build_mako_context_sections_with_home(&repo, &mako_home).join("\n\n");

        assert!(context.contains("[MAKO SOUL - SOUL.md]"));
        assert!(context.contains("Legacy soul."));
        assert!(context.contains("[MAKO IDENTITY - IDENTITY.md]"));
        assert!(context.contains("Legacy identity."));
    }

    #[test]
    fn build_mako_context_uses_global_home_path_helper_without_panic() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let context = build_mako_context_sections(&repo).join("\n\n");

        assert!(context.contains("Always Swimming."));
    }

    #[test]
    fn build_mako_context_sections_preserve_layer_order() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let mako_home = temp.path().join("mako-home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&mako_home).unwrap();

        fs::write(mako_home.join(paths::MAKO_SOUL_FILE), "Soul.").unwrap();
        fs::write(mako_home.join(paths::MAKO_IDENTITY_FILE), "Identity.").unwrap();
        fs::write(mako_home.join(paths::MAKO_HEARTBEAT_FILE), "Heartbeat.").unwrap();
        fs::write(mako_home.join(paths::MAKO_MEMORY_FILE), "Memory.").unwrap();
        fs::write(mako_home.join(paths::MAKO_CHANNELS_FILE), "Channels.").unwrap();
        fs::write(repo.join("MAKO.md"), "Overlay.").unwrap();

        let sections = build_mako_context_sections_with_home(&repo, &mako_home);
        let labels = sections
            .iter()
            .map(|section| {
                section
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                "[MAKO SOUL - MAKO_SOUL.md]".to_string(),
                "[MAKO IDENTITY - MAKO_IDENTITY.md]".to_string(),
                "[MAKO HEARTBEAT - MAKO_HEARTBEAT.md]".to_string(),
                "[MAKO MEMORY - MAKO_MEMORY.md]".to_string(),
                "[MAKO CHANNELS - MAKO_CHANNELS.md]".to_string(),
                "[MAKO PROJECT OVERLAY - MAKO.md]".to_string(),
            ]
        );
    }

    #[test]
    fn build_plan_context_falls_back_to_generic_plan_mode_when_store_unavailable() {
        let temp = TempDir::new().unwrap();
        let missing_db_path = temp.path().join("missing").join("krusty.db");

        let context = build_plan_context(&missing_db_path, "session-id", WorkMode::Plan);

        assert!(context.contains("[PLAN MODE ACTIVE]"));
        assert!(context.contains("You CANNOT write, edit, or create files"));
    }

    #[test]
    fn build_plan_context_returns_empty_when_store_unavailable_in_build_mode() {
        let temp = TempDir::new().unwrap();
        let missing_db_path = temp.path().join("missing").join("krusty.db");

        let context = build_plan_context(&missing_db_path, "session-id", WorkMode::Build);

        assert!(context.is_empty());
    }

    #[test]
    fn build_skills_context_returns_empty_when_manager_is_busy() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let _guard = skills
            .try_write()
            .unwrap_or_else(|_| panic!("test should acquire write lock"));

        let context = build_skills_context(&skills, true);

        assert!(context.is_empty());
    }

    #[test]
    fn summarize_git_status_counts_modified_staged_and_untracked() {
        let summary = summarize_git_status(" M src/lib.rs\nA  Cargo.toml\n?? scratch.txt\n");

        assert_eq!(
            summary.as_deref(),
            Some("1 modified, 1 staged, 1 untracked")
        );
    }

    #[test]
    fn summarize_git_status_returns_none_for_clean_status() {
        let summary = summarize_git_status("");

        assert!(summary.is_none());
    }

    #[test]
    fn inject_context_skips_project_instructions_without_explicit_project_dir() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("AGENTS.md"), "repo instructions").unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "hello".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            repo.join("krusty.db").as_path(),
            "session-id",
            repo,
            None,
            WorkMode::Build,
            &skills,
            None,
            None,
        );

        assert_eq!(injected.len(), 3);
        assert_eq!(injected[0].role, Role::System);
        assert!(matches!(
            &injected[0].content[0],
            Content::Text { text } if text.contains("[WORKSPACE MODE: NEUTRAL]")
        ));
        assert_eq!(injected[1].role, Role::System);
        assert!(matches!(
            &injected[1].content[0],
            Content::Text { text } if text.contains("[ENVIRONMENT]")
        ));
        assert_eq!(injected[2].role, Role::User);
    }

    #[test]
    fn inject_context_includes_mako_identity_only_for_mako_sessions() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("AGENTS.md"), "repo instructions").unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "hello".to_string(),
            }],
        }];

        let mako_injected = inject_context(
            &conversation,
            repo.join("krusty.db").as_path(),
            "session-id",
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("mako"),
        );
        let code_injected = inject_context(
            &conversation,
            repo.join("krusty.db").as_path(),
            "session-id",
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("code"),
        );

        assert!(mako_injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text } if text.contains("[MAKO PROJECT OVERLAY - MAKO.md]") && text.contains("Always Swimming.")
            )
        }));
        assert!(mako_injected.iter().all(|message| {
            !matches!(
                &message.content[0],
                Content::Text { text } if text.contains("[MAKO HOME ")
            )
        }));
        assert!(!code_injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text } if text.contains("[MAKO PROJECT OVERLAY - MAKO.md]")
            )
        }));
    }

    #[test]
    fn inject_context_places_all_mako_layers_before_project_settings() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join(".krusty")).unwrap();
        fs::write(repo.join("AGENTS.md"), "repo instructions").unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();
        fs::write(
            repo.join(".krusty").join("settings.json"),
            r#"{ "system_prompt_append": "Project append." }"#,
        )
        .unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "hello".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            repo.join("krusty.db").as_path(),
            "session-id",
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("mako"),
        );

        let texts = injected
            .iter()
            .filter_map(|message| match &message.content[0] {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let settings_index = texts
            .iter()
            .position(|text| text.contains("[PROJECT SETTINGS]"))
            .unwrap();
        let mako_indices = texts
            .iter()
            .enumerate()
            .filter_map(|(index, text)| text.contains("[MAKO ").then_some(index))
            .collect::<Vec<_>>();

        assert!(!mako_indices.is_empty());
        assert!(mako_indices.iter().all(|index| *index < settings_index));
    }

    #[test]
    fn inject_context_does_not_inline_mako_coordinator_prompt() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "hello".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            repo.join("krusty.db").as_path(),
            "session-id",
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("mako"),
        );

        assert!(!injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text } if text.contains("[MAKO COORDINATOR]")
            )
        }));
    }

    #[test]
    fn inject_context_includes_mako_knowledge_from_memory_and_reports() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let db_path = repo.join("krusty.db");
        let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
        let session_id = session_manager
            .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
            .unwrap();

        let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
        memory_store
            .save(
                MemoryType::Project,
                "Auth decision",
                "Use the daemon loop as the canonical wake path.",
                Some(repo.to_string_lossy().as_ref()),
                None,
            )
            .unwrap();

        let report_store = ReportStore::new(Database::new(&db_path).unwrap());
        report_store
            .create_report(CreateReportInput {
                title: "Wake pipeline check",
                session_id: &session_id,
                project_dir: Some(repo.to_string_lossy().as_ref()),
                report_root: Some(repo),
                content: "The wake pipeline is healthy.",
                summary: "Validated the wake pipeline end to end.",
                tags: &[],
                sources: &[],
            })
            .unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "hello".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            db_path.as_path(),
            &session_id,
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("mako"),
        );

        assert!(injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text }
                    if text.contains("[MAKO KNOWLEDGE]")
                        && text.contains("## Carry Forward")
                        && text.contains("Auth decision")
                        && text.contains("## Recent Reports")
                        && text.contains("Wake pipeline check")
            )
        }));
    }

    #[test]
    fn inject_context_prioritizes_relevant_reports_over_recent_reports() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();

        let db_path = repo.join("krusty.db");
        let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
        let session_id = session_manager
            .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
            .unwrap();

        let report_store = ReportStore::new(Database::new(&db_path).unwrap());
        report_store
            .create_report(CreateReportInput {
                title: "Queue Scheduling Audit",
                session_id: &session_id,
                project_dir: Some(repo.to_string_lossy().as_ref()),
                report_root: Some(repo),
                content: "Investigated overdue runs and wake cadence.",
                summary: "Queue scheduling and overdue run analysis.",
                tags: &["queue".into(), "scheduling".into()],
                sources: &[],
            })
            .unwrap();
        for index in 0..5 {
            report_store
                .create_report(CreateReportInput {
                    title: &format!("Unrelated report {index}"),
                    session_id: &session_id,
                    project_dir: Some(repo.to_string_lossy().as_ref()),
                    report_root: Some(repo),
                    content: "Miscellaneous notes.",
                    summary: "General project notes.",
                    tags: &["misc".into()],
                    sources: &[],
                })
                .unwrap();
        }

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "Please stabilize queue scheduling and overdue runs.".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            db_path.as_path(),
            &session_id,
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("code"),
        );

        assert!(injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text }
                    if text.contains("[RELEVANT REPORTS]")
                        && text.contains("Queue Scheduling Audit")
            )
        }));
    }

    #[test]
    fn inject_context_uses_active_mako_tasks_for_report_relevance() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let db_path = repo.join("krusty.db");
        let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
        let session_id = session_manager
            .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
            .unwrap();

        let report_store = ReportStore::new(Database::new(&db_path).unwrap());
        report_store
            .create_report(CreateReportInput {
                title: "Scheduler Drift Runbook",
                session_id: &session_id,
                project_dir: Some(repo.to_string_lossy().as_ref()),
                report_root: Some(repo),
                content: "Drift handling and wake diagnostics.",
                summary: "How to investigate scheduler drift safely.",
                tags: &["scheduler".into(), "drift".into()],
                sources: &[],
            })
            .unwrap();
        for index in 0..5 {
            report_store
                .create_report(CreateReportInput {
                    title: &format!("Background note {index}"),
                    session_id: &session_id,
                    project_dir: Some(repo.to_string_lossy().as_ref()),
                    report_root: Some(repo),
                    content: "Background knowledge.",
                    summary: "General notes.",
                    tags: &["misc".into()],
                    sources: &[],
                })
                .unwrap();
        }

        let task_store = AutonomousTaskStore::new(Database::new(&db_path).unwrap());
        task_store
            .create_task(&session_id, "Investigate scheduler drift", "", &[])
            .unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "Keep watch and continue.".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            db_path.as_path(),
            &session_id,
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("mako"),
        );

        assert!(injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text }
                    if text.contains("[MAKO KNOWLEDGE]")
                        && text.contains("## Relevant Reports")
                        && text.contains("Scheduler Drift Runbook")
            )
        }));
    }

    #[test]
    fn inject_context_does_not_duplicate_generic_memory_and_report_blocks_for_mako() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let db_path = repo.join("krusty.db");
        let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
        let session_id = session_manager
            .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
            .unwrap();

        let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
        memory_store
            .save(
                MemoryType::Feedback,
                "Status preference",
                "Show upcoming wakes before aggregate counters.",
                Some(repo.to_string_lossy().as_ref()),
                None,
            )
            .unwrap();

        let report_store = ReportStore::new(Database::new(&db_path).unwrap());
        report_store
            .create_report(CreateReportInput {
                title: "Queue audit",
                session_id: &session_id,
                project_dir: Some(repo.to_string_lossy().as_ref()),
                report_root: Some(repo),
                content: "Queue ordering is stable.",
                summary: "Queue ordering remains stable.",
                tags: &[],
                sources: &[],
            })
            .unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "hello".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            db_path.as_path(),
            &session_id,
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            Some("mako"),
        );

        let texts = injected
            .iter()
            .filter_map(|message| match &message.content[0] {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(texts.iter().any(|text| text.contains("[MAKO KNOWLEDGE]")));
        assert!(!texts
            .iter()
            .any(|text| text.contains("[PERSISTENT MEMORY]")));
        assert!(!texts.iter().any(|text| text.contains("[RECENT REPORTS]")));
    }

    #[test]
    fn inject_context_includes_recent_delegated_run_guidance() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::create_dir_all(repo.join(".git")).unwrap();
        let db_path = repo.join("krusty.db");
        let db = Database::new(&db_path).unwrap();
        let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
        let session_id = session_manager
            .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
            .unwrap();
        let store = DelegatedRunStore::new(db);
        store
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: "run-1".to_string(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: Some("tool-1".to_string()),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Created,
                provider: Some("MiniMax".to_string()),
                model: Some("MiniMax-M2.5".to_string()),
                resumable: true,
                resumed_from_run_id: None,
                target_scope: vec![DelegatedRunScope {
                    label: "src/storage".to_string(),
                    path: "crates/krusty-core/src/storage".to_string(),
                    kind: "directory".to_string(),
                }],
            })
            .unwrap();
        store
            .finalize_run(
                "run-1",
                DelegatedRunStage::Complete,
                &serde_json::json!({"outcome":"success"}),
                Some("Architecture review completed across 1 targets."),
                true,
            )
            .unwrap();

        let skills = RwLock::new(SkillsManager::with_defaults(repo));
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "use explore again".to_string(),
            }],
        }];

        let injected = inject_context(
            &conversation,
            db_path.as_path(),
            &session_id,
            repo,
            Some(repo),
            WorkMode::Build,
            &skills,
            None,
            None,
        );

        assert!(injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text }
                    if text.contains("[RECENT DELEGATED RUNS]")
                        && text.contains("prefer calling `explore` again")
                        && text.contains("run-1")
                        && text.contains("src/storage")
            )
        }));
    }
}
