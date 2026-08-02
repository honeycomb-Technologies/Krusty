use chrono::Utc;

use super::{PlanFile, PlanStatus, PlanTask, TaskStatus};

impl PlanFile {
    /// Find a task by ID (e.g. "1.2").
    pub fn find_task(&self, task_id: &str) -> Option<&PlanTask> {
        for phase in &self.phases {
            if let Some(task) = phase.tasks.iter().find(|t| t.id == task_id) {
                return Some(task);
            }
        }
        None
    }

    /// Find a task by ID (mutable).
    pub fn find_task_mut(&mut self, task_id: &str) -> Option<&mut PlanTask> {
        for phase in &mut self.phases {
            if let Some(task) = phase.tasks.iter_mut().find(|t| t.id == task_id) {
                return Some(task);
            }
        }
        None
    }

    /// Mark a task as complete (simple boolean, for backward compatibility).
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

    /// Complete a task with a required result summary.
    pub fn complete_task(&mut self, task_id: &str, result: &str) -> Result<(), String> {
        let task = self
            .find_task_mut(task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        task.completed = true;
        task.status = TaskStatus::Completed;
        task.result = Some(result.to_string());
        task.completed_at = Some(Utc::now());

        self.update_blocked_status();
        self.update_status();
        Ok(())
    }

    /// Start working on a task (marks as InProgress).
    pub fn start_task(&mut self, task_id: &str) -> Result<(), String> {
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

    /// Check if a task is blocked by incomplete dependencies.
    pub fn is_task_blocked(&self, task_id: &str) -> bool {
        let Some(task) = self.find_task(task_id) else {
            return false;
        };

        task.blocked_by
            .iter()
            .any(|blocker_id| !self.is_task_completed(blocker_id))
    }

    fn is_task_completed(&self, task_id: &str) -> bool {
        self.find_task(task_id)
            .map(|t| t.status == TaskStatus::Completed || t.completed)
            .unwrap_or(false)
    }

    fn update_blocked_status(&mut self) {
        let completed_tasks: std::collections::HashSet<String> = self
            .phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter(|t| t.status == TaskStatus::Completed || t.completed)
            .map(|t| t.id.clone())
            .collect();

        let blocked_tasks: std::collections::HashSet<String> = self
            .phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter(|task| {
                !task.blocked_by.is_empty()
                    && task
                        .blocked_by
                        .iter()
                        .any(|blocker| !completed_tasks.contains(blocker))
            })
            .map(|task| task.id.clone())
            .collect();

        for phase in &mut self.phases {
            for task in &mut phase.tasks {
                if task.status == TaskStatus::Completed {
                    continue;
                }
                if blocked_tasks.contains(&task.id) {
                    task.status = TaskStatus::Blocked;
                } else if task.status == TaskStatus::Blocked {
                    task.status = TaskStatus::Pending;
                }
            }
        }
    }

    /// Get tasks that are ready to work on (no unresolved blockers).
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

    /// Get tasks that are blocked by incomplete dependencies.
    pub fn get_blocked_tasks(&self) -> Vec<&PlanTask> {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter(|t| t.status == TaskStatus::Blocked || self.is_task_blocked(&t.id))
            .collect()
    }

    /// Get all subtasks of a task.
    pub fn get_subtasks(&self, parent_id: &str) -> Vec<&PlanTask> {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter(|t| t.parent_id.as_deref() == Some(parent_id))
            .collect()
    }

    /// Add a subtask to an existing task.
    pub fn add_subtask(
        &mut self,
        parent_id: &str,
        description: &str,
        context: Option<&str>,
    ) -> Result<String, String> {
        let phase_number = self
            .phases
            .iter()
            .find(|p| p.tasks.iter().any(|t| t.id == parent_id))
            .map(|p| p.number)
            .ok_or_else(|| format!("Parent task {} not found", parent_id))?;

        let existing_subtasks = self.get_subtasks(parent_id).len();
        let subtask_id = format!("{}.{}", parent_id, existing_subtasks + 1);

        let mut subtask = PlanTask::new_subtask(subtask_id.clone(), parent_id, description);
        subtask.context = context.map(|s| s.to_string());

        if let Some(parent) = self.find_task_mut(parent_id) {
            parent.children.push(subtask_id.clone());
        }

        if let Some(phase) = self.phases.iter_mut().find(|p| p.number == phase_number) {
            phase.tasks.push(subtask);
        }

        Ok(subtask_id)
    }

    /// Add a dependency between tasks (task_id is blocked by blocked_by_id).
    pub fn add_dependency(&mut self, task_id: &str, blocked_by_id: &str) -> Result<(), String> {
        if self.find_task(task_id).is_none() {
            return Err(format!("Task {} not found", task_id));
        }
        if self.find_task(blocked_by_id).is_none() {
            return Err(format!("Blocker task {} not found", blocked_by_id));
        }

        if self.would_create_cycle(task_id, blocked_by_id) {
            return Err(format!(
                "Adding dependency would create cycle: {} -> {}",
                task_id, blocked_by_id
            ));
        }

        if let Some(task) = self.find_task_mut(task_id) {
            if !task.blocked_by.iter().any(|dep| dep == blocked_by_id) {
                task.blocked_by.push(blocked_by_id.to_string());
            }
        }

        if let Some(blocker) = self.find_task_mut(blocked_by_id) {
            if !blocker.blocks.iter().any(|dep| dep == task_id) {
                blocker.blocks.push(task_id.to_string());
            }
        }

        self.update_blocked_status();
        Ok(())
    }

    fn would_create_cycle(&self, task_id: &str, blocked_by_id: &str) -> bool {
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![blocked_by_id.to_string()];

        while let Some(current) = stack.pop() {
            if current == task_id {
                return true;
            }
            if visited.insert(current.clone()) {
                if let Some(task) = self.find_task(&current) {
                    stack.extend(task.blocked_by.iter().cloned());
                }
            }
        }

        false
    }

    fn update_status(&mut self) {
        if self.status != PlanStatus::Abandoned {
            if self.is_complete() {
                self.status = PlanStatus::Completed;
            } else {
                self.status = PlanStatus::InProgress;
            }
        }
    }

    /// Count total tasks.
    pub fn total_tasks(&self) -> usize {
        self.phases.iter().map(|p| p.tasks.len()).sum()
    }

    /// Count completed tasks.
    pub fn completed_tasks(&self) -> usize {
        self.phases.iter().map(|p| p.completed_count()).sum()
    }

    /// Check if all tasks are complete.
    pub fn is_complete(&self) -> bool {
        !self.phases.is_empty() && self.phases.iter().all(|p| p.is_complete())
    }

    /// Check if any tasks are currently in progress.
    pub fn has_in_progress_tasks(&self) -> bool {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .any(|t| t.status == TaskStatus::InProgress)
    }

    /// Get progress as fraction (completed / total).
    pub fn progress(&self) -> (usize, usize) {
        (self.completed_tasks(), self.total_tasks())
    }

    /// Merge another plan into this one.
    pub fn merge_from(&mut self, other: &PlanFile) {
        if other.title != "Untitled Plan"
            && (self.title == "Untitled Plan" || self.title.is_empty())
        {
            self.title = other.title.clone();
        }

        for other_phase in &other.phases {
            if let Some(existing) = self
                .phases
                .iter_mut()
                .find(|p| p.number == other_phase.number)
            {
                for other_task in &other_phase.tasks {
                    if let Some(existing_task) =
                        existing.tasks.iter_mut().find(|t| t.id == other_task.id)
                    {
                        if other_task.completed || other_task.status == TaskStatus::Completed {
                            existing_task.completed = true;
                            existing_task.status = TaskStatus::Completed;
                            if existing_task.completed_at.is_none() {
                                existing_task.completed_at = other_task.completed_at;
                            }
                        }
                        if !other_task.description.is_empty() {
                            existing_task.description = other_task.description.clone();
                        }
                        if other_task.context.is_some() {
                            existing_task.context = other_task.context.clone();
                        }
                        if other_task.result.is_some() {
                            existing_task.result = other_task.result.clone();
                        }
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
                        existing.tasks.push(other_task.clone());
                    }
                }
            } else {
                self.phases.push(other_phase.clone());
            }
        }

        self.phases.sort_by_key(|p| p.number);
    }
}
