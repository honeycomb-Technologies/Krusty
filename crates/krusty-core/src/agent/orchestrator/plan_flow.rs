use std::path::Path;

use tokio::sync::mpsc;

use crate::plan::PlanManager;
use crate::storage::{
    Database, PendingInteractionSnapshot, PendingPlanTaskSnapshot, SessionManager, WorkMode,
};
use crate::tools::registry::PermissionMode;

use super::super::loop_events::{LoopEvent, PlanTaskInfo};
use super::super::plan_handler;

const AUTONOMOUS_PLAN_COMPLETE_REASON: &str =
    "Plan completed; continuing autonomously in Build mode";

pub(super) enum PlanDetectionOutcome {
    AwaitingConfirmation(PendingInteractionSnapshot),
    ContinueInBuildMode,
    Failed(String),
}

pub(super) fn handle_plan_detection(
    text: &str,
    session_id: &str,
    working_dir: &Path,
    db_path: &Path,
    permission_mode: PermissionMode,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) -> Option<PlanDetectionOutcome> {
    let mut plan = plan_handler::try_detect_plan(text)?;
    plan.plan_file.session_id = Some(session_id.to_string());
    plan.plan_file.working_dir = Some(working_dir.to_string_lossy().to_string());

    let save_error = match PlanManager::new(db_path.to_path_buf()) {
        Ok(plan_manager) => {
            if let Err(e) = plan_manager.save_plan_for_session(session_id, &plan.plan_file) {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to save detected plan: {}", e
                );
                Some(format!("Failed to save detected plan: {e}"))
            } else {
                None
            }
        }
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                "Failed to initialize plan manager for detected plan: {}", e
            );
            Some(format!("Failed to initialize plan manager: {e}"))
        }
    };

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

    if permission_mode == PermissionMode::Autonomous {
        if let Some(error) = save_error {
            return Some(PlanDetectionOutcome::Failed(error));
        }

        let mode_update = Database::new(db_path)
            .map(SessionManager::new)
            .and_then(|manager| manager.update_session_work_mode(session_id, WorkMode::Build));
        if let Err(error) = mode_update {
            tracing::error!(
                session_id = %session_id,
                %error,
                "Failed to continue autonomous plan in Build mode"
            );
            return Some(PlanDetectionOutcome::Failed(format!(
                "Failed to continue autonomous plan in Build mode: {error}"
            )));
        }

        let _ = event_tx.send(LoopEvent::ModeChange {
            mode: WorkMode::Build.to_string(),
            reason: Some(AUTONOMOUS_PLAN_COMPLETE_REASON.to_string()),
        });
        return Some(PlanDetectionOutcome::ContinueInBuildMode);
    }

    let tool_call_id = format!("plan-confirm-{}", uuid::Uuid::new_v4());
    let title = plan.title.clone();
    let _ = event_tx.send(LoopEvent::PlanComplete {
        tool_call_id: tool_call_id.clone(),
        title: title.clone(),
        task_count: tasks.len(),
    });

    let _ = event_tx.send(LoopEvent::AwaitingInput {
        tool_call_id: tool_call_id.clone(),
        tool_name: "PlanConfirm".to_string(),
    });

    let pending_tasks = tasks
        .into_iter()
        .map(|task| PendingPlanTaskSnapshot {
            description: task.description,
            completed: task.completed,
        })
        .collect();

    Some(PlanDetectionOutcome::AwaitingConfirmation(
        PendingInteractionSnapshot::plan_confirm(
            tool_call_id,
            title,
            plan.tasks.len(),
            pending_tasks,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const PLAN: &str = r#"# Plan: Tiny service

## Phase 1: Build

- [ ] Create the service
- [ ] Verify the service
"#;

    fn session_in_plan_mode(db_path: &Path) -> String {
        let manager = SessionManager::new(Database::new(db_path).expect("database"));
        let session_id = manager
            .create_session("plan flow", Some("test-model"), Some("/tmp"))
            .expect("session");
        manager
            .update_session_work_mode(&session_id, WorkMode::Plan)
            .expect("plan mode");
        session_id
    }

    fn collect_events(receiver: &mut mpsc::UnboundedReceiver<LoopEvent>) -> Vec<LoopEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn supervised_plan_still_waits_for_confirmation() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("krusty.db");
        let session_id = session_in_plan_mode(&db_path);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        let outcome = handle_plan_detection(
            PLAN,
            &session_id,
            temp.path(),
            &db_path,
            PermissionMode::Supervised,
            &event_tx,
        )
        .expect("plan detected");

        assert!(matches!(
            outcome,
            PlanDetectionOutcome::AwaitingConfirmation(_)
        ));
        let session = SessionManager::new(Database::new(&db_path).expect("database"))
            .get_session(&session_id)
            .expect("session lookup")
            .expect("session exists");
        assert_eq!(session.work_mode, WorkMode::Plan);

        let events = collect_events(&mut event_rx);
        assert!(events
            .iter()
            .any(|event| matches!(event, LoopEvent::PlanUpdate { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, LoopEvent::PlanComplete { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, LoopEvent::AwaitingInput { tool_name, .. } if tool_name == "PlanConfirm")));
        assert!(!events
            .iter()
            .any(|event| matches!(event, LoopEvent::ModeChange { .. })));
    }

    #[test]
    fn autonomous_plan_persists_build_mode_without_confirmation() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("krusty.db");
        let session_id = session_in_plan_mode(&db_path);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        let outcome = handle_plan_detection(
            PLAN,
            &session_id,
            temp.path(),
            &db_path,
            PermissionMode::Autonomous,
            &event_tx,
        )
        .expect("plan detected");

        assert!(matches!(outcome, PlanDetectionOutcome::ContinueInBuildMode));
        let session = SessionManager::new(Database::new(&db_path).expect("database"))
            .get_session(&session_id)
            .expect("session lookup")
            .expect("session exists");
        assert_eq!(session.work_mode, WorkMode::Build);

        let events = collect_events(&mut event_rx);
        assert!(events
            .iter()
            .any(|event| matches!(event, LoopEvent::PlanUpdate { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            LoopEvent::ModeChange { mode, reason }
                if mode == "build"
                    && reason.as_deref() == Some(AUTONOMOUS_PLAN_COMPLETE_REASON)
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, LoopEvent::PlanComplete { .. })));
        assert!(!events
            .iter()
            .any(|event| matches!(event, LoopEvent::AwaitingInput { .. })));
    }
}
