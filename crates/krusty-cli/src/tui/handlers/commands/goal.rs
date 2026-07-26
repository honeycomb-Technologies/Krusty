use crate::tui::app::{App, WorkMode};

impl App {
    pub(super) fn handle_goal_command(&mut self, subcommand: Option<&str>) {
        let Some(session_id) = self.runtime.current_session_id.clone() else {
            self.goal_message("No active session.");
            return;
        };
        let db_path = crate::paths::config_dir().join("krusty.db");
        let manager = match krusty_core::workflow::WorkflowManager::new(db_path) {
            Ok(manager) => manager,
            Err(error) => {
                self.goal_message(format!("Could not open Goal state: {error}"));
                return;
            }
        };
        let snapshot = match manager.get_snapshot(&session_id) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.goal_message(
                    "No durable Goal. Enter Plan mode and ask Krusty to draft one.".to_string(),
                );
                return;
            }
            Err(error) => {
                self.goal_message(format!("Could not load Goal: {error}"));
                return;
            }
        };
        let operation_id = format!(
            "tui:{}:{}",
            subcommand.unwrap_or("show"),
            uuid::Uuid::new_v4()
        );
        let result = match subcommand {
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
                self.goal_message(format!(
                    "Goal: {} [{}]\n{}\nPlan progress: {}/{}\nAllowed: {}",
                    snapshot.goal.title,
                    snapshot.goal.status,
                    snapshot.goal.objective,
                    completed,
                    snapshot.steps.len(),
                    snapshot.allowed_actions.join(", ")
                ));
                return;
            }
            Some("activate") => manager.activate_goal(
                &session_id,
                &snapshot.goal.id,
                snapshot.aggregate_revision,
                &operation_id,
                "user",
            ),
            Some("pause") => manager.pause_goal(
                &session_id,
                &snapshot.goal.id,
                snapshot.aggregate_revision,
                Some("paused_from_tui"),
                &operation_id,
                "user",
            ),
            Some("resume") => manager.resume_goal(
                &session_id,
                &snapshot.goal.id,
                snapshot.aggregate_revision,
                &operation_id,
                "user",
            ),
            Some("cancel") => manager.cancel_goal(
                &session_id,
                &snapshot.goal.id,
                snapshot.aggregate_revision,
                Some("cancelled_from_tui"),
                &operation_id,
                "user",
            ),
            Some(unknown) => {
                self.goal_message(format!(
                    "Unknown: /goal {unknown}. Use: /goal, /goal activate, /goal pause, /goal resume, /goal cancel"
                ));
                return;
            }
        };

        match result {
            Ok(mutation) => {
                if mutation.snapshot.goal.status == krusty_core::workflow::GoalStatus::Active {
                    self.set_work_mode(WorkMode::Build);
                }
                self.goal_message(format!(
                    "Goal '{}' is now {} (revision {}).",
                    mutation.snapshot.goal.title,
                    mutation.snapshot.goal.status,
                    mutation.snapshot.aggregate_revision
                ));
            }
            Err(error) => self.goal_message(format!("Goal command failed: {error}")),
        }
    }

    fn goal_message(&mut self, message: impl Into<String>) {
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), message.into()));
    }
}
