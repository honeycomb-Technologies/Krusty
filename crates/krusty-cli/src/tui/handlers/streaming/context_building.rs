//! Context building for AI conversations
//!
//! Builds various context sections that get injected into AI conversations:
//! - LSP diagnostics
//! - Active plans
//! - Available skills
//! - Project instructions

use crate::tui::app::{App, WorkMode};

/// Sanitize plan titles for safe markdown embedding
/// Escapes backticks and quotes that could break formatting
pub fn sanitize_plan_title(title: &str) -> String {
    title
        .replace(['`', '"'], "'")
        .replace('[', "(")
        .replace(']', ")")
}

impl App {
    /// Build diagnostics context for AI from LSP
    pub fn build_diagnostics_context(&self) -> String {
        let cache = self.services.lsp_manager.diagnostics_cache();
        let error_count = cache.error_count();
        let warning_count = cache.warning_count();

        if error_count == 0 && warning_count == 0 {
            return String::new();
        }

        let diagnostics_str = cache.format_for_display();

        format!(
            "[SYSTEM CONTEXT] Current LSP Diagnostics ({} errors, {} warnings):\n{}",
            error_count, warning_count, diagnostics_str
        )
    }

    /// Build plan context for AI - shown in both PLAN and BUILD modes when a plan is active
    pub fn build_plan_context(&self) -> String {
        match self.ui.work_mode {
            WorkMode::Plan => self.build_plan_mode_context(),
            WorkMode::Build => self.build_build_mode_context(),
        }
    }

    /// Build context for Plan mode
    fn build_plan_mode_context(&self) -> String {
        let Some(plan) = &self.active_plan else {
            // In plan mode but no active plan - provide instructions with format
            return r#"[PLAN MODE ACTIVE]

You are in PLAN MODE. The user wants to create a plan before implementing.

In plan mode:
- You can READ files, search code, and explore the codebase
- You CANNOT write, edit, or create files
- You CANNOT run modifying bash commands (git commit, rm, mv, etc.)
- Focus on understanding the codebase and designing an implementation approach

IMPORTANT: When requirements are ambiguous or you need clarification, use the AskUserQuestion tool instead of asking in plain text. This provides a better UX with clickable options.

When creating a plan, use this EXACT format (the system will auto-detect and save it):

```
# Plan: [Title]

## Phase 1: [Phase Name]

- [ ] Task description here
  > Context: Implementation details or notes
- [ ] Another task
  - [ ] Subtask for complex items

## Phase 2: [Phase Name]

- [ ] Task description
  > Blocked-By: 1.1, 1.2
```

Key formatting rules:
- Title line: `# Plan: Your Title Here`
- Phase headers: `## Phase N: Phase Name`
- Tasks: `- [ ] Description` (pending), `- [x] Description` (completed), `- [>] Description` (in-progress), `- [~] Description` (blocked)
- Context: `> Context: details` - optional implementation notes
- Dependencies: `> Blocked-By: task_ids` - tasks that must complete first
- Subtasks: Indent 2 spaces for subtasks under a parent task

After exploring the codebase, output your plan in this format. The user can exit plan mode with Ctrl+B to begin implementation."#.to_string();
        };

        // Build context from active plan (truncated if large)
        let (completed, total) = plan.progress();
        let markdown = plan.to_context();

        // Get ready and blocked tasks for visibility
        let ready_tasks = plan.get_ready_tasks();
        let blocked_tasks = plan.get_blocked_tasks();

        let ready_count = ready_tasks.len();
        let blocked_count = blocked_tasks.len();

        format!(
            r#"[PLAN MODE ACTIVE - Plan: "{}"]

Progress: {}/{} tasks completed | {} ready | {} blocked

In plan mode:
- You can READ files, search code, and explore the codebase
- You CANNOT write, edit, or create files until plan mode is exited
- Focus on the current plan and track progress
- Use the AskUserQuestion tool for clarifications (not plain text questions)

## Current Plan

{}

---

When working on tasks, update progress by telling the user which task you're working on.
The user can exit plan mode with Ctrl+B when ready to implement."#,
            sanitize_plan_title(&plan.title),
            completed,
            total,
            ready_count,
            blocked_count,
            markdown
        )
    }

