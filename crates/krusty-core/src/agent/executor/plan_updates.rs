use std::path::Path;

use tokio::sync::mpsc;

use crate::plan::{PlanFile, PlanManager};

use super::super::loop_events::{LoopEvent, PlanTaskInfo};

pub(super) fn emit_plan_update(
    session_id: &str,
    db_path: &Path,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) {
    let tasks = load_plan_update_tasks(session_id, db_path);
    let _ = event_tx.send(LoopEvent::PlanUpdate { tasks });
}

fn load_plan_update_tasks(session_id: &str, db_path: &Path) -> Vec<PlanTaskInfo> {
    match PlanManager::new(db_path.to_path_buf()).and_then(|pm| pm.get_plan(session_id)) {
        Ok(Some(plan)) => plan_task_infos(&plan),
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "Failed to load plan while emitting plan update"
            );
            Vec::new()
        }
    }
}

fn plan_task_infos(plan: &PlanFile) -> Vec<PlanTaskInfo> {
    plan.phases
        .iter()
        .flat_map(|phase| phase.tasks.iter())
        .map(|task| PlanTaskInfo {
            description: task.description.clone(),
            completed: task.completed,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use super::{emit_plan_update, load_plan_update_tasks};
    use crate::agent::loop_events::LoopEvent;
    use crate::plan::{PlanFile, PlanManager, TaskStatus};
    use crate::storage::SessionManager;
    use crate::Database;

    fn create_test_db() -> (std::path::PathBuf, TempDir) {
        let temp_dir = TempDir::new().expect("tempdir");
        let db_path = temp_dir.path().join("executor.db");
        Database::new(&db_path).expect("database");
        (db_path, temp_dir)
    }

    #[test]
    fn load_plan_update_tasks_returns_saved_plan_tasks() {
        let (db_path, _temp_dir) = create_test_db();
        let session_manager =
            SessionManager::new(Database::new(&db_path).expect("database should open"));
        let session_id = session_manager
            .create_session("Executor Test", None, Some("/tmp"))
            .expect("session should be created");

        let mut plan = PlanFile::new("Executor Plan");
        {
            let phase = plan.add_phase("Phase 1");
            phase.add_task("Keep continuity");
            phase.add_task("Harden plan updates");
        }
        plan.phases[0].tasks[1].completed = true;
        plan.phases[0].tasks[1].status = TaskStatus::Completed;

        let plan_manager = PlanManager::new(db_path.clone()).expect("plan manager");
        plan_manager
            .save_plan_for_session(&session_id, &plan)
            .expect("plan should save");

        let tasks = load_plan_update_tasks(&session_id, &db_path);

        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].description, "Keep continuity");
        assert!(!tasks[0].completed);
        assert_eq!(tasks[1].description, "Harden plan updates");
        assert!(tasks[1].completed);
    }

    #[test]
    fn load_plan_update_tasks_returns_empty_when_plan_store_unavailable() {
        let temp_dir = TempDir::new().expect("tempdir");
        let missing_db_path = temp_dir.path().join("missing").join("executor.db");

        let tasks = load_plan_update_tasks("session-1", &missing_db_path);

        assert!(tasks.is_empty());
    }

    #[test]
    fn emit_plan_update_sends_plan_tasks() {
        let (db_path, _temp_dir) = create_test_db();
        let session_manager =
            SessionManager::new(Database::new(&db_path).expect("database should open"));
        let session_id = session_manager
            .create_session("Executor Test", None, Some("/tmp"))
            .expect("session should be created");

        let mut plan = PlanFile::new("Executor Plan");
        plan.add_phase("Phase 1").add_task("Emit update");
        PlanManager::new(db_path.clone())
            .expect("plan manager")
            .save_plan_for_session(&session_id, &plan)
            .expect("plan should save");

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        emit_plan_update(&session_id, &db_path, &event_tx);

        let event = event_rx.try_recv().expect("plan update event");
        assert!(matches!(
            event,
            LoopEvent::PlanUpdate { tasks }
            if tasks.len() == 1 && tasks[0].description == "Emit update" && !tasks[0].completed
        ));
    }
}
