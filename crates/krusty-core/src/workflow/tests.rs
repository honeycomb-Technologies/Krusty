use tempfile::TempDir;

use crate::storage::{Database, SessionManager};

use super::{
    AttemptProgressInput, CompleteStepInput, CreateGoalInput, CriterionInput, CriterionStatus,
    GoalStatus, PlanProposalInput, SetCriterionInput, StartAttemptInput, StepProposalInput,
    WorkflowError, WorkflowManager, WorkflowStepStatus,
};

fn setup() -> (TempDir, String, WorkflowManager) {
    let temp = TempDir::new().expect("temp dir");
    let path = temp.path().join("workflow.db");
    let sessions = SessionManager::new(Database::new(&path).expect("database"));
    let session_id = sessions
        .create_session("Workflow", Some("test-model"), Some("/tmp"))
        .expect("session");
    let manager = WorkflowManager::new(path).expect("workflow manager");
    (temp, session_id, manager)
}

fn goal_input() -> CreateGoalInput {
    CreateGoalInput {
        title: "Finish the product".to_string(),
        objective: "Implement the workflow and prove it works".to_string(),
        constraints: vec!["Keep permissions unchanged".to_string()],
        criteria: vec![CriterionInput {
            description: "All required tests pass".to_string(),
            required: true,
        }],
        token_budget: None,
    }
}

fn plan_input() -> PlanProposalInput {
    PlanProposalInput {
        title: "Implementation".to_string(),
        rationale: Some("Deliver one verified slice".to_string()),
        source_message_id: None,
        predecessor_id: None,
        legacy_markdown: None,
        steps: vec![StepProposalInput {
            display_key: "1.1".to_string(),
            description: "Implement and verify the slice".to_string(),
            context: None,
            parent_display_key: None,
            dependencies: Vec::new(),
            acceptance_criteria: vec!["Targeted test passes".to_string()],
            required: true,
        }],
    }
}

fn activate_fixture(manager: &WorkflowManager, session_id: &str) -> (String, String, u64) {
    let created = manager
        .create_goal(session_id, goal_input(), "create", "user")
        .expect("create goal");
    let goal_id = created.snapshot.goal.id.clone();
    let proposed = manager
        .propose_plan(
            session_id,
            &goal_id,
            created.snapshot.aggregate_revision,
            plan_input(),
            "propose",
            "agent",
        )
        .expect("propose plan");
    let plan_id = proposed
        .snapshot
        .plan_revision
        .as_ref()
        .expect("plan")
        .id
        .clone();
    let approved = manager
        .approve_plan(
            session_id,
            &goal_id,
            &plan_id,
            proposed.snapshot.aggregate_revision,
            "approve",
            "user",
        )
        .expect("approve plan");
    let active = manager
        .activate_goal(
            session_id,
            &goal_id,
            approved.snapshot.aggregate_revision,
            "activate",
            "user",
        )
        .expect("activate goal");
    (goal_id, plan_id, active.snapshot.aggregate_revision)
}

