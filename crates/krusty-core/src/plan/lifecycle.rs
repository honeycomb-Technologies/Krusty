//! Shared plan lifecycle helpers.
//!
//! Keeps "active plan" and "effective work mode" resolution consistent across
//! CLI, server, and core context injection.

use crate::storage::WorkMode;

use super::{PlanFile, PlanStatus};

/// Canonicalized plan lifecycle state for a session.
#[derive(Debug, Clone)]
pub struct PlanLifecycleState {
    pub plan: Option<PlanFile>,
    pub active_plan: Option<PlanFile>,
    pub effective_work_mode: WorkMode,
}

impl PlanLifecycleState {
    pub fn from_session_mode(work_mode: WorkMode, plan: Option<PlanFile>) -> Self {
        let effective_work_mode = resolve_effective_work_mode(work_mode, plan.as_ref());
        let active_plan = plan.clone().filter(is_active_plan);

        Self {
            plan,
            active_plan,
            effective_work_mode,
        }
    }
}

/// Returns true when a stored plan should still be treated as active runtime state.
pub fn is_active_plan(plan: &PlanFile) -> bool {
    matches!(plan.status, PlanStatus::InProgress) && !plan.is_complete()
}

/// Resolve the effective work mode for a session from persisted mode + plan state.
///
/// This preserves the stored mode for new planning sessions, but also repairs
/// older sessions where work had already started while the persisted mode
/// remained stuck in `plan`.
pub fn resolve_effective_work_mode(work_mode: WorkMode, plan: Option<&PlanFile>) -> WorkMode {
    match plan {
        Some(plan) if is_active_plan(plan) => {
            if work_mode == WorkMode::Plan
                && (plan.completed_tasks() > 0 || plan.has_in_progress_tasks())
            {
                WorkMode::Build
            } else {
                work_mode
            }
        }
        Some(_) => WorkMode::Build,
        None => work_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_active_plan, resolve_effective_work_mode, PlanLifecycleState};
    use crate::plan::{PlanFile, PlanStatus};
    use crate::storage::WorkMode;

    fn sample_plan() -> PlanFile {
        let mut plan = PlanFile::new("Lifecycle");
        let phase = plan.add_phase("Phase 1");
        phase.add_task("Do the thing");
        plan
    }

    #[test]
    fn in_progress_plan_with_started_work_repairs_stale_plan_mode() {
        let mut plan = sample_plan();
        plan.start_task("1.1").unwrap();

        assert_eq!(
            resolve_effective_work_mode(WorkMode::Plan, Some(&plan)),
            WorkMode::Build
        );
    }

    #[test]
    fn archived_plan_is_not_active() {
        let mut plan = sample_plan();
        plan.status = PlanStatus::Completed;

        assert!(!is_active_plan(&plan));
        assert_eq!(
            resolve_effective_work_mode(WorkMode::Plan, Some(&plan)),
            WorkMode::Build
        );
    }

    #[test]
    fn lifecycle_state_filters_archived_plans_from_active_runtime_state() {
        let mut plan = sample_plan();
        plan.status = PlanStatus::Abandoned;

        let lifecycle = PlanLifecycleState::from_session_mode(WorkMode::Plan, Some(plan));

        assert!(lifecycle.plan.is_some());
        assert!(lifecycle.active_plan.is_none());
        assert_eq!(lifecycle.effective_work_mode, WorkMode::Build);
    }
}
