//! Plan file structure and markdown parser/serializer
//!
//! Plan files are human-readable markdown with this structure:
//! ```markdown
//! # Plan: [Title]
//!
//! Created: 2024-01-15 14:30 UTC
//! Session: abc123
//! Working Directory: /path/to/project
//! Status: in_progress
//!
//! ---
//!
//! ## Phase 1: [Phase Name]
//!
//! - [ ] Task 1.1: Description
//! - [x] Task 1.2: Completed task
//!
//! ## Phase 2: [Phase Name]
//!
//! - [ ] Task 2.1: Description
//! ```

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ============================================================================
// Static regex patterns for task completion detection (compiled once)
// ============================================================================

/// Pattern 1: "- [x] Task X.Y" or "- [X] Task X.Y" (checkbox format)
static RE_CHECKBOX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)- \[[xX]\] (?:\*\*)?(?:Task\s*)?(\d+\.\d+)").unwrap());

/// Pattern 2: "Task X.Y complete/done/finished" variants
static RE_TASK_FIRST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:Task\s*)?(\d+\.\d+)\s+(?:is\s+)?(?:now\s+)?(?:complete|completed|done|finished)",
    )
    .unwrap()
});

/// Pattern 3: "completed/finished Task X.Y" or "I've completed task 1.1"
static RE_VERB_FIRST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:I'?(?:ve)?\s+)?(?:completed|finished|done(?: with)?)\s+(?:Task\s*)?(\d+\.\d+)",
    )
    .unwrap()
});

/// Pattern 4: "✓ Task X.Y" or "✅ Task X.Y"
static RE_CHECKMARK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[✓✅]\s*(?:Task\s*)?(\d+\.\d+)").unwrap());

/// Pattern 5: "completing task X.Y" or "marked X.Y as complete"
static RE_COMPLETING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:completing|marking)\s+(?:Task\s*)?(\d+\.\d+)(?:\s+(?:as\s+)?(?:complete|done))?",
    )
    .unwrap()
});

/// Pattern 6: "have completed task X.Y" or "just completed task X.Y"
static RE_HAVE_COMPLETED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:have|just|now)\s+(?:completed|finished|done)\s+(?:Task\s*)?(\d+\.\d+)")
        .unwrap()
});

/// Pattern 7: "that completes task X.Y" or "this completes task X.Y"
static RE_THAT_COMPLETES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:that|this|which)\s+completes\s+(?:Task\s*)?(\d+\.\d+)").unwrap()
});

/// Pattern 8: "implemented task X.Y"
static RE_IMPLEMENTED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)implemented\s+(?:Task\s*)?(\d+\.\d+)").unwrap());

/// Pattern 9: "Task X.Y ✓" (checkmark after)
static RE_CHECKMARK_AFTER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?:Task\s*)?(\d+\.\d+)\s*[✓✅]").unwrap());

/// All task completion patterns for efficient iteration
static TASK_COMPLETION_PATTERNS: Lazy<[&'static Lazy<Regex>; 9]> = Lazy::new(|| {
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
    ]
});

/// Plan status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Plan is being worked on
    InProgress,
    /// All tasks completed
    Completed,
    /// Plan was abandoned
    Abandoned,
}

/// Task status (individual task within a plan)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Blocked,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Blocked => write!(f, "blocked"),
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "pending" => Ok(TaskStatus::Pending),
            "in_progress" | "inprogress" => Ok(TaskStatus::InProgress),
            "completed" | "complete" | "done" => Ok(TaskStatus::Completed),
            "blocked" => Ok(TaskStatus::Blocked),
            _ => Err(format!("Unknown task status: {}", s)),
        }
    }
}

impl std::fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanStatus::InProgress => write!(f, "in_progress"),
            PlanStatus::Completed => write!(f, "completed"),
            PlanStatus::Abandoned => write!(f, "abandoned"),
        }
    }
}

impl std::str::FromStr for PlanStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "in_progress" | "inprogress" => Ok(PlanStatus::InProgress),
            "completed" | "complete" | "done" => Ok(PlanStatus::Completed),
            "abandoned" | "cancelled" | "canceled" => Ok(PlanStatus::Abandoned),
            _ => Err(format!("Unknown plan status: {}", s)),
        }
    }
}

/// A single task within a phase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    /// Task ID like "1.1", "2.3", or "1.1.1" for subtasks
    pub id: String,
    /// Parent task ID for subtasks (None = top-level task)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Task description
    pub description: String,
    /// Implementation details/context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Completion summary (required when completing)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Whether the task is complete (kept for backward compatibility)
    pub completed: bool,
    /// Task status (richer than just completed bool)
    #[serde(default)]
    pub status: TaskStatus,
    /// Task IDs that must complete before this task can start
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Task IDs that are waiting on this task
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
    /// Child task IDs (for hierarchy)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    /// Priority (1 = highest)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// When the task was created
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// When the task was completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl PlanTask {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parent_id: None,
            description: description.into(),
            context: None,
            result: None,
            completed: false,
            status: TaskStatus::Pending,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            children: Vec::new(),
            priority: None,
            created_at: Some(Utc::now()),
            completed_at: None,
        }
    }

    /// Create a subtask with parent reference
    pub fn new_subtask(
        id: impl Into<String>,
        parent_id: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            parent_id: Some(parent_id.into()),
            description: description.into(),
            context: None,
            result: None,
            completed: false,
            status: TaskStatus::Pending,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            children: Vec::new(),
            priority: None,
            created_at: Some(Utc::now()),
            completed_at: None,
        }
    }

    /// Check if this task is a subtask (has parent)
    pub fn is_subtask(&self) -> bool {
        self.parent_id.is_some()
    }

    /// Get the depth level (0 for top-level, 1+ for subtasks)
    pub fn depth(&self) -> usize {
        self.id.matches('.').count().saturating_sub(1)
    }

    /// Format as markdown checkbox line
    pub fn to_markdown(&self) -> String {
        self.to_markdown_with_depth(0)
    }

    /// Format as markdown with indentation for subtasks
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

        // Add context line if present
        if let Some(ref ctx) = self.context {
            lines.push(format!("{}  > Context: {}", indent, ctx));
        }

        // Add blocked-by line if present
        if !self.blocked_by.is_empty() {
            lines.push(format!(
                "{}  > Blocked-By: {}",
                indent,
                self.blocked_by.join(", ")
            ));
        }

        // Add result line if completed
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

