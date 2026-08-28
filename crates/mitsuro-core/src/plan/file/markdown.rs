use chrono::{DateTime, Utc};

use super::{PlanFile, PlanPhase, PlanStatus, PlanTask, TaskStatus};

impl PlanTask {
    /// Format as markdown checkbox line.
    pub fn to_markdown(&self) -> String {
        self.to_markdown_with_depth(0)
    }

    /// Format as markdown with indentation for subtasks.
    pub fn to_markdown_with_depth(&self, depth: usize) -> String {
        let indent = "  ".repeat(depth);
        let checkbox = match self.status {
            TaskStatus::Completed => "[x]",
            TaskStatus::InProgress => "[>]",
            TaskStatus::Blocked => "[~]",
            TaskStatus::Pending => "[ ]",
        };

        let mut lines = vec![format!(
            "{}- {} Task {}: {}",
            indent, checkbox, self.id, self.description
        )];

        if let Some(ref ctx) = self.context {
            lines.push(format!("{}  > Context: {}", indent, ctx));
        }

        if !self.blocked_by.is_empty() {
            lines.push(format!(
                "{}  > Blocked-By: {}",
                indent,
                self.blocked_by.join(", ")
            ));
        }

        if let Some(ref result) = self.result {
            if let Some(ts) = self.completed_at {
                lines.push(format!(
                    "{}  > Result [{}]: {}",
                    indent,
                    ts.format("%Y-%m-%d %H:%M"),
                    result
                ));
            } else {
                lines.push(format!("{}  > Result: {}", indent, result));
            }
        }

        lines.join("\n")
    }
}

impl PlanPhase {
    /// Format as markdown.
    pub fn to_markdown(&self) -> String {
        let mut lines = vec![
            format!("## Phase {}: {}", self.number, self.name),
            String::new(),
        ];

        let top_level: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| t.parent_id.is_none())
            .collect();

        for task in top_level {
            lines.push(task.to_markdown_with_depth(0));
            for subtask in self
                .tasks
                .iter()
                .filter(|t| t.parent_id.as_ref().map(|p| p == &task.id).unwrap_or(false))
            {
                lines.push(subtask.to_markdown_with_depth(1));
                for subsubtask in self.tasks.iter().filter(|t| {
                    t.parent_id
                        .as_ref()
                        .map(|p| p == &subtask.id)
                        .unwrap_or(false)
                }) {
                    lines.push(subsubtask.to_markdown_with_depth(2));
                }
            }
        }

        lines.join("\n")
    }
}

impl PlanFile {
    /// Maximum context size in characters (~2000 tokens ≈ 8000 chars).
    const MAX_CONTEXT_CHARS: usize = 8000;

    /// Serialize to markdown string for AI context.
    pub fn to_context(&self) -> String {
        let full = self.to_markdown();

        if full.len() <= Self::MAX_CONTEXT_CHARS {
            return full;
        }

        let mut lines = Vec::new();
        lines.push(format!("# Plan: {}", self.title));
        lines.push(String::new());

        let (completed, total) = self.progress();
        lines.push(format!("Progress: {}/{} tasks", completed, total));
        lines.push(String::new());

        for phase in &self.phases {
            let incomplete: Vec<_> = phase.tasks.iter().filter(|t| !t.completed).collect();
            if incomplete.is_empty() {
                continue;
            }

            lines.push(format!("## Phase {}: {}", phase.number, phase.name));
            lines.push(String::new());

            for task in incomplete {
                lines.push(task.to_markdown());
            }
            lines.push(String::new());
        }

        lines.push("---".to_string());
        lines.push(format!(
            "*Plan truncated for context. {} completed tasks hidden.*",
            completed
        ));

        lines.join("\n")
    }

