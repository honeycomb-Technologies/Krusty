//! Context building for AI conversations
//!
//! Builds various context sections that get injected into AI conversations:
//! - Active plans
//! - Available skills
//! - Project instructions

use crate::ai::types::Content;
use crate::tui::app::{App, WorkMode};
use krusty_core::index::{CodebaseStore, InsightStore, SearchQuery, SemanticRetrieval};

/// Sanitize plan titles for safe markdown embedding
/// Escapes backticks and quotes that could break formatting
pub fn sanitize_plan_title(title: &str) -> String {
    title
        .replace(['`', '"'], "'")
        .replace('[', "(")
        .replace(']', ")")
}

impl App {
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

    /// Build insights context from accumulated codebase learnings
    pub fn build_insights_context(&self) -> String {
        let Some(ref sm) = self.services.session_manager else {
            return String::new();
        };

        let conn = sm.db().conn();
        let working_dir_str = self.working_dir.to_string_lossy().to_string();

        let codebase_id = match CodebaseStore::new(conn).get_by_path(&working_dir_str) {
            Ok(Some(codebase)) => codebase.id,
            _ => return String::new(),
        };

        let insight_store = InsightStore::new(conn);
        let insights = match insight_store.get_top(&codebase_id, 20) {
            Ok(insights) if !insights.is_empty() => insights,
            _ => return String::new(),
        };

        // Touch access counts for ranking
        let ids: Vec<&str> = insights.iter().map(|i| i.id.as_str()).collect();
        if let Err(e) = insight_store.touch_accessed(&ids) {
            tracing::debug!("Failed to update insight access counts: {e}");
        }

        let mut context = String::from("[CODEBASE RULES]\nIMPORTANT: These are verified patterns and conventions for this codebase. You MUST follow them.\nViolating these will introduce inconsistencies and bugs.\n\n");
        for insight in &insights {
            context.push_str(&format!(
                "- [{}] {} (confidence: {:.0}%)\n",
                insight.insight_type.as_str(),
                insight.content,
                insight.confidence * 100.0
            ));
        }
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

    /// Extract the latest user message text from the conversation
    fn extract_latest_user_query(&self) -> Option<String> {
        self.chat
            .conversation
            .iter()
            .rev()
            .find(|msg| msg.role == crate::ai::types::Role::User)
            .and_then(|msg| {
                msg.content.iter().find_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
    }

    /// Build search context from codebase index (semantic or keyword search)
    pub fn build_search_context(&self) -> String {
        let query_text = match self.extract_latest_user_query() {
            Some(text) if !text.is_empty() => text,
            _ => return String::new(),
        };

        let Some(ref sm) = self.services.session_manager else {
            return String::new();
        };

        let conn = sm.db().conn();
        let working_dir_str = self.working_dir.to_string_lossy().to_string();

        let codebase_id = match CodebaseStore::new(conn).get_by_path(&working_dir_str) {
            Ok(Some(codebase)) => codebase.id,
            _ => return String::new(),
        };

        let engine_guard = self.embedding_engine.try_read().ok();
        let engine_ref = engine_guard.as_ref().and_then(|g| g.as_ref());
        let has_embeddings = engine_ref.is_some();
        let mut retrieval = SemanticRetrieval::new(conn);
        if let Some(engine) = engine_ref {
            retrieval = retrieval.with_embeddings(engine);
        }

        let search_query = SearchQuery::new().text(&query_text).limit(10);

        let results =
            match futures::executor::block_on(retrieval.search(&codebase_id, search_query)) {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!("Codebase search failed: {e}");
                    return String::new();
                }
            };

        let filtered: Vec<_> = results.into_iter().filter(|r| r.score >= 0.3).collect();

        if filtered.is_empty() {
            tracing::debug!(
                mode = if has_embeddings {
                    "semantic"
                } else {
                    "keyword"
                },
                "Search: no results above threshold"
            );
            return String::new();
        }

        let top_score = filtered.first().map(|r| r.score).unwrap_or(0.0);
        tracing::info!(
            mode = if has_embeddings {
                "semantic"
            } else {
                "keyword"
            },
            results = filtered.len(),
            top_score = format!("{:.2}", top_score),
            "Search: matched symbols"
        );

        let mut context = String::from("[CODEBASE SEARCH RESULTS]\nSymbols matching the current query. Reference these locations before searching manually.\n\n");
        for result in &filtered {
            let sig = result
                .signature
                .as_deref()
                .map(|s| format!(": {s}"))
                .unwrap_or_default();
            context.push_str(&format!(
                "- [{}] {}{} ({}:{}-{})\n",
                result.symbol_type.as_str(),
                result.symbol_path,
                sig,
                result.file_path,
                result.line_start,
                result.line_end,
            ));
        }
        context
    }
}
