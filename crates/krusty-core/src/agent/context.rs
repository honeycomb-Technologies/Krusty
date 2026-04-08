//! Context injection for the agentic loop.
//!
//! Builds plan, skills, and project context strings that get injected as
//! system messages at the head of the conversation before each AI call.
//! This ensures the AI is always aware of the active plan, available skills,
//! and project-specific instructions.

use std::path::{Path, PathBuf};
use std::process::Command;

use tokio::sync::RwLock;
use tracing::warn;

use crate::ai::types::{Content, ModelMessage, Role};
use crate::plan::PlanManager;
use crate::skills::SkillsManager;
use crate::storage::{
    AutonomousTaskStore, Database, DelegatedRunStore, MemoryStore, MemoryType, ProjectSettings,
    ReportStore, TaskStatus, WorkMode,
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
    let memory_ctx = build_memory_context(
        db_path,
        project_dir
            .map(|p| p.to_string_lossy().to_string())
            .as_deref(),
        None, // user_id — single-tenant for now
    );
    let plan_ctx = build_plan_context(db_path, session_id, work_mode);
    let delegated_ctx = build_delegated_context(db_path, session_id);
    let task_ctx = build_autonomous_task_context(db_path, session_id);
    let report_ctx = build_report_context(
        db_path,
        project_dir
            .map(|p| p.to_string_lossy().to_string())
            .as_deref(),
    );
    let coordinator_ctx = build_coordinator_context(session_type.unwrap_or("code"));
    let skills_ctx = build_skills_context(skills_manager, project_dir.is_some());
    let project_ctx = project_dir.map(build_project_context).unwrap_or_default();
    let mako_ctx = if session_type == Some("mako") {
        build_mako_context(project_dir.unwrap_or(working_dir))
    } else {
        String::new()
    };
    let project_settings = project_dir.map(ProjectSettings::load).unwrap_or_default();

    let mut injected = Vec::with_capacity(conversation.len() + 8);

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
    if !project_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: project_ctx }],
        });
    }
    if !mako_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: mako_ctx }],
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
    if !coordinator_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: coordinator_ctx,
            }],
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
    let memories = store.list(project_dir, user_id);
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
fn build_report_context(db_path: &Path, project_dir: Option<&str>) -> String {
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

    let mut lines = vec!["[RECENT REPORTS]".to_string()];
    for report in reports.iter().take(5) {
        let summary = truncate_utf8(&report.summary, 200);
        lines.push(format!(
            "- \"{}\" ({}): {}",
            report.title, report.created_at, summary
        ));
    }
    lines.push("Use `ReadReport` tool to access full content.".to_string());

    lines.join("\n")
}

/// Build coordinator prompt for Mako sessions.
fn build_coordinator_context(session_type: &str) -> String {
    if session_type == "mako" {
        crate::agent::coordinator_prompt::COORDINATOR_SYSTEM_PROMPT.to_string()
    } else {
        String::new()
    }
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

fn build_mako_context(project_root: &Path) -> String {
    let Some(path) = discover_named_file(project_root, MAKO_FILES) else {
        return String::new();
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            warn!(path = %path.display(), error = %error, "Failed to read Mako identity file");
            return String::new();
        }
    };
    if content.trim().is_empty() {
        return String::new();
    }

    let label = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("MAKO.md");

    format!(
        "[MAKO IDENTITY - {}]\n\n{}\n\n[END MAKO IDENTITY]",
        label, content
    )
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
        build_mako_context, build_plan_context, build_project_context, build_skills_context,
        inject_context, summarize_git_status,
    };
    use std::fs;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    use crate::agent::DelegatedRunStage;
    use crate::ai::types::{Content, ModelMessage, Role};
    use crate::skills::SkillsManager;
    use crate::storage::{
        Database, DelegatedRunRole, DelegatedRunScope, DelegatedRunStartInput, DelegatedRunStore,
        SessionManager, WorkMode,
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
    fn build_mako_context_loads_project_root_identity_file() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

        let context = build_mako_context(repo);

        assert!(context.contains("[MAKO IDENTITY - MAKO.md]"));
        assert!(context.contains("Always Swimming."));
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
                Content::Text { text } if text.contains("[MAKO IDENTITY - MAKO.md]") && text.contains("Always Swimming.")
            )
        }));
        assert!(!code_injected.iter().any(|message| {
            matches!(
                &message.content[0],
                Content::Text { text } if text.contains("[MAKO IDENTITY - MAKO.md]")
            )
        }));
    }

    #[test]
    fn inject_context_places_mako_identity_before_project_settings() {
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
        let mako_index = texts
            .iter()
            .position(|text| text.contains("[MAKO IDENTITY - MAKO.md]"))
            .unwrap();
        let settings_index = texts
            .iter()
            .position(|text| text.contains("[PROJECT SETTINGS]"))
            .unwrap();

        assert!(mako_index < settings_index);
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
