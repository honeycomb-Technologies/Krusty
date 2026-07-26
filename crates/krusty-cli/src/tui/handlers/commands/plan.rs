use crate::tui::app::App;

impl App {
    /// Handle /plan command.
    pub(super) fn handle_plan_command(&mut self, subcommand: Option<&str>) {
        use crate::plan::PlanStatus;

        if !matches!(subcommand, Some("list") | Some("history")) {
            if let Some(session_id) = self.runtime.current_session_id.clone() {
                let db_path = crate::paths::config_dir().join("krusty.db");
                match krusty_core::workflow::WorkflowManager::new(db_path).and_then(|manager| {
                    manager
                        .get_snapshot(&session_id)
                        .map(|snapshot| (manager, snapshot))
                }) {
                    Ok((manager, Some(snapshot))) => {
                        match subcommand {
                            None | Some("show") => {
                                let completed = snapshot
                                    .steps
                                    .iter()
                                    .filter(|step| {
                                        matches!(
                                            step.status,
                                            krusty_core::workflow::WorkflowStepStatus::Completed
                                                | krusty_core::workflow::WorkflowStepStatus::Skipped
                                        )
                                    })
                                    .count();
                                self.runtime.chat.messages.push((
                                    "system".to_string(),
                                    format!(
                                        "Plan '{}' [{}] — {}/{} steps\nGoal: {} [{}]\nUse /plan approve for a proposed revision; /goal controls execution.",
                                        snapshot
                                            .plan_revision
                                            .as_ref()
                                            .map(|plan| plan.title.as_str())
                                            .unwrap_or("not proposed"),
                                        snapshot
                                            .plan_revision
                                            .as_ref()
                                            .map(|plan| plan.status.as_str())
                                            .unwrap_or("none"),
                                        completed,
                                        snapshot.steps.len(),
                                        snapshot.goal.title,
                                        snapshot.goal.status
                                    ),
                                ));
                            }
                            Some("approve") => {
                                let Some(plan) = snapshot.plan_revision.as_ref() else {
                                    self.runtime.chat.messages.push((
                                        "system".to_string(),
                                        "No proposed plan revision to approve.".to_string(),
                                    ));
                                    return;
                                };
                                let operation_id =
                                    format!("tui:approve-plan:{}", uuid::Uuid::new_v4());
                                let message = match manager.approve_plan(
                                    &session_id,
                                    &snapshot.goal.id,
                                    &plan.id,
                                    snapshot.aggregate_revision,
                                    &operation_id,
                                    "user",
                                ) {
                                    Ok(mutation) => format!(
                                        "Approved plan '{}' (revision {}). Use /goal activate to begin.",
                                        plan.title, mutation.snapshot.aggregate_revision
                                    ),
                                    Err(error) => format!("Plan approval failed: {error}"),
                                };
                                self.runtime
                                    .chat
                                    .messages
                                    .push(("system".to_string(), message));
                            }
                            Some("clear") | Some("abandon") => {
                                let operation_id =
                                    format!("tui:cancel-goal:{}", uuid::Uuid::new_v4());
                                let message = match manager.cancel_goal(
                                    &session_id,
                                    &snapshot.goal.id,
                                    snapshot.aggregate_revision,
                                    Some("cancelled_from_tui_plan_command"),
                                    &operation_id,
                                    "user",
                                ) {
                                    Ok(_) => {
                                        self.clear_plan();
                                        format!(
                                            "Cancelled Goal '{}' and its active plan.",
                                            snapshot.goal.title
                                        )
                                    }
                                    Err(error) => format!("Could not cancel plan: {error}"),
                                };
                                self.runtime
                                    .chat
                                    .messages
                                    .push(("system".to_string(), message));
                            }
                            Some(unknown) => {
                                self.runtime.chat.messages.push((
                                    "system".to_string(),
                                    format!(
                                        "Unknown: /plan {unknown}. Use: /plan, /plan approve, /plan list, /plan clear"
                                    ),
                                ));
                            }
                        }
                        return;
                    }
                    Ok((_, None)) => {}
                    Err(error) => tracing::warn!("Failed to load durable workflow: {error}"),
                }
            }
        }

        match subcommand {
            Some("clear") | Some("abandon") => {
                if let Some(ref mut plan) = self.runtime.active_plan {
                    plan.status = PlanStatus::Abandoned;
                    if let Some(ref pm) = self.services.plan_manager {
                        if let Err(e) = pm.save_plan(plan) {
                            tracing::warn!("Failed to save abandoned plan: {}", e);
                        }
                    }
                    let title = plan.title.clone();
                    let file_path = plan
                        .file_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    self.clear_plan();
                    let msg = if file_path.is_empty() {
                        format!("Plan '{}' abandoned.", title)
                    } else {
                        format!("Plan '{}' abandoned. Saved at: {}", title, file_path)
                    };
                    self.runtime.chat.messages.push(("system".to_string(), msg));
                } else {
                    self.runtime
                        .chat
                        .messages
                        .push(("system".to_string(), "No active plan to clear.".to_string()));
                }
            }
            Some("list") | Some("history") => {
                let working_dir_str = self.runtime.working_dir.to_string_lossy().into_owned();
                if let Some(ref pm) = self.services.plan_manager {
                    match pm.list_completed_for_dir(&working_dir_str) {
                        Ok(plans) if plans.is_empty() => {
                            self.runtime.chat.messages.push((
                                "system".to_string(),
                                "No completed plans for this directory.".to_string(),
                            ));
                        }
                        Ok(plans) => {
                            let mut msg = String::from("Completed plans:\n");
                            for plan in plans.iter().take(5) {
                                let date = plan.created_at.format("%Y-%m-%d");
                                msg.push_str(&format!(
                                    "  • {} ({}) - {}/{} tasks\n",
                                    plan.title, date, plan.progress.0, plan.progress.1,
                                ));
                            }
                            if plans.len() > 5 {
                                msg.push_str(&format!("  ... and {} more", plans.len() - 5));
                            }
                            self.runtime.chat.messages.push(("system".to_string(), msg));
                        }
                        Err(e) => {
                            self.runtime.chat.messages.push((
                                "system".to_string(),
                                format!("Failed to list plans: {}", e),
                            ));
                        }
                    }
                }
            }
            Some("show") | None => {
                if let Some(ref plan) = self.runtime.active_plan {
                    let (completed, total) = plan.progress();
                    let status_icon = if completed == total { "✓" } else { "◐" };
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!(
                            "{} '{}' ({}/{} tasks)\nUse Ctrl+T to toggle sidebar, /plan clear to abandon.",
                            status_icon, plan.title, completed, total
                        ),
                    ));
                    if !self.ui.plan_sidebar.visible {
                        self.ui.plan_sidebar.toggle();
                    }
                } else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "No active plan.\n\
                        • Enter PLAN mode (Ctrl+G) and ask the AI to create a plan\n\
                        • Use /plan list to see completed plans"
                            .to_string(),
                    ));
                }
            }
            Some(unknown) => {
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!(
                        "Unknown: /plan {}. Use: /plan, /plan approve, /plan list, /plan clear",
                        unknown
                    ),
                ));
            }
        }
    }
}