    /// Serialize to markdown string.
    pub fn to_markdown(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!("# Plan: {}", self.title));
        lines.push(String::new());
        lines.push(format!(
            "Created: {}",
            self.created_at.format("%Y-%m-%d %H:%M UTC")
        ));
        if let Some(session) = &self.session_id {
            lines.push(format!("Session: {}", session));
        }
        if let Some(dir) = &self.working_dir {
            lines.push(format!("Working Directory: {}", dir));
        }
        lines.push(format!("Status: {}", self.status));
        if self.version > 0 {
            lines.push(format!("Version: {}", self.version));
        }
        lines.push(String::new());
        lines.push("---".to_string());
        lines.push(String::new());

        for phase in &self.phases {
            lines.push(phase.to_markdown());
            lines.push(String::new());
        }

        if let Some(notes) = &self.notes {
            lines.push("---".to_string());
            lines.push(String::new());
            lines.push("## Notes".to_string());
            lines.push(String::new());
            lines.push(notes.clone());
        }

        lines.join("\n")
    }

    /// Parse from markdown string.
    pub fn from_markdown(content: &str) -> Result<Self, String> {
        const MAX_PLAN_SIZE: usize = 1_024 * 1_024;
        if content.len() > MAX_PLAN_SIZE {
            return Err("Plan file exceeds maximum size of 1MB".to_string());
        }

        tracing::debug!("Parsing plan from markdown");
        let mut plan = PlanFile::new(String::new());
        let mut current_phase: Option<PlanPhase> = None;
        let mut in_notes = false;
        let mut notes_lines: Vec<String> = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();

            if let Some(title) = trimmed.strip_prefix("# Plan:") {
                plan.title = title.trim().to_string();
                tracing::debug!(title = %plan.title, "Parsed plan title");
                continue;
            }

            if parse_metadata_line(&mut plan, trimmed) {
                continue;
            }

            if trimmed == "## Notes" {
                if let Some(phase) = current_phase.take() {
                    plan.phases.push(phase);
                }
                in_notes = true;
                continue;
            }

            if in_notes {
                notes_lines.push(line.to_string());
                continue;
            }

            if let Some(phase) = parse_phase_header(trimmed, plan.phases.len() + 1) {
                if let Some(prev) = current_phase.replace(phase) {
                    plan.phases.push(prev);
                }
                continue;
            }

            if is_metadata_continuation(trimmed) {
                if let Some(ref mut phase) = current_phase {
                    parse_task_metadata_line(phase, trimmed);
                }
                continue;
            }

            if let Some(ref mut phase) = current_phase {
                parse_markdown_task_line(phase, line, trimmed);
            }
        }

        if let Some(phase) = current_phase {
            plan.phases.push(phase);
        }

        if !notes_lines.is_empty() {
            let notes = notes_lines.join("\n").trim().to_string();
            if !notes.is_empty() {
                plan.notes = Some(notes);
            }
        }

        if plan.title.is_empty() {
            return Err("Plan file missing title".to_string());
        }

        Ok(plan)
    }
}

fn parse_metadata_line(plan: &mut PlanFile, trimmed: &str) -> bool {
    if let Some(date_str) = trimmed.strip_prefix("Created:") {
        if let Ok(dt) = DateTime::parse_from_str(
            &format!("{} +0000", date_str.trim()),
            "%Y-%m-%d %H:%M UTC %z",
        ) {
            plan.created_at = dt.with_timezone(&Utc);
        }
        return true;
    }

    if let Some(session) = trimmed.strip_prefix("Session:") {
        plan.session_id = Some(session.trim().to_string());
        return true;
    }

    if let Some(dir) = trimmed.strip_prefix("Working Directory:") {
        plan.working_dir = Some(dir.trim().to_string());
        return true;
    }

    if let Some(status) = trimmed.strip_prefix("Status:") {
        plan.status = status.trim().parse().unwrap_or(PlanStatus::InProgress);
        return true;
    }

    if let Some(version) = trimmed.strip_prefix("Version:") {
        plan.version = version.trim().parse().unwrap_or(0);
        return true;
    }

    false
}

fn parse_phase_header(trimmed: &str, default_number: usize) -> Option<PlanPhase> {
    let after_phase = trimmed.strip_prefix("## Phase")?.trim();
    let colon_pos = after_phase.find(':')?;
    let num_str = after_phase[..colon_pos].trim();
    let name = after_phase[colon_pos + 1..].trim();
    let number = num_str.parse().unwrap_or(default_number);
    tracing::debug!(phase_num = number, phase_name = %name, "Parsed phase");
    Some(PlanPhase::new(number, name))
}

