use crate::tui::app::App;

impl App {
    /// Handle /plan command.
    pub(super) fn handle_plan_command(&mut self, subcommand: Option<&str>) {
        use crate::plan::PlanStatus;

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
                        "Unknown: /plan {}. Use: /plan, /plan list, /plan clear",
                        unknown
                    ),
                ));
            }
        }
    }
}