    /// Build context for Build mode with active plan
    fn build_build_mode_context(&self) -> String {
        let Some(plan) = &self.active_plan else {
            return String::new();
        };

        let (completed, total) = plan.progress();
        let markdown = plan.to_context();

        // Get ready and blocked tasks
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
                    let blockers = t.blocked_by.join(", ");
                    format!(
                        "  - Task {}: {} (waiting on: {})",
                        t.id, t.description, blockers
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"[ACTIVE PLAN - "{}"]

Progress: {}/{} tasks completed

## Ready to Work (no blockers)
{}

## Blocked Tasks (waiting on dependencies)
{}

## Current Plan

{}

---

## CRITICAL: Task Workflow Protocol

You MUST follow this disciplined workflow. Do NOT batch-complete tasks or skip steps.

### For EACH task (one at a time):

1. **PICK ONE** ready task from the list above
2. **START IT**: `task_start(task_id: "X.X")` - marks as in-progress
3. **DO THE WORK** for that specific task only
4. **COMPLETE IT**: `task_complete(task_id: "X.X", result: "specific accomplishment")`
5. **THEN** move to the next task

### Rules:
- Work on ONE task at a time - no parallel task completion
- Always `task_start` BEFORE doing work (shows user what you're working on)
- Always `task_complete` with a SPECIFIC result for THAT task (not generic)
- If a task is complex, use `add_subtask` to break it down BEFORE starting
- Check the "Ready to Work" list - only work on unblocked tasks

### Tools:
- `task_start(task_id)` - REQUIRED before starting work (will fail if task is blocked)
- `task_complete(task_id, result)` - REQUIRED: task must be in-progress, result must be specific to that task
- `add_subtask(parent_id, description, context)` - break down complex tasks before starting
- `set_dependency(task_id, blocked_by)` - establish task ordering

### What will FAIL:
- `task_complete` without `task_start` first → Error
- `task_complete` on a blocked task → Error
- `task_complete` with batch task_ids → Error (one at a time only)
- `task_start` on a blocked task → Error"#,
            sanitize_plan_title(&plan.title),
            completed,
            total,
            ready_list,
            blocked_list,
            markdown
        )
    }

    /// Build skills context for AI - lists available skills with metadata only
    ///
    /// Uses progressive disclosure: only names/descriptions in system prompt,
    /// AI can invoke the skill tool to load full content when needed.
    pub fn build_skills_context(&self) -> String {
        // Get skill infos - needs write lock as list_skills may refresh cache
        let mut skills_guard = match self.services.skills_manager.try_write() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("Skills manager locked, skipping skills context");
                return String::new();
            }
        };

        let skills = skills_guard.list_skills();
        if skills.is_empty() {
            return String::new();
        }

        let mut context = String::from("[AVAILABLE SKILLS]\n\n");
        context.push_str("The following skills are available. Use the `skill` tool to invoke a skill and get detailed instructions.\n\n");

        for info in skills {
            context.push_str(&format!("- **{}**: {}\n", info.name, info.description));
            if !info.tags.is_empty() {
                context.push_str(&format!("  Tags: {}\n", info.tags.join(", ")));
            }
        }

        context.push_str("\nTo use a skill: `skill(skill: \"skill-name\")`\n");
        context
    }

    /// Build project context from instruction files.
    ///
    /// Reads project-specific instructions from the working directory.
    /// These files provide context about the codebase, conventions, and guidelines.
    pub fn build_project_context(&self) -> String {
        // Support common AI coding assistant instruction file formats
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

        for filename in PROJECT_FILES {
            let path = self.working_dir.join(filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                tracing::debug!(
                    "Loaded project context from {} ({} chars)",
                    filename,
                    content.len()
                );
                return format!(
                    "[PROJECT INSTRUCTIONS - {}]\n\n{}\n\n[END PROJECT INSTRUCTIONS]",
                    filename, content
                );
            }
        }

        String::new()
    }
}
