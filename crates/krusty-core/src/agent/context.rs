//! Context injection for the agentic loop.
//!
//! Builds plan, skills, and project context strings that get injected as
//! system messages at the head of the conversation before each AI call.
//! This ensures the AI is always aware of the active plan, available skills,
//! and project-specific instructions.

use std::path::Path;

use tokio::sync::RwLock;

use crate::ai::types::{Content, ModelMessage, Role};
use crate::plan::PlanManager;
use crate::skills::SkillsManager;
use crate::storage::WorkMode;

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

/// Build a conversation clone with context system messages prepended.
///
/// Injects plan, skills, and project context in the same order as the TUI:
/// project → plan → skills → original conversation.
pub fn inject_context(
    conversation: &[ModelMessage],
    db_path: &Path,
    session_id: &str,
    working_dir: &Path,
    work_mode: WorkMode,
    skills_manager: &RwLock<SkillsManager>,
) -> Vec<ModelMessage> {
    let plan_ctx = build_plan_context(db_path, session_id, work_mode);
    let skills_ctx = build_skills_context(skills_manager);
    let project_ctx = build_project_context(working_dir);

    let mut injected = Vec::with_capacity(conversation.len() + 3);

    if !project_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: project_ctx }],
        });
    }
    if !plan_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: plan_ctx }],
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

/// Build plan context from the active plan for this session.
pub fn build_plan_context(db_path: &Path, session_id: &str, work_mode: WorkMode) -> String {
    let plan_manager = match PlanManager::new(db_path.to_path_buf()) {
        Ok(pm) => pm,
        Err(_) => return String::new(),
    };

    let plan = match plan_manager.get_plan(session_id) {
        Ok(Some(p)) => p,
        _ => {
            return if work_mode == WorkMode::Plan {
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
pub fn build_skills_context(skills_manager: &RwLock<SkillsManager>) -> String {
    let mut guard = match skills_manager.try_write() {
        Ok(g) => g,
        Err(_) => return String::new(),
    };

    let skills = guard.list_skills();
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

/// Build project context from instruction files in the working directory.
///
/// Searches for well-known instruction files (KRAB.md, CLAUDE.md, etc.)
/// and returns the first one found wrapped in marker tags.
pub fn build_project_context(working_dir: &Path) -> String {
    for filename in PROJECT_FILES {
        let path = working_dir.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            return format!(
                "[PROJECT INSTRUCTIONS - {}]\n\n{}\n\n[END PROJECT INSTRUCTIONS]",
                filename, content
            );
        }
    }
    String::new()
}