fn is_metadata_continuation(trimmed: &str) -> bool {
    trimmed.starts_with("> ") || trimmed.starts_with('>')
}

fn parse_task_metadata_line(phase: &mut PlanPhase, trimmed: &str) {
    let Some(last_task) = phase.tasks.last_mut() else {
        return;
    };

    let meta = trimmed
        .strip_prefix("> ")
        .unwrap_or(trimmed.strip_prefix('>').unwrap_or(""))
        .trim();
    if let Some(ctx) = meta.strip_prefix("Context:") {
        last_task.context = Some(ctx.trim().to_string());
    } else if let Some(result_with_ts) = meta.strip_prefix("Result") {
        let result_text = if result_with_ts.starts_with(" [") {
            if let Some(bracket_end) = result_with_ts.find("]:") {
                result_with_ts[bracket_end + 2..].trim()
            } else {
                result_with_ts
                    .strip_prefix(':')
                    .unwrap_or(result_with_ts)
                    .trim()
            }
        } else {
            result_with_ts
                .strip_prefix(':')
                .unwrap_or(result_with_ts)
                .trim()
        };
        last_task.result = Some(result_text.to_string());
    } else if let Some(blocked) = meta.strip_prefix("Blocked-By:") {
        last_task.blocked_by = blocked
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
}

fn parse_markdown_task_line(phase: &mut PlanPhase, line: &str, trimmed: &str) {
    let Some((status, completed, task_text)) = parse_checkbox_line(trimmed) else {
        return;
    };

    let indent_level = line.len() - line.trim_start().len();
    let is_subtask = indent_level >= 2;
    let (id, description) =
        parse_markdown_task_identity(task_text, phase.number, phase.tasks.len() + 1);
    let parent_id = if is_subtask {
        id.rfind('.').map(|pos| id[..pos].to_string())
    } else {
        None
    };

    let mut task = PlanTask::new(id.clone(), description);
    task.parent_id = parent_id.clone();
    task.completed = completed;
    task.status = status;

    if let Some(ref pid) = parent_id {
        if let Some(parent_task) = phase.tasks.iter_mut().find(|t| t.id == *pid) {
            parent_task.children.push(id);
        }
    }

    phase.tasks.push(task);
}

pub(super) fn parse_checkbox_line(trimmed: &str) -> Option<(TaskStatus, bool, &str)> {
    if trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]") {
        Some((
            TaskStatus::Completed,
            true,
            trimmed
                .strip_prefix("- [x]")
                .or_else(|| trimmed.strip_prefix("- [X]"))?
                .trim(),
        ))
    } else if trimmed.starts_with("- [>]") {
        Some((
            TaskStatus::InProgress,
            false,
            trimmed.strip_prefix("- [>]")?.trim(),
        ))
    } else if trimmed.starts_with("- [~]") {
        Some((
            TaskStatus::Blocked,
            false,
            trimmed.strip_prefix("- [~]")?.trim(),
        ))
    } else if trimmed.starts_with("- [ ]") {
        Some((
            TaskStatus::Pending,
            false,
            trimmed.strip_prefix("- [ ]")?.trim(),
        ))
    } else {
        None
    }
}

fn parse_markdown_task_identity(
    task_text: &str,
    phase_number: usize,
    next_task_number: usize,
) -> (String, String) {
    if let Some(after_task) = task_text.strip_prefix("Task ") {
        if let Some(colon_pos) = after_task.find(':') {
            let id = after_task[..colon_pos].trim().to_string();
            let desc = after_task[colon_pos + 1..].trim().to_string();
            return (id, desc);
        }
        return (
            format!("{}.{}", phase_number, next_task_number),
            after_task.to_string(),
        );
    }

    (
        format!("{}.{}", phase_number, next_task_number),
        task_text.to_string(),
    )
}
