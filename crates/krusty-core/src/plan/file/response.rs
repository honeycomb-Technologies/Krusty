use once_cell::sync::Lazy;
use regex::Regex;

use super::{PlanFile, PlanPhase, PlanTask, TaskStatus};

static RE_CHECKBOX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)- \[[xX]\] (?:\*\*)?(?:Task\s*)?(\d+\.\d+)").expect("RE_CHECKBOX: valid regex")
});
static RE_TASK_FIRST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:Task\s*)?(\d+\.\d+)\s+(?:is\s+)?(?:now\s+)?(?:complete|completed|done|finished)",
    )
    .expect("valid regex pattern")
});
static RE_VERB_FIRST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:I'?(?:ve)?\s+)?(?:completed|finished|done(?: with)?)\s+(?:Task\s*)?(\d+\.\d+)",
    )
    .expect("valid regex pattern")
});
static RE_CHECKMARK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[✓✅]\s*(?:Task\s*)?(\d+\.\d+)").expect("valid regex pattern"));
static RE_COMPLETING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:completing|marking)\s+(?:Task\s*)?(\d+\.\d+)(?:\s+(?:as\s+)?(?:complete|done))?",
    )
    .expect("valid regex pattern")
});
static RE_HAVE_COMPLETED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:have|just|now)\s+(?:completed|finished|done)\s+(?:Task\s*)?(\d+\.\d+)")
        .expect("valid regex pattern")
});
static RE_THAT_COMPLETES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:that|this|which)\s+completes\s+(?:Task\s*)?(\d+\.\d+)")
        .expect("valid regex pattern")
});
static RE_IMPLEMENTED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)implemented\s+(?:Task\s*)?(\d+\.\d+)").expect("valid regex pattern")
});
static RE_CHECKMARK_AFTER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?:Task\s*)?(\d+\.\d+)\s*[✓✅]").expect("valid regex pattern"));
static RE_TASK_TRAILING_DONE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)(?:Task\s*)?(\d+\.\d+)[:\s].*?\b(?:DONE|done|COMPLETE|complete)\s*$")
        .expect("valid regex pattern")
});
static RE_TASK_DASH_STATUS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)(?:Task\s*)?(\d+\.\d+)[:\s].*?[—–-]\s*(?:DONE|done|complete|completed)\s*$")
        .expect("valid regex pattern")
});
static TASK_COMPLETION_PATTERNS: Lazy<[&'static Lazy<Regex>; 11]> = Lazy::new(|| {
    [
        &RE_CHECKBOX,
        &RE_TASK_FIRST,
        &RE_VERB_FIRST,
        &RE_CHECKMARK,
        &RE_COMPLETING,
        &RE_HAVE_COMPLETED,
        &RE_THAT_COMPLETES,
        &RE_IMPLEMENTED,
        &RE_CHECKMARK_AFTER,
        &RE_TASK_TRAILING_DONE,
        &RE_TASK_DASH_STATUS,
    ]
});