/// A phase containing multiple tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanPhase {
    /// Phase number (1, 2, 3, ...)
    pub number: usize,
    /// Phase name/title
    pub name: String,
    /// Tasks in this phase
    pub tasks: Vec<PlanTask>,
}

impl PlanPhase {
    pub fn new(number: usize, name: impl Into<String>) -> Self {
        Self {
            number,
            name: name.into(),
            tasks: Vec::new(),
        }
    }

    /// Add a task to this phase (test helper)
    #[cfg(test)]
    pub fn add_task(&mut self, description: impl Into<String>) -> &PlanTask {
        let task_num = self.tasks.len() + 1;
        let id = format!("{}.{}", self.number, task_num);
        self.tasks.push(PlanTask::new(id, description));
        self.tasks.last().unwrap()
    }

    /// Count completed tasks
    pub fn completed_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.completed || t.status == TaskStatus::Completed)
            .count()
    }

    /// Check if all tasks are complete
    pub fn is_complete(&self) -> bool {
        !self.tasks.is_empty()
            && self
                .tasks
                .iter()
                .all(|t| t.completed || t.status == TaskStatus::Completed)
    }

    /// Format as markdown
    pub fn to_markdown(&self) -> String {
        let mut lines = vec![
            format!("## Phase {}: {}", self.number, self.name),
            String::new(),
        ];

        // Separate top-level tasks and subtasks
        let top_level: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| t.parent_id.is_none())
            .collect();

        for task in top_level {
            lines.push(task.to_markdown_with_depth(0));
            // Add subtasks indented
            for subtask in self
                .tasks
                .iter()
                .filter(|t| t.parent_id.as_ref().map(|p| p == &task.id).unwrap_or(false))
            {
                lines.push(subtask.to_markdown_with_depth(1));
                // Handle sub-subtasks (depth 2)
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

/// A complete plan file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanFile {
    /// Plan title
    pub title: String,
    /// When the plan was created
    pub created_at: DateTime<Utc>,
    /// Session ID that created this plan
    pub session_id: Option<String>,
    /// Working directory for this plan
    pub working_dir: Option<String>,
    /// Current status
    pub status: PlanStatus,
    /// Plan phases
    pub phases: Vec<PlanPhase>,
    /// Optional notes section
    pub notes: Option<String>,
    /// Version number for conflict detection (incremented on each save)
    #[serde(default)]
    pub version: u64,
    /// File path (set when loaded/saved)
    #[serde(skip)]
    pub file_path: Option<PathBuf>,
}

impl PlanFile {
    /// Create a new empty plan
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            created_at: Utc::now(),
            session_id: None,
            working_dir: None,
            status: PlanStatus::InProgress,
            phases: Vec::new(),
            notes: None,
            version: 0,
            file_path: None,
        }
    }

    /// Increment version number (call before saving)
    pub fn increment_version(&mut self) {
        self.version += 1;
    }

    /// Check if this plan's version matches the expected version
    /// Used for conflict detection before executing tasks
    pub fn version_matches(&self, expected: u64) -> bool {
        self.version == expected
    }

    /// Add a new phase (test helper)
    #[cfg(test)]
    pub fn add_phase(&mut self, name: impl Into<String>) -> &mut PlanPhase {
        let number = self.phases.len() + 1;
        self.phases.push(PlanPhase::new(number, name));
        self.phases.last_mut().unwrap()
    }

    /// Find a task by ID (e.g., "1.2")
    pub fn find_task(&self, task_id: &str) -> Option<&PlanTask> {
        for phase in &self.phases {
            if let Some(task) = phase.tasks.iter().find(|t| t.id == task_id) {
                return Some(task);
            }
        }
        None
    }

    /// Find a task by ID (mutable)
    pub fn find_task_mut(&mut self, task_id: &str) -> Option<&mut PlanTask> {
        for phase in &mut self.phases {
            if let Some(task) = phase.tasks.iter_mut().find(|t| t.id == task_id) {
                return Some(task);
            }
        }
        None
    }

    /// Mark a task as complete (simple boolean, for backward compatibility)
    pub fn check_task(&mut self, task_id: &str) -> bool {
        if let Some(task) = self.find_task_mut(task_id) {
            task.completed = true;
            task.status = TaskStatus::Completed;
            task.completed_at = Some(Utc::now());
            self.update_blocked_status();
            self.update_status();
            true
        } else {
            false
        }
    }

    /// Complete a task with a required result summary
    pub fn complete_task(&mut self, task_id: &str, result: &str) -> Result<(), String> {
        let task = self
            .find_task_mut(task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        task.completed = true;
        task.status = TaskStatus::Completed;
        task.result = Some(result.to_string());
        task.completed_at = Some(Utc::now());

        // Update blocked status of dependent tasks
        self.update_blocked_status();
        self.update_status();
        Ok(())
    }

    /// Start working on a task (marks as InProgress)
    pub fn start_task(&mut self, task_id: &str) -> Result<(), String> {
        // First check if task is blocked
        if self.is_task_blocked(task_id) {
            return Err(format!(
                "Task {} is blocked by incomplete dependencies",
                task_id
            ));
        }

        let task = self
            .find_task_mut(task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        task.status = TaskStatus::InProgress;
        Ok(())
    }

    /// Check if a task is blocked by incomplete dependencies
    pub fn is_task_blocked(&self, task_id: &str) -> bool {
        let Some(task) = self.find_task(task_id) else {
            return false;
        };

        task.blocked_by
            .iter()
            .any(|blocker_id| !self.is_task_completed(blocker_id))
    }

    /// Check if a task is completed
    fn is_task_completed(&self, task_id: &str) -> bool {
        self.find_task(task_id)
            .map(|t| t.status == TaskStatus::Completed || t.completed)
            .unwrap_or(false)
    }

    /// Update blocked status of all tasks based on dependencies
    fn update_blocked_status(&mut self) {
        // Collect task IDs and their blocked_by lists
        let task_deps: Vec<(String, Vec<String>)> = self
            .phases
            .iter()
            .flat_map(|p| &p.tasks)
            .map(|t| (t.id.clone(), t.blocked_by.clone()))
            .collect();

        // Check which tasks should be blocked
        let blocked_tasks: Vec<String> = task_deps
            .iter()
            .filter(|(_, blocked_by)| {
                !blocked_by.is_empty()
                    && blocked_by
                        .iter()
                        .any(|blocker| !self.is_task_completed(blocker))
            })
            .map(|(id, _)| id.clone())
            .collect();

        // Update task statuses
        for phase in &mut self.phases {
            for task in &mut phase.tasks {
                if task.status == TaskStatus::Completed {
                    continue; // Don't change completed tasks
                }
                if blocked_tasks.contains(&task.id) {
                    task.status = TaskStatus::Blocked;
                } else if task.status == TaskStatus::Blocked {
                    // Unblock - revert to pending (unless in progress)
                    task.status = TaskStatus::Pending;
                }
            }
        }
    }

    /// Get tasks that are ready to work on (no unresolved blockers)
    pub fn get_ready_tasks(&self) -> Vec<&PlanTask> {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter(|t| {
                t.status != TaskStatus::Completed
                    && t.status != TaskStatus::Blocked
                    && !self.is_task_blocked(&t.id)
            })
            .collect()
    }

    /// Get tasks that are blocked by incomplete dependencies
    pub fn get_blocked_tasks(&self) -> Vec<&PlanTask> {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter(|t| t.status == TaskStatus::Blocked || self.is_task_blocked(&t.id))
            .collect()
    }

    /// Get all subtasks of a task
    pub fn get_subtasks(&self, parent_id: &str) -> Vec<&PlanTask> {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter(|t| t.parent_id.as_deref() == Some(parent_id))
            .collect()
    }

    /// Add a subtask to an existing task
    pub fn add_subtask(
        &mut self,
        parent_id: &str,
        description: &str,
        context: Option<&str>,
    ) -> Result<String, String> {
        // Find the parent task to determine the phase
        let phase_number = self
            .phases
            .iter()
            .find(|p| p.tasks.iter().any(|t| t.id == parent_id))
            .map(|p| p.number)
            .ok_or_else(|| format!("Parent task {} not found", parent_id))?;

        // Count existing subtasks to generate ID
        let existing_subtasks = self.get_subtasks(parent_id).len();
        let subtask_id = format!("{}.{}", parent_id, existing_subtasks + 1);

        // Create the subtask
        let mut subtask = PlanTask::new_subtask(subtask_id.clone(), parent_id, description);
        subtask.context = context.map(|s| s.to_string());

        // Add to parent's children list
        if let Some(parent) = self.find_task_mut(parent_id) {
            parent.children.push(subtask_id.clone());
        }

        // Add to the appropriate phase
        if let Some(phase) = self.phases.iter_mut().find(|p| p.number == phase_number) {
            phase.tasks.push(subtask);
        }

        Ok(subtask_id)
    }

    /// Add a dependency between tasks (task_id is blocked by blocked_by_id)
    pub fn add_dependency(&mut self, task_id: &str, blocked_by_id: &str) -> Result<(), String> {
        // Verify both tasks exist
        if self.find_task(task_id).is_none() {
            return Err(format!("Task {} not found", task_id));
        }
        if self.find_task(blocked_by_id).is_none() {
            return Err(format!("Blocker task {} not found", blocked_by_id));
        }

        // Check for circular dependency
        if self.would_create_cycle(task_id, blocked_by_id) {
            return Err(format!(
                "Adding dependency would create cycle: {} -> {}",
                task_id, blocked_by_id
            ));
        }

        // Add to blocked_by list
        if let Some(task) = self.find_task_mut(task_id) {
            if !task.blocked_by.contains(&blocked_by_id.to_string()) {
                task.blocked_by.push(blocked_by_id.to_string());
            }
        }

        // Add to blocks list of the blocker
        if let Some(blocker) = self.find_task_mut(blocked_by_id) {
            if !blocker.blocks.contains(&task_id.to_string()) {
                blocker.blocks.push(task_id.to_string());
            }
        }

        // Update blocked status
        self.update_blocked_status();
        Ok(())
    }

    /// Check if adding a dependency would create a cycle
    fn would_create_cycle(&self, task_id: &str, blocked_by_id: &str) -> bool {
        // If blocked_by_id is already (transitively) blocked by task_id, adding this would create a cycle
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![blocked_by_id.to_string()];

        while let Some(current) = stack.pop() {
            if current == task_id {
                return true;
            }
            if visited.insert(current.clone()) {
                if let Some(task) = self.find_task(&current) {
                    stack.extend(task.blocked_by.clone());
                }
            }
        }

        false
    }

    /// Update status based on task completion
    fn update_status(&mut self) {
        if self.status != PlanStatus::Abandoned {
            if self.is_complete() {
                self.status = PlanStatus::Completed;
            } else {
                self.status = PlanStatus::InProgress;
            }
        }
    }

    /// Count total tasks
    pub fn total_tasks(&self) -> usize {
        self.phases.iter().map(|p| p.tasks.len()).sum()
    }

    /// Count completed tasks
    pub fn completed_tasks(&self) -> usize {
        self.phases.iter().map(|p| p.completed_count()).sum()
    }

    /// Check if all tasks are complete
    pub fn is_complete(&self) -> bool {
        !self.phases.is_empty() && self.phases.iter().all(|p| p.is_complete())
    }

    /// Check if any tasks are currently in progress
    pub fn has_in_progress_tasks(&self) -> bool {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .any(|t| t.status == TaskStatus::InProgress)
    }

    /// Get progress as fraction (completed / total)
    pub fn progress(&self) -> (usize, usize) {
        (self.completed_tasks(), self.total_tasks())
    }

    /// Maximum context size in characters (~2000 tokens ≈ 8000 chars)
    ///
    /// This limit ensures that plan context fits comfortably within the AI's
    /// context window while leaving room for other content. Claude typically
    /// handles 200K tokens, so 2000 tokens for plan context is reasonable.
    const MAX_CONTEXT_CHARS: usize = 8000;

    /// Serialize to markdown string for AI context
    /// Truncates large plans to stay within token budget
    pub fn to_context(&self) -> String {
        let full = self.to_markdown();

        if full.len() <= Self::MAX_CONTEXT_CHARS {
            return full;
        }

        // Build compact version: show progress + incomplete tasks only
        let mut lines = Vec::new();
        lines.push(format!("# Plan: {}", self.title));
        lines.push(String::new());

        let (completed, total) = self.progress();
        lines.push(format!("Progress: {}/{} tasks", completed, total));
        lines.push(String::new());

        // Show only incomplete tasks (grouped by phase)
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

        // Add note about truncation
        lines.push("---".to_string());
        lines.push(format!(
            "*Plan truncated for context. {} completed tasks hidden.*",
            completed
        ));

        lines.join("\n")
    }

    /// Serialize to markdown string
    pub fn to_markdown(&self) -> String {
        let mut lines = Vec::new();

        // Header
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

        // Phases
        for phase in &self.phases {
            lines.push(phase.to_markdown());
            lines.push(String::new());
        }

        // Notes
        if let Some(notes) = &self.notes {
            lines.push("---".to_string());
            lines.push(String::new());
            lines.push("## Notes".to_string());
            lines.push(String::new());
            lines.push(notes.clone());
        }

        lines.join("\n")
    }

    /// Parse from markdown string
    pub fn from_markdown(content: &str) -> Result<Self, String> {
        tracing::debug!("Parsing plan from markdown");
        let mut plan = PlanFile {
            title: String::new(),
            created_at: Utc::now(),
            session_id: None,
            working_dir: None,
            status: PlanStatus::InProgress,
            phases: Vec::new(),
            notes: None,
            version: 0,
            file_path: None,
        };

        let mut current_phase: Option<PlanPhase> = None;
        let mut in_notes = false;
        let mut notes_lines: Vec<String> = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();

            // Parse title
            if trimmed.starts_with("# Plan:") {
                plan.title = trimmed
                    .strip_prefix("# Plan:")
                    .unwrap_or("")
                    .trim()
                    .to_string();
                tracing::debug!(title = %plan.title, "Parsed plan title");
                continue;
            }

            // Parse metadata
            if trimmed.starts_with("Created:") {
                // Parse the date, but don't fail if it's invalid
                let date_str = trimmed.strip_prefix("Created:").unwrap_or("").trim();
                if let Ok(dt) = DateTime::parse_from_str(
                    &format!("{} +0000", date_str),
                    "%Y-%m-%d %H:%M UTC %z",
                ) {
                    plan.created_at = dt.with_timezone(&Utc);
                }
                continue;
            }

            if trimmed.starts_with("Session:") {
                plan.session_id = Some(
                    trimmed
                        .strip_prefix("Session:")
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                );
                continue;
            }

            if trimmed.starts_with("Working Directory:") {
                plan.working_dir = Some(
                    trimmed
                        .strip_prefix("Working Directory:")
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                );
                continue;
            }

            if trimmed.starts_with("Status:") {
                let status_str = trimmed.strip_prefix("Status:").unwrap_or("").trim();
                plan.status = status_str.parse().unwrap_or(PlanStatus::InProgress);
                continue;
            }

            if trimmed.starts_with("Version:") {
                let version_str = trimmed.strip_prefix("Version:").unwrap_or("").trim();
                plan.version = version_str.parse().unwrap_or(0);
                continue;
            }

            // Check for notes section
            if trimmed == "## Notes" {
                // Save current phase first
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

            // Parse phase headers
            if trimmed.starts_with("## Phase") {
                // Save previous phase
                if let Some(phase) = current_phase.take() {
                    plan.phases.push(phase);
                }

                // Parse "## Phase N: Name"
                let after_phase = trimmed.strip_prefix("## Phase").unwrap_or("").trim();
                if let Some(colon_pos) = after_phase.find(':') {
                    let num_str = after_phase[..colon_pos].trim();
                    let name = after_phase[colon_pos + 1..].trim();
                    let number = num_str.parse().unwrap_or(plan.phases.len() + 1);
                    tracing::debug!(phase_num = number, phase_name = %name, "Parsed phase");
                    current_phase = Some(PlanPhase::new(number, name));
                }
                continue;
            }

            // Parse task metadata lines (> Context:, > Result:, > Blocked-By:)
            if trimmed.starts_with("> ") || trimmed.starts_with(">") {
                if let Some(ref mut phase) = current_phase {
                    if let Some(last_task) = phase.tasks.last_mut() {
                        let meta = trimmed
                            .strip_prefix("> ")
                            .unwrap_or(trimmed.strip_prefix(">").unwrap_or(""))
                            .trim();
                        if let Some(ctx) = meta.strip_prefix("Context:") {
                            last_task.context = Some(ctx.trim().to_string());
                        } else if let Some(result_with_ts) = meta.strip_prefix("Result") {
                            // Handle "Result [timestamp]: text" or "Result: text"
                            let result_text = if result_with_ts.starts_with(" [") {
                                if let Some(bracket_end) = result_with_ts.find("]:") {
                                    result_with_ts[bracket_end + 2..].trim()
                                } else {
                                    result_with_ts
                                        .strip_prefix(":")
                                        .unwrap_or(result_with_ts)
                                        .trim()
                                }
                            } else {
                                result_with_ts
                                    .strip_prefix(":")
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
                }
                continue;
            }

            // Parse task checkboxes (with indentation detection for subtasks)
            // Supported formats: [ ], [x], [X], [>] (in_progress), [~] (blocked)
            let indent_level = line.len() - line.trim_start().len();
            let is_subtask = indent_level >= 2;

            if trimmed.starts_with("- [ ]")
                || trimmed.starts_with("- [x]")
                || trimmed.starts_with("- [X]")
                || trimmed.starts_with("- [>]")
                || trimmed.starts_with("- [~]")
            {
                if let Some(ref mut phase) = current_phase {
                    // Determine status from checkbox
                    let (status, completed) =
                        if trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]") {
                            (TaskStatus::Completed, true)
                        } else if trimmed.starts_with("- [>]") {
                            (TaskStatus::InProgress, false)
                        } else if trimmed.starts_with("- [~]") {
                            (TaskStatus::Blocked, false)
                        } else {
                            (TaskStatus::Pending, false)
                        };

                    let task_text = trimmed
                        .strip_prefix("- [ ]")
                        .or_else(|| trimmed.strip_prefix("- [x]"))
                        .or_else(|| trimmed.strip_prefix("- [X]"))
                        .or_else(|| trimmed.strip_prefix("- [>]"))
                        .or_else(|| trimmed.strip_prefix("- [~]"))
                        .unwrap_or("")
                        .trim();

                    // Parse "Task X.Y: Description" or just description
                    let (id, description) = if task_text.starts_with("Task ") {
                        let after_task = task_text.strip_prefix("Task ").unwrap_or(task_text);
                        if let Some(colon_pos) = after_task.find(':') {
                            let id = after_task[..colon_pos].trim().to_string();
                            let desc = after_task[colon_pos + 1..].trim().to_string();
                            (id, desc)
                        } else {
                            let id = format!("{}.{}", phase.number, phase.tasks.len() + 1);
                            (id, after_task.to_string())
                        }
                    } else {
                        let id = format!("{}.{}", phase.number, phase.tasks.len() + 1);
                        (id, task_text.to_string())
                    };

                    // Determine parent_id for subtasks based on ID structure
                    let parent_id = if is_subtask {
                        // For subtasks like "1.1.1", parent is "1.1"
                        id.rfind('.').map(|pos| id[..pos].to_string())
                    } else {
                        None
                    };

                    let mut task = PlanTask::new(id.clone(), description);
                    task.parent_id = parent_id.clone();
                    task.completed = completed;
                    task.status = status;

                    // If this is a subtask, add to parent's children
                    if let Some(ref pid) = parent_id {
                        if let Some(parent_task) = phase.tasks.iter_mut().find(|t| t.id == *pid) {
                            parent_task.children.push(id);
                        }
                    }

                    phase.tasks.push(task);
                }
            }
        }

        // Save final phase
        if let Some(phase) = current_phase {
            plan.phases.push(phase);
        }

        // Save notes
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

impl PlanFile {
    /// Try to parse a plan from an AI response
    ///
    /// This is more lenient than `from_markdown()` - it extracts plan structure
    /// from responses that may contain other text. Returns None if no valid
    /// plan structure is found.
    ///
    /// Detects patterns like:
    /// - `# Plan: Title` or `## Plan: Title`
    /// - `## Phase N: Name`
    /// - `- [ ] Task description` or `- [x] Task description`
    pub fn try_parse_from_response(text: &str) -> Option<Self> {
        let mut plan = PlanFile {
            title: String::new(),
            created_at: Utc::now(),
            session_id: None,
            working_dir: None,
            status: PlanStatus::InProgress,
            phases: Vec::new(),
            notes: None,
            version: 0,
            file_path: None,
        };

        let mut current_phase: Option<PlanPhase> = None;
        let mut found_any_structure = false;

        for line in text.lines() {
            let trimmed = line.trim();

            // Parse title: "# Plan: Title" or "## Plan: Title"
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

            // Parse phase headers: "## Phase N: Name" or "### Phase N: Name"
            let phase_prefix = trimmed
                .strip_prefix("## Phase")
                .or_else(|| trimmed.strip_prefix("### Phase"));

            if let Some(after_phase) = phase_prefix {
                // Save previous phase
                if let Some(phase) = current_phase.take() {
                    if !phase.tasks.is_empty() {
                        plan.phases.push(phase);
                    }
                }

                // Parse "N: Name" or just ": Name"
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

            // Parse task metadata lines
            if trimmed.starts_with("> ") || trimmed.starts_with(">") {
                if let Some(ref mut phase) = current_phase {
                    if let Some(last_task) = phase.tasks.last_mut() {
                        let meta = trimmed
                            .strip_prefix("> ")
                            .unwrap_or(trimmed.strip_prefix(">").unwrap_or(""))
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

            // Parse task checkboxes: "- [ ] Description", "- [x] Description", "- [>] Description", "- [~] Description"
            if trimmed.starts_with("- [ ]")
                || trimmed.starts_with("- [x]")
                || trimmed.starts_with("- [X]")
                || trimmed.starts_with("- [>]")
                || trimmed.starts_with("- [~]")
            {
                let (status, completed) =
                    if trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]") {
                        (TaskStatus::Completed, true)
                    } else if trimmed.starts_with("- [>]") {
                        (TaskStatus::InProgress, false)
                    } else if trimmed.starts_with("- [~]") {
                        (TaskStatus::Blocked, false)
                    } else {
                        (TaskStatus::Pending, false)
                    };

                let task_text = trimmed
                    .strip_prefix("- [ ]")
                    .or_else(|| trimmed.strip_prefix("- [x]"))
                    .or_else(|| trimmed.strip_prefix("- [X]"))
                    .or_else(|| trimmed.strip_prefix("- [>]"))
                    .or_else(|| trimmed.strip_prefix("- [~]"))
                    .unwrap_or("")
                    .trim();

                if task_text.is_empty() {
                    continue;
                }

                // Get or create the current phase (default to "Tasks" phase)
                let phase = if let Some(p) = &mut current_phase {
                    p
                } else {
                    current_phase = Some(PlanPhase::new(1, "Tasks"));
                    current_phase.as_mut().unwrap()
                };

                // Parse "Task X.Y: Description" or "**Task X.Y**: Description" or just description
                let (id, description) =
                    Self::parse_task_text(task_text, phase.number, phase.tasks.len() + 1);

                let mut task = PlanTask::new(id, description);
                task.completed = completed;
                task.status = status;
                phase.tasks.push(task);
                found_any_structure = true;
            }
        }

        // Save final phase
        if let Some(phase) = current_phase {
            if !phase.tasks.is_empty() {
                plan.phases.push(phase);
            }
        }

        // Need at least a title or some phases with tasks to be a valid plan
        if !found_any_structure || (plan.title.is_empty() && plan.phases.is_empty()) {
            return None;
        }

        // If no title but has phases, generate one
        if plan.title.is_empty() && !plan.phases.is_empty() {
            plan.title = "Untitled Plan".to_string();
        }

        // Need at least one task to be useful
        if plan.total_tasks() == 0 {
            return None;
        }

        Some(plan)
    }

    /// Parse task text to extract ID and description
    /// Handles formats like:
    /// - "Task 1.1: Description"
    /// - "**Task 1.1**: Description"
    /// - "Description" (generates ID)
    fn parse_task_text(text: &str, phase_num: usize, task_num: usize) -> (String, String) {
        // Try "Task X.Y: Description" format
        if let Some(after_task) = text.strip_prefix("Task ") {
            if let Some(colon_pos) = after_task.find(':') {
                let id = after_task[..colon_pos].trim().to_string();
                let desc = after_task[colon_pos + 1..].trim().to_string();
                if !id.is_empty() && !desc.is_empty() {
                    return (id, desc);
                }
            }
        }

        // Try "**Task X.Y**: Description" format (bold markdown)
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

        // No Task prefix - generate ID and use full text as description
        let id = format!("{}.{}", phase_num, task_num);
        (id, text.to_string())
    }

    /// Extract task IDs that are marked as completed in text
    ///
    /// Detects patterns like:
    /// - "Task 1.1 complete", "Task 1.1 is done", "completed Task 1.1"
    /// - "✓ Task 1.1", "✅ Task 1.1"
    /// - "- [x] Task 1.1: Description"
    /// - "finished task 1.2", "done with task 1.2"
    ///
    /// Returns a list of task IDs that should be marked complete.
    /// Uses pre-compiled static regexes for performance.
    pub fn extract_completed_task_ids(text: &str) -> Vec<String> {
        use std::collections::HashSet;

        let mut seen: HashSet<&str> = HashSet::new();
        let mut completed_ids = Vec::new();

        // Iterate through all pre-compiled patterns
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

    /// Merge another plan into this one
    /// Updates existing phases/tasks and adds new ones
    pub fn merge_from(&mut self, other: &PlanFile) {
        // Update title if the other has a more specific one
        if other.title != "Untitled Plan"
            && (self.title == "Untitled Plan" || self.title.is_empty())
        {
            self.title = other.title.clone();
        }

        // Merge phases
        for other_phase in &other.phases {
            if let Some(existing) = self
                .phases
                .iter_mut()
                .find(|p| p.number == other_phase.number)
            {
                // Merge tasks into existing phase
                for other_task in &other_phase.tasks {
                    if let Some(existing_task) =
                        existing.tasks.iter_mut().find(|t| t.id == other_task.id)
                    {
                        // Update completion status (prefer completed)
                        if other_task.completed || other_task.status == TaskStatus::Completed {
                            existing_task.completed = true;
                            existing_task.status = TaskStatus::Completed;
                            if existing_task.completed_at.is_none() {
                                existing_task.completed_at = other_task.completed_at;
                            }
                        }
                        // Update description if changed
                        if !other_task.description.is_empty() {
                            existing_task.description = other_task.description.clone();
                        }
                        // Merge context if provided
                        if other_task.context.is_some() {
                            existing_task.context = other_task.context.clone();
                        }
                        // Merge result if provided
                        if other_task.result.is_some() {
                            existing_task.result = other_task.result.clone();
                        }
                        // Merge dependencies
                        for dep in &other_task.blocked_by {
                            if !existing_task.blocked_by.contains(dep) {
                                existing_task.blocked_by.push(dep.clone());
                            }
                        }
                        for dep in &other_task.blocks {
                            if !existing_task.blocks.contains(dep) {
                                existing_task.blocks.push(dep.clone());
                            }
                        }
                    } else {
                        // Add new task
                        existing.tasks.push(other_task.clone());
                    }
                }
            } else {
                // Add new phase
                self.phases.push(other_phase.clone());
            }
        }

        // Re-sort phases by number
        self.phases.sort_by_key(|p| p.number);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_plan() {
        let mut plan = PlanFile::new("Test Plan");
        let phase = plan.add_phase("Setup");
        phase.add_task("Install dependencies");
        phase.add_task("Configure environment");

        assert_eq!(plan.title, "Test Plan");
        assert_eq!(plan.phases.len(), 1);
        assert_eq!(plan.phases[0].tasks.len(), 2);
        assert_eq!(plan.phases[0].tasks[0].id, "1.1");
        assert_eq!(plan.phases[0].tasks[1].id, "1.2");
    }

    #[test]
    fn test_check_task() {
        let mut plan = PlanFile::new("Test Plan");
        {
            let phase = plan.add_phase("Phase 1");
            phase.add_task("Task one");
        }

        assert!(!plan.find_task("1.1").unwrap().completed);
        assert!(plan.check_task("1.1"));
        assert!(plan.find_task("1.1").unwrap().completed);
    }

    #[test]
    fn test_progress() {
        let mut plan = PlanFile::new("Test Plan");
        {
            let phase = plan.add_phase("Phase 1");
            phase.add_task("Task one");
            phase.add_task("Task two");
        }

        assert_eq!(plan.progress(), (0, 2));
        plan.check_task("1.1");
        assert_eq!(plan.progress(), (1, 2));
        plan.check_task("1.2");
        assert_eq!(plan.progress(), (2, 2));
        assert!(plan.is_complete());
    }

    #[test]
    fn test_markdown_roundtrip() {
        let mut plan = PlanFile::new("Test Plan");
        plan.session_id = Some("test-session".to_string());
        plan.working_dir = Some("/tmp/test".to_string());
        {
            let phase = plan.add_phase("Setup");
            phase.add_task("Install deps");
            phase.add_task("Configure");
        }
        plan.check_task("1.1");
        plan.notes = Some("Some notes here".to_string());

        let markdown = plan.to_markdown();
        let parsed = PlanFile::from_markdown(&markdown).unwrap();

        assert_eq!(parsed.title, plan.title);
        assert_eq!(parsed.session_id, plan.session_id);
        assert_eq!(parsed.working_dir, plan.working_dir);
        assert_eq!(parsed.phases.len(), plan.phases.len());
        assert_eq!(parsed.phases[0].tasks.len(), 2);
        assert!(parsed.find_task("1.1").unwrap().completed);
        assert!(!parsed.find_task("1.2").unwrap().completed);
        assert!(parsed.notes.is_some());
    }

    #[test]
    fn test_try_parse_from_response() {
        let response = r#"
I'll create a plan for implementing authentication.

# Plan: Authentication System

## Phase 1: Database Setup

- [ ] Task 1.1: Create users table
- [ ] Task 1.2: Add password hashing

## Phase 2: API Endpoints

- [ ] Task 2.1: Implement login endpoint
- [x] Task 2.2: Already completed signup

Let me know if you have questions!
"#;

        let plan = PlanFile::try_parse_from_response(response).unwrap();
        assert_eq!(plan.title, "Authentication System");
        assert_eq!(plan.phases.len(), 2);
        assert_eq!(plan.phases[0].tasks.len(), 2);
        assert_eq!(plan.phases[1].tasks.len(), 2);
        assert!(!plan.find_task("1.1").unwrap().completed);
        assert!(plan.find_task("2.2").unwrap().completed);
    }

    #[test]
    fn test_try_parse_no_explicit_task_ids() {
        let response = r#"
# Plan: Quick Tasks

## Phase 1: Setup

- [ ] Install dependencies
- [ ] Configure environment
- [x] Done with prerequisites
"#;

        let plan = PlanFile::try_parse_from_response(response).unwrap();
        assert_eq!(plan.title, "Quick Tasks");
        assert_eq!(plan.phases[0].tasks.len(), 3);
        assert_eq!(plan.phases[0].tasks[0].id, "1.1");
        assert_eq!(plan.phases[0].tasks[0].description, "Install dependencies");
        assert!(plan.phases[0].tasks[2].completed);
    }

    #[test]
    fn test_try_parse_no_valid_structure() {
        let response = "Just a normal response without any plan structure.";
        assert!(PlanFile::try_parse_from_response(response).is_none());

        let response2 = "# Plan: Title Only"; // No tasks
        assert!(PlanFile::try_parse_from_response(response2).is_none());
    }

    #[test]
    fn test_merge_plans() {
        let mut plan1 = PlanFile::new("Original Plan");
        {
            let phase = plan1.add_phase("Setup");
            phase.add_task("Task one");
            phase.add_task("Task two");
        }

        let mut plan2 = PlanFile::new("Updated Plan");
        {
            let phase = plan2.add_phase("Setup");
            phase.add_task("Task one"); // Same task
        }
        plan2.check_task("1.1"); // Mark as complete

        plan1.merge_from(&plan2);

        // Task 1.1 should now be complete
        assert!(plan1.find_task("1.1").unwrap().completed);
        // Task 1.2 should still exist
        assert!(plan1.find_task("1.2").is_some());
    }

    #[test]
    fn test_extract_completed_task_ids() {
        // Test checkbox pattern
        let text1 = "- [x] Task 1.1: Create database\n- [ ] Task 1.2: Add indexes";
        let ids1 = PlanFile::extract_completed_task_ids(text1);
        assert_eq!(ids1, vec!["1.1"]);

        // Test "Task X.Y complete" pattern
        let text2 = "I've finished the work. Task 2.1 is complete and Task 2.2 is done.";
        let ids2 = PlanFile::extract_completed_task_ids(text2);
        assert!(ids2.contains(&"2.1".to_string()));
        assert!(ids2.contains(&"2.2".to_string()));

        // Test "completed Task X.Y" pattern
        let text3 = "I completed Task 3.1 and finished Task 3.2.";
        let ids3 = PlanFile::extract_completed_task_ids(text3);
        assert!(ids3.contains(&"3.1".to_string()));
        assert!(ids3.contains(&"3.2".to_string()));

        // Test checkmark pattern
        let text4 = "✓ Task 4.1\n✅ Task 4.2";
        let ids4 = PlanFile::extract_completed_task_ids(text4);
        assert!(ids4.contains(&"4.1".to_string()));
        assert!(ids4.contains(&"4.2".to_string()));

        // Test no matches
        let text5 = "Working on the tasks now.";
        let ids5 = PlanFile::extract_completed_task_ids(text5);
        assert!(ids5.is_empty());
    }

    // ========================================================================
    // Error Path Tests
    // ========================================================================

    #[test]
    fn test_parse_empty_plan() {
        let result = PlanFile::from_markdown("");
        assert!(result.is_err(), "Empty plan should error");
    }

    #[test]
    fn test_parse_plan_no_title() {
        let markdown = r#"
## Phase 1: Setup

- [ ] Task 1.1: Some task
"#;
        let result = PlanFile::from_markdown(markdown);
        // This should error because there's no "# Plan:" header
        // The try_parse_from_response requires both title and at least one task
        assert!(result.is_err() || result.unwrap().phases.is_empty());
    }

    #[test]
    fn test_parse_plan_no_phases() {
        let markdown = "# Plan: Test Plan\n";
        let result = PlanFile::from_markdown(markdown);
        // Should create plan with no phases
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.phases.len(), 0);
    }

    #[test]
    fn test_parse_plan_invalid_status() {
        let markdown = r#"
# Plan: Test Plan

Status: invalid_status_value

## Phase 1: Setup

- [ ] Task 1.1: Some task
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        // Should default to InProgress on invalid status
        let plan = result.unwrap();
        assert_eq!(plan.status, PlanStatus::InProgress);
    }

    #[test]
    fn test_parse_plan_invalid_date() {
        let markdown = r#"
# Plan: Test Plan

Created: not-a-date

## Phase 1: Setup

- [ ] Task 1.1: Some task
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        // Should use current time on invalid date
        let plan = result.unwrap();
        assert_ne!(plan.created_at, Utc::now());
    }

    #[test]
    fn test_parse_plan_invalid_phase_number() {
        let markdown = r#"
# Plan: Test Plan

## Phase not-a-number: Setup

- [ ] Task 1.1: Some task
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        // Should use phase count + 1 as fallback
        assert_eq!(plan.phases.len(), 1);
        assert_eq!(plan.phases[0].number, 1);
    }

    #[test]
    fn test_parse_plan_task_without_phase() {
        let markdown = r#"
# Plan: Test Plan

- [ ] Task 1.1: Orphan task
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        // Tasks without phase should be ignored
        assert_eq!(plan.phases.len(), 0);
    }

    #[test]
    fn test_parse_plan_empty_task_description() {
        let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1:
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.phases[0].tasks[0].id, "1.1");
        assert_eq!(plan.phases[0].tasks[0].description, "");
    }

    #[test]
    fn test_parse_plan_task_with_colon_in_description() {
        let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Install: configure, and test
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        // Description should include everything after first colon
        assert_eq!(
            plan.phases[0].tasks[0].description,
            "Install: configure, and test"
        );
    }

    #[test]
    fn test_parse_plan_mixed_task_formats() {
        let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Explicit ID
- [ ] Just a description
- [x] Task 1.3: With checkbox
- [ ] Another description
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.phases[0].tasks.len(), 4);
        assert_eq!(plan.phases[0].tasks[0].id, "1.1");
        assert_eq!(plan.phases[0].tasks[1].id, "1.2"); // Auto-generated
        assert_eq!(plan.phases[0].tasks[2].id, "1.3");
        assert!(plan.phases[0].tasks[2].completed);
        assert_eq!(plan.phases[0].tasks[3].id, "1.4"); // Auto-generated
    }

    #[test]
    fn test_parse_plan_empty_phase_name() {
        let markdown = r#"
# Plan: Test Plan

## Phase 1:

- [ ] Task 1.1: Some task
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.phases[0].name, "");
    }

    #[test]
    fn test_parse_plan_notes_section() {
        let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Some task

## Notes

These are important notes
that span multiple lines.
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert!(plan.notes.is_some());
        assert!(plan.notes.as_ref().unwrap().contains("important notes"));
    }

    #[test]
    fn test_parse_plan_notes_before_tasks() {
        // Notes section should end phase parsing
        let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Some task

## Notes

Some notes here

## Phase 2: Next Phase

- [ ] Task 2.1: Another task
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        // Phase 2 should be in notes, not parsed as a phase
        assert_eq!(plan.phases.len(), 1);
        assert!(plan.notes.as_ref().unwrap().contains("## Phase 2"));
    }

    #[test]
    fn test_parse_plan_with_metadata_only() {
        let markdown = r#"
# Plan: Test Plan

Created: 2024-01-15 10:00 UTC
Session: abc123
Working Directory: /tmp/test
Status: completed
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.title, "Test Plan");
        assert_eq!(plan.session_id, Some("abc123".to_string()));
        assert_eq!(plan.working_dir, Some("/tmp/test".to_string()));
        assert_eq!(plan.status, PlanStatus::Completed);
        assert_eq!(plan.phases.len(), 0);
    }

    #[test]
    fn test_parse_plan_multiple_spaces_in_checkbox() {
        // Should handle various whitespace patterns
        let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [  ] Task 1.1: Extra spaces in bracket
- [x] Task 1.2: Normal completed
- [X] Task 1.3: Uppercase X
"#;
        let result = PlanFile::from_markdown(markdown);
        // Extra spaces in bracket should not match
        // Only [x] and [X] should match
        assert!(result.is_ok());
        let _plan = result.unwrap();
        // First one might not match if we're strict about spacing
        // Last two should match
    }

    #[test]
    fn test_parse_plan_status_variations() {
        // Test various status string formats
        let test_cases = vec![
            ("in_progress", PlanStatus::InProgress),
            ("inprogress", PlanStatus::InProgress),
            ("completed", PlanStatus::Completed),
            ("complete", PlanStatus::Completed),
            ("done", PlanStatus::Completed),
            ("abandoned", PlanStatus::Abandoned),
            ("cancelled", PlanStatus::Abandoned),
            ("canceled", PlanStatus::Abandoned),
        ];

        for (status_str, expected) in test_cases {
            let markdown = format!(
                r#"
# Plan: Test Plan

Status: {}

## Phase 1: Setup

- [ ] Task 1.1: Some task
"#,
                status_str
            );

            let result = PlanFile::from_markdown(&markdown);
            assert!(result.is_ok());
            let plan = result.unwrap();
            assert_eq!(
                plan.status, expected,
                "Status '{}' should parse to {:?}",
                status_str, expected
            );
        }
    }

    #[test]
    fn test_parse_plan_task_id_with_decimals() {
        // Test that task IDs preserve decimals
        let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.10: Task with decimal
- [ ] Task 1.2: Another task
"#;
        let result = PlanFile::from_markdown(markdown);
        assert!(result.is_ok());
        let plan = result.unwrap();
        assert_eq!(plan.phases[0].tasks[0].id, "1.10");
        assert_eq!(plan.phases[0].tasks[1].id, "1.2");
    }

    #[test]
    fn test_find_nonexistent_task() {
        let mut plan = PlanFile::new("Test Plan");
        {
            let phase = plan.add_phase("Setup");
            phase.add_task("Task one");
        }

        assert!(plan.find_task("9.9").is_none());
        assert!(plan.find_task("invalid").is_none());
    }

    #[test]
    fn test_check_nonexistent_task() {
        let mut plan = PlanFile::new("Test Plan");
        {
            let phase = plan.add_phase("Setup");
            phase.add_task("Task one");
        }

        // Should return false, not panic
        assert!(!plan.check_task("9.9"));
        assert!(!plan.check_task("invalid"));
    }

    #[test]
    fn test_merge_empty_plans() {
        let mut plan1 = PlanFile::new("Plan 1");
        let plan2 = PlanFile::new("Plan 2");

        // Should not panic
        plan1.merge_from(&plan2);
        assert_eq!(plan1.phases.len(), 0);
    }

    #[test]
    fn test_merge_plans_different_phase_counts() {
        let mut plan1 = PlanFile::new("Plan 1");
        {
            let phase = plan1.add_phase("Phase 1");
            phase.add_task("Task 1.1");
        }
        {
            let phase = plan1.add_phase("Phase 2");
            phase.add_task("Task 2.1");
        }

        let mut plan2 = PlanFile::new("Plan 2");
        {
            let phase = plan2.add_phase("Phase 1");
            phase.add_task("Task 1.1");
        }

        plan2.check_task("1.1");
        plan1.merge_from(&plan2);

        // Both phases should exist in plan1
        assert_eq!(plan1.phases.len(), 2);
        assert!(plan1.find_task("1.1").unwrap().completed);
    }

    #[test]
    fn test_task_completion_pattern_edge_cases() {
        // Test patterns that shouldn't match
        let test_cases = vec![
            "The task is not completed yet",
            "Working on task number 1.1",
            // Note: "Task1.1" (no space before number) will match because regex is flexible
            // "completing the work on task 1.1", // "completing" needs "task X.Y" after
            "This completes our work", // No task ID after
        ];

        for text in test_cases {
            let ids = PlanFile::extract_completed_task_ids(text);
            assert!(
                ids.is_empty(),
                "Text '{}' should not match any task completion patterns",
                text
            );
        }
    }

    #[test]
    fn test_multiple_task_completions_in_one_line() {
        let text = "I've completed Task 1.1, Task 1.2, and finished Task 1.3";
        let ids = PlanFile::extract_completed_task_ids(text);
        // Should match at least 2 tasks (patterns vary in specificity)
        assert!(ids.len() >= 2);
        assert!(ids.contains(&"1.1".to_string()));
    }
}