#[test]
fn lifecycle_is_revisioned_evidence_backed_and_idempotent() {
    let (_temp, session_id, manager) = setup();
    let (goal_id, _plan_id, revision) = activate_fixture(&manager, &session_id);

    let attempt = manager
        .start_attempt(
            &session_id,
            &goal_id,
            revision,
            StartAttemptInput {
                step_id: None,
                permission_mode: "autonomous".to_string(),
                max_turns: 8,
                max_tool_calls: 32,
                max_wall_time_secs: 600,
                max_research_actions: 8,
            },
            "attempt",
            "agent",
        )
        .expect("start attempt");
    let attempt_id = attempt
        .snapshot
        .latest_attempt
        .as_ref()
        .expect("attempt")
        .id
        .clone();
    let step_id = attempt.snapshot.steps[0].id.clone();
    let claimed = manager
        .claim_step(
            &session_id,
            &goal_id,
            &attempt_id,
            &step_id,
            attempt.snapshot.aggregate_revision,
            "claim",
            "agent",
        )
        .expect("claim step");
    assert!(claimed.changed);
    assert_eq!(
        claimed.snapshot.steps[0].status,
        WorkflowStepStatus::InProgress
    );

    let duplicate = manager
        .claim_step(
            &session_id,
            &goal_id,
            &attempt_id,
            &step_id,
            claimed.snapshot.aggregate_revision,
            "claim-again",
            "agent",
        )
        .expect("repeat claim is no-op");
    assert!(!duplicate.changed);
    assert_eq!(
        duplicate.snapshot.aggregate_revision,
        claimed.snapshot.aggregate_revision
    );

    let completed_step = manager
        .complete_step(
            &session_id,
            &goal_id,
            &step_id,
            duplicate.snapshot.aggregate_revision,
            CompleteStepInput {
                attempt_id,
                outcome: "Implemented the slice".to_string(),
                evidence: vec!["targeted test passed".to_string()],
            },
            "complete-step",
            "agent",
        )
        .expect("complete step");
    assert_eq!(
        completed_step.snapshot.steps[0].status,
        WorkflowStepStatus::Completed
    );
    assert_eq!(
        completed_step.snapshot.goal.status,
        GoalStatus::Active,
        "finishing a plan step must not complete the goal"
    );

    let criterion_id = completed_step.snapshot.criteria[0].id.clone();
    let verified = manager
        .set_criterion(
            &session_id,
            &goal_id,
            &criterion_id,
            completed_step.snapshot.aggregate_revision,
            SetCriterionInput {
                status: CriterionStatus::Passed,
                evidence: vec!["cargo test passed".to_string()],
                verifier: "test-runner".to_string(),
            },
            "verify",
            "agent",
        )
        .expect("verify criterion");
    let completed = manager
        .complete_goal(
            &session_id,
            &goal_id,
            verified.snapshot.aggregate_revision,
            "complete-goal",
            "agent",
        )
        .expect("complete goal");
    assert_eq!(completed.snapshot.goal.status, GoalStatus::Completed);
    assert_eq!(completed.snapshot.permission_mode, "autonomous");
}

#[test]
fn finite_research_budget_pauses_instead_of_looping() {
    let (_temp, session_id, manager) = setup();
    let (goal_id, _plan_id, revision) = activate_fixture(&manager, &session_id);
    let attempt = manager
        .start_attempt(
            &session_id,
            &goal_id,
            revision,
            StartAttemptInput {
                step_id: None,
                permission_mode: "supervised".to_string(),
                max_turns: 20,
                max_tool_calls: 100,
                max_wall_time_secs: 900,
                max_research_actions: 2,
            },
            "attempt",
            "agent",
        )
        .expect("start attempt");
    let attempt_id = attempt
        .snapshot
        .latest_attempt
        .as_ref()
        .expect("attempt")
        .id
        .clone();
    let stopped = manager
        .record_attempt_progress(
            &session_id,
            &goal_id,
            &attempt_id,
            attempt.snapshot.aggregate_revision,
            AttemptProgressInput {
                turn_count: 2,
                tool_call_count: 2,
                research_action_count: 2,
                material_progress: false,
                blocker_fingerprint: None,
            },
            "progress",
            "agent",
        )
        .expect("record progress");
    assert_eq!(stopped.snapshot.goal.status, GoalStatus::Paused);
    assert_eq!(
        stopped.snapshot.goal.status_reason.as_deref(),
        Some("research_budget_exhausted")
    );
}

#[test]
fn stale_writers_and_second_unfinished_goal_fail_safely() {
    let (_temp, session_id, manager) = setup();
    let created = manager
        .create_goal(&session_id, goal_input(), "create", "user")
        .expect("create goal");
    let second = manager.create_goal(&session_id, goal_input(), "second", "user");
    assert!(matches!(second, Err(WorkflowError::Conflict(_))));

    let stale = manager.edit_goal(
        &session_id,
        &created.snapshot.goal.id,
        created.snapshot.aggregate_revision + 1,
        Default::default(),
        "stale",
        "user",
    );
    assert!(matches!(stale, Err(WorkflowError::Conflict(_))));
}