impl PlanFile {
    /// Try to parse a plan from an AI response.
    pub fn try_parse_from_response(text: &str) -> Option<Self> {
        let mut plan = PlanFile::new(String::new());
        let mut current_phase: Option<PlanPhase> = None;
        let mut found_any_structure = false;

        for line in text.lines() {
            let trimmed = line.trim();

            if plan.title.is_empty() {
                if let Some(title) = trimmed
                    .strip_prefix("# Plan:")
                    .or_else(|| trimmed.strip_prefix("## Plan:"))
                {
                    plan.title = title.trim().to_string();
                    found_any_structure = true;
                    continue;
                }
            }

            let phase_prefix = trimmed
                .strip_prefix("## Phase")
                .or_else(|| trimmed.strip_prefix("### Phase"));
            if let Some(after_phase) = phase_prefix {
                if let Some(phase) = current_phase.take() {
                    if !phase.tasks.is_empty() {
                        plan.phases.push(phase);
                    }
                }

                let after_phase = after_phase.trim();
                if let Some(colon_pos) = after_phase.find(':') {
                    let num_str = after_phase[..colon_pos].trim();
                    let name = after_phase[colon_pos + 1..].trim();
                    let number = num_str.parse().unwrap_or(plan.phases.len() + 1);
                    current_phase = Some(PlanPhase::new(number, name));
                    found_any_structure = true;
                }
                continue;
            }

            if trimmed.starts_with("> ") || trimmed.starts_with('>') {
                if let Some(ref mut phase) = current_phase {
                    if let Some(last_task) = phase.tasks.last_mut() {
                        let meta = trimmed
                            .strip_prefix("> ")
                            .unwrap_or(trimmed.strip_prefix('>').unwrap_or(""))
                            .trim();
                        if let Some(ctx) = meta.strip_prefix("Context:") {
                            last_task.context = Some(ctx.trim().to_string());
                        } else if let Some(result_text) = meta.strip_prefix("Result:") {
                            last_task.result = Some(result_text.trim().to_string());
                        } else if let Some(blocked) = meta.strip_prefix("Blocked-By:") {
                            last_task.blocked_by = blocked
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                    }
                }
                continue;
            }

            if let Some((status, completed, task_text)) = parse_response_checkbox_line(trimmed) {
                if task_text.is_empty() {
                    continue;
                }

                let phase = if let Some(p) = &mut current_phase {
                    p
                } else {
                    current_phase = Some(PlanPhase::new(1, "Tasks"));
                    current_phase.as_mut().expect("default phase just created")
                };

                let (id, description) =
                    Self::parse_task_text(task_text, phase.number, phase.tasks.len() + 1);

                let mut task = PlanTask::new(id, description);
                task.completed = completed;
                task.status = status;
                phase.tasks.push(task);
                found_any_structure = true;
            }
        }

        if let Some(phase) = current_phase {
            if !phase.tasks.is_empty() {
                plan.phases.push(phase);
            }
        }

        if !found_any_structure || (plan.title.is_empty() && plan.phases.is_empty()) {
            return None;
        }

        if plan.title.is_empty() && !plan.phases.is_empty() {
            plan.title = "Untitled Plan".to_string();
        }

        if plan.total_tasks() == 0 {
            return None;
        }

        Some(plan)
    }

    fn parse_task_text(text: &str, phase_num: usize, task_num: usize) -> (String, String) {
        if let Some(after_task) = text.strip_prefix("Task ") {
            if let Some(colon_pos) = after_task.find(':') {
                let id = after_task[..colon_pos].trim().to_string();
                let desc = after_task[colon_pos + 1..].trim().to_string();
                if !id.is_empty() && !desc.is_empty() {
                    return (id, desc);
                }
            }
        }

        if let Some(after_task) = text.strip_prefix("**Task ") {
            if let Some(end_bold) = after_task.find("**") {
                let id = after_task[..end_bold].trim().to_string();
                let rest = after_task[end_bold + 2..].trim();
                let desc = rest.strip_prefix(':').unwrap_or(rest).trim().to_string();
                if !id.is_empty() && !desc.is_empty() {
                    return (id, desc);
                }
            }
        }

        let id = format!("{}.{}", phase_num, task_num);
        (id, text.to_string())
    }

    /// Extract task IDs that are marked as completed in text.
    pub fn extract_completed_task_ids(text: &str) -> Vec<String> {
        use std::collections::HashSet;

        let mut seen: HashSet<&str> = HashSet::new();
        let mut completed_ids = Vec::new();

        for pattern in TASK_COMPLETION_PATTERNS.iter() {
            for cap in pattern.captures_iter(text) {
                if let Some(id) = cap.get(1) {
                    let id_str = id.as_str();
                    if seen.insert(id_str) {
                        completed_ids.push(id_str.to_string());
                    }
                }
            }
        }

        completed_ids
    }
}

fn parse_response_checkbox_line(trimmed: &str) -> Option<(TaskStatus, bool, &str)> {
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
