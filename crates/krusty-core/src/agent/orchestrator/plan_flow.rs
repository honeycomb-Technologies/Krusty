use std::path::Path;

use tokio::sync::mpsc;

use crate::plan::PlanManager;

use super::super::loop_events::{LoopEvent, PlanTaskInfo};
use super::super::plan_handler;

pub(super) fn handle_plan_detection(
    text: &str,
    session_id: &str,
    working_dir: &Path,
    db_path: &Path,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) -> bool {
    let Some(mut plan) = plan_handler::try_detect_plan(text) else {
        return false;
    };
    plan.plan_file.session_id = Some(session_id.to_string());
    plan.plan_file.working_dir = Some(working_dir.to_string_lossy().to_string());

    match PlanManager::new(db_path.to_path_buf()) {
        Ok(plan_manager) => {
            if let Err(e) = plan_manager.save_plan_for_session(session_id, &plan.plan_file) {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to save detected plan: {}", e
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                "Failed to initialize plan manager for detected plan: {}", e
            );
        }
    }

    let tasks: Vec<PlanTaskInfo> = plan
        .tasks
        .iter()
        .map(|t| PlanTaskInfo {
            description: t.description.clone(),
            completed: t.completed,
        })
        .collect();

    let _ = event_tx.send(LoopEvent::PlanUpdate {
        tasks: tasks.clone(),
    });

    let tool_call_id = format!("plan-confirm-{}", uuid::Uuid::new_v4());
    let _ = event_tx.send(LoopEvent::PlanComplete {
        tool_call_id: tool_call_id.clone(),
        title: plan.title,
        task_count: tasks.len(),
    });

    let _ = event_tx.send(LoopEvent::AwaitingInput {
        tool_call_id,
        tool_name: "PlanConfirm".to_string(),
    });

    true
}
