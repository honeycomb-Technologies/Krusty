use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Transaction, TransactionBehavior};
use tempfile::TempDir;

use crate::agent::{
    WorkerGoalAttemptOutcome, WorkerGoalEffectSummary, WorkerGoalEvidence, WorkerGoalEvidenceKind,
    WorkerGoalOutcomeCommitInput, WorkerGoalOutcomeCommitter, WorkerGoalOutcomeCounters,
};
use crate::hive::HiveRunStatus;
use crate::storage::{
    ClaimRunRequest, DaemonFence, Database, HiveRunExecutionModeV1, HiveRunKind, HiveRunStore,
    RunCompletion, SqliteWorkerGoalAcceptanceStore, SqliteWorkerGoalOutcomeStore,
    WorkerGoalAcceptanceCommitDisposition, WorkerGoalAcceptanceStoreError, WorkerRunOrigin,
    WorkerWorkflowProviderRecovery,
};
use crate::workflow::{
    activate_or_resume_worker_workflow_in_transaction,
    archive_worker_goal_acceptances_in_transaction, cancel_worker_workflow_in_transaction,
    finalize_worker_workflow_attempt_in_transaction,
    materialize_due_worker_workflow_rollovers_in_transaction, pause_worker_workflow_in_transaction,
    UserGoalCriterionAcceptance, UserGoalCriterionDecision, UserWorkerGoalAcceptanceDecision,
    UserWorkerGoalAcceptanceRequest, WorkerWorkflowActivation, WorkerWorkflowActivationRequest,
    WorkerWorkflowActivationSource, WorkerWorkflowLifecycleRequest, WorkflowError, WorkflowManager,
};

use super::store::apply_trusted_workflow_outcome;
use super::{
    reconcile_worker_workflow_provider_boundary_in_transaction,
    worker_goal_outcome_is_accounted_in_transaction,
};

const NOW: &str = "2026-08-25T00:00:00.000000Z";
const WORKSPACE: &str = "/tmp/mitsuro-worker-workflow-test";

struct Fixture {
    db: Database,
    path: PathBuf,
    _temp: TempDir,
}

fn instant(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 25, 0, 0, second)
        .single()
        .unwrap()
}

fn durable_lease_duration() -> Duration {
    (Utc.with_ymd_and_hms(2099, 8, 25, 0, 10, 0)
        .single()
        .unwrap()
        - instant(0))
    .to_std()
    .unwrap()
}

fn fixture() -> Fixture {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("worker-workflow.db");
    let db = Database::new(&path).unwrap();
    db.conn()
        .execute_batch(&format!(
            r#"
            INSERT INTO users (id, email)
            VALUES ('alice', 'alice@example.test');
            INSERT INTO sessions (
                id, title, created_at, updated_at, session_type, permission_mode,
                user_id, working_dir, project_dir, workspace_mode
            ) VALUES (
                'worker-dm', 'Worker DM', '{NOW}', '{NOW}', 'hive', 'autonomous',
                'alice', '{WORKSPACE}', '{WORKSPACE}', 'selected'
            );
            INSERT INTO hive_workers (
                id, user_id, slug, display_name, model, model_key_json,
                model_catalog_revision, permission_mode, autonomy, status,
                dm_session_id, memory_namespace_id, created_at, updated_at
            ) VALUES (
                'worker-1', 'alice', 'worker-1', 'Worker 1', 'test-model',
                '{{"provider":"grok","model_id":"test-model","api_format":"open_ai_responses"}}', 'catalog-1',
                'autonomous', 'manual', 'active', 'worker-dm', 'worker-1',
                '{NOW}', '{NOW}'
            );
            INSERT INTO hive_controllers (
                id, scope_key, user_id, session_id, status, timezone,
                max_concurrent_runs, worker_id, created_at, updated_at
            ) VALUES (
                'controller-1', 'worker:worker-1', 'alice', 'worker-dm',
                'active', 'UTC', 1, 'worker-1', '{NOW}', '{NOW}'
            );
            INSERT INTO hive_worker_introductions (
                worker_id, run_id, status, prompt_version,
                created_at, updated_at, completed_at
            ) VALUES (
                'worker-1', NULL, 'confirmed', 1, '{NOW}', '{NOW}', '{NOW}'
            );
            INSERT INTO workflow_goals (
                id, session_id, title, objective, constraints_json, status,
                needs_definition, revision, source, created_at, updated_at
            ) VALUES (
                'goal-1', 'worker-dm', 'Build it', 'Implement the requested change',
                '[]', 'draft', 0, 1, 'user', '{NOW}', '{NOW}'
            );
            INSERT INTO workflow_goal_criteria (
                id, goal_id, position, description, required, status
            ) VALUES ('criterion-1', 'goal-1', 0, 'Tests pass', 1, 'pending');
            INSERT INTO workflow_plan_revisions (
                id, goal_id, revision_number, status, title, created_at,
                approved_at
            ) VALUES (
                'plan-1', 'goal-1', 1, 'active', 'Implementation', '{NOW}', '{NOW}'
            );
            INSERT INTO workflow_plan_steps (
                id, plan_revision_id, display_key, position, description,
                acceptance_criteria_json, required, status, evidence_json,
                revision, created_at
            ) VALUES (
                'step-1', 'plan-1', '1', 0, 'Implement', '["Tests pass"]',
                1, 'pending', '[]', 1, '{NOW}'
            );
            "#
        ))
        .unwrap();
    Fixture {
        db,
        path,
        _temp: temp,
    }
}

fn activation_request(operation_id: &str) -> WorkerWorkflowActivationRequest {
    WorkerWorkflowActivationRequest {
        worker_id: "worker-1".into(),
        expected_worker_revision: 1,
        owner_user_id: Some("alice".into()),
        goal_id: "goal-1".into(),
        expected_goal_revision: 1,
        operation_id: operation_id.into(),
        source: WorkerWorkflowActivationSource::UserActivation,
        now: instant(0),
    }
}

fn activate(fixture: &Fixture, operation_id: &str) -> WorkerWorkflowActivation {
    activate_with_source(
        fixture,
        operation_id,
        WorkerWorkflowActivationSource::UserActivation,
    )
}

fn activate_with_source(
    fixture: &Fixture,
    operation_id: &str,
    source: WorkerWorkflowActivationSource,
) -> WorkerWorkflowActivation {
    let mut request = activation_request(operation_id);
    request.source = source;
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let activation = activate_or_resume_worker_workflow_in_transaction(&tx, &request).unwrap();
    tx.commit().unwrap();
    activation
}

fn lifecycle_request(
    operation_id: &str,
    goal_revision: u64,
    reason: &str,
) -> WorkerWorkflowLifecycleRequest {
    WorkerWorkflowLifecycleRequest {
        worker_id: "worker-1".into(),
        expected_worker_revision: 1,
        owner_user_id: Some("alice".into()),
        goal_id: "goal-1".into(),
        expected_goal_revision: goal_revision,
        operation_id: operation_id.into(),
        reason: reason.into(),
        now: instant(1),
    }
}

#[test]
fn activation_receipt_replays_exact_projection_and_rejects_mismatch() {
    let fixture = fixture();
    let created = activate(&fixture, "activate-1");
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let replayed =
        activate_or_resume_worker_workflow_in_transaction(&tx, &activation_request("activate-1"))
            .unwrap();
    assert_eq!(replayed, created);

    let mut mismatched = activation_request("activate-1");
    mismatched.source = WorkerWorkflowActivationSource::WorkflowRollover;
    assert!(matches!(
        activate_or_resume_worker_workflow_in_transaction(&tx, &mismatched),
        Err(WorkflowError::Conflict(_))
    ));
    tx.commit().unwrap();

    let (config_json, context_json, actor): (String, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT run.config_json, run.execution_context_json, event.actor
             FROM hive_runs run
             JOIN workflow_events event ON event.attempt_id = run.workflow_attempt_id
             WHERE run.id = ?1",
            [created.run_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_json).unwrap();
    let context: serde_json::Value = serde_json::from_str(&context_json).unwrap();
    assert_eq!(config["working_dir"], WORKSPACE);
    assert_eq!(config["project_dir"], WORKSPACE);
    assert_eq!(context["mode"]["kind"], "worker_goal");
    assert_eq!(context["mode"]["working_dir"], WORKSPACE);
    assert_eq!(context["mode"]["project_dir"], WORKSPACE);
    assert_eq!(actor, "user");
}

#[test]
fn lifecycle_receipt_replays_before_revision_fence_and_rejects_mismatch() {
    let fixture = fixture();
    let activation = activate(&fixture, "activate-lifecycle");
    let request = lifecycle_request("pause-1", activation.goal_revision, "user paused");
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let first = pause_worker_workflow_in_transaction(&tx, &request).unwrap();
    tx.commit().unwrap();
    assert!(first.changed);

    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    assert_eq!(
        pause_worker_workflow_in_transaction(&tx, &request).unwrap(),
        first
    );
    let mut mismatch = request;
    mismatch.reason = "different reason".into();
    assert!(matches!(
        pause_worker_workflow_in_transaction(&tx, &mismatch),
        Err(WorkflowError::Conflict(_))
    ));
    tx.commit().unwrap();
}

#[test]
fn inverse_guard_rejects_worker_goal_authority_on_non_workflow_run() {
    let fixture = fixture();
    let error = fixture
        .db
        .conn()
        .execute(
            "INSERT INTO hive_runs (
                 id, controller_id, kind, objective, config_json, status,
                 available_at, max_attempts, created_at, updated_at,
                 workflow_goal_id
             ) VALUES (
                 'smuggled-run', 'controller-1', 'dispatch', 'bad', '{}',
                 'queued', ?1, 1, ?1, ?1, 'goal-1'
             )",
            [NOW],
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("non-Workflow run cannot carry Worker Goal authority"));
}

#[test]
fn schema_rejects_unverified_succeeded_worker_goal_outcome() {
    let fixture = fixture();
    let activation = activate(&fixture, "activate-false-success");
    let error = fixture
        .db
        .conn()
        .execute(
            "INSERT INTO hive_worker_goal_outcomes (
                 run_id, worker_id, owner_user_id, session_id, workflow_goal_id,
                 workflow_attempt_id, plan_revision_id, step_id, workspace_dir,
                 provider_call_ids_json, outcome, evidence_json, effect_json,
                 counters_json, no_progress_streak, committed_at
             ) VALUES (
                 ?1, 'worker-1', 'alice', 'worker-dm', 'goal-1', ?2,
                 'plan-1', 'step-1', ?3, json_array('unverified-call'),
                 'succeeded', '[]',
                 '{\"summary\":\"unverified\",\"workspace_mutated\":false}',
                 '{\"provider_calls\":1}', 0, ?4
             )",
            params![
                activation.run_id,
                activation.workflow_attempt_id,
                WORKSPACE,
                NOW,
            ],
        )
        .unwrap_err();
    assert!(error.to_string().contains("CHECK constraint failed"));
}

fn insert_provider_started(
    fixture: &Fixture,
    activation: &WorkerWorkflowActivation,
    call_id: &str,
    call_kind: &str,
    lease_token: &str,
    lease_epoch: u64,
    reserved_tokens: u64,
) {
    insert_provider_started_with_owner_and_origin(
        fixture,
        activation,
        call_id,
        call_kind,
        lease_token,
        lease_epoch,
        reserved_tokens,
        Some("alice"),
        WorkerRunOrigin::UserWorkflowActivation,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_provider_started_with_owner(
    fixture: &Fixture,
    activation: &WorkerWorkflowActivation,
    call_id: &str,
    call_kind: &str,
    lease_token: &str,
    lease_epoch: u64,
    reserved_tokens: u64,
    owner_user_id: Option<&str>,
) {
    insert_provider_started_with_owner_and_origin(
        fixture,
        activation,
        call_id,
        call_kind,
        lease_token,
        lease_epoch,
        reserved_tokens,
        owner_user_id,
        WorkerRunOrigin::UserWorkflowActivation,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_provider_started_with_owner_and_origin(
    fixture: &Fixture,
    activation: &WorkerWorkflowActivation,
    call_id: &str,
    call_kind: &str,
    lease_token: &str,
    lease_epoch: u64,
    reserved_tokens: u64,
    owner_user_id: Option<&str>,
    origin: WorkerRunOrigin,
) {
    fixture
        .db
        .conn()
        .execute(
            r#"INSERT INTO hive_worker_provider_calls (
                 provider_call_id, worker_id, worker_revision, owner_user_id,
                 session_id, run_id, run_lease_token, run_lease_epoch,
                 run_lease_expires_at, workflow_goal_id, workflow_attempt_id,
                 origin, lane_key, call_kind, provider_id, model_id,
                 model_key_json, model_key_fingerprint, model_catalog_revision,
                 permission_mode, policy_revision, timezone, local_day,
                 reserved_tokens, started_at
             ) VALUES (
                 ?1, 'worker-1', 1, ?2, 'worker-dm', ?3, ?4, ?5,
                 '2099-08-25T00:10:00.000000Z', 'goal-1', ?6,
                 ?7, 'dm', ?8, 'grok', 'test-model',
                 '{"provider":"grok","model_id":"test-model","api_format":"open_ai_responses"}', ?9,
                 'catalog-1', 'autonomous', 1, 'UTC', '2026-08-25', ?10, ?11
             )"#,
            params![
                call_id,
                owner_user_id,
                activation.run_id,
                lease_token,
                lease_epoch,
                activation.workflow_attempt_id,
                origin.as_str(),
                call_kind,
                "a".repeat(64),
                reserved_tokens,
                NOW,
            ],
        )
        .unwrap();
}

#[test]
fn authoritative_reserved_usage_blocks_a_new_activation_before_attempt_insert() {
    let fixture = fixture();
    let (activation, lease_token) = running_activation(&fixture, "activate-budget");
    fixture
        .db
        .conn()
        .execute(
            "UPDATE workflow_goals SET token_budget = 10 WHERE id = 'goal-1'",
            [],
        )
        .unwrap();
    insert_provider_started(
        &fixture,
        &activation,
        "budget-call",
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    fixture
        .db
        .conn()
        .execute(
            "UPDATE hive_runs SET status = 'cancelled' WHERE id = ?1",
            [activation.run_id.as_str()],
        )
        .unwrap();
    let mut request = activation_request("activate-after-budget");
    request.expected_goal_revision = activation.goal_revision;
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let error = activate_or_resume_worker_workflow_in_transaction(&tx, &request).unwrap_err();
    assert!(matches!(error, WorkflowError::InvalidTransition(_)));
    let attempts: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM workflow_execution_attempts WHERE goal_id = 'goal-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 1);
    tx.rollback().unwrap();
}

#[test]
fn stale_daemon_cannot_reconcile_or_finalize() {
    let fixture = fixture();
    let activation = activate(&fixture, "activate-stale-daemon");
    let stale = DaemonFence {
        lease_name: "hive".into(),
        owner_id: "stale".into(),
        fencing_token: 99,
    };
    let manager = WorkflowManager::new(fixture.path.clone()).unwrap();
    assert!(matches!(
        manager.reconcile_worker_workflow_run(&stale, &activation.run_id, instant(2)),
        Err(WorkflowError::Conflict(_))
    ));
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    assert!(matches!(
        finalize_worker_workflow_attempt_in_transaction(
            &tx,
            &stale,
            "worker-1",
            Some("alice"),
            &activation.run_id,
            "rollover-stale",
            instant(2),
        ),
        Err(WorkflowError::Conflict(_))
    ));
    tx.rollback().unwrap();
}

fn running_activation(fixture: &Fixture, operation: &str) -> (WorkerWorkflowActivation, String) {
    let activation = activate(fixture, operation);
    let lease_token = claim_activation(fixture, &activation);
    (activation, lease_token)
}

fn claim_activation(fixture: &Fixture, activation: &WorkerWorkflowActivation) -> String {
    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    let claimed = store
        .claim_next(&ClaimRunRequest {
            executor_id: "executor-1".into(),
            lease_epoch: 7,
            now: instant(0),
            lease_duration: durable_lease_duration(),
            global_concurrency_limit: 4,
        })
        .unwrap()
        .unwrap();
    assert_eq!(claimed.run.id, activation.run_id);
    assert!(store
        .mark_running(&activation.run_id, &claimed.lease_token, 7, instant(1))
        .unwrap());
    claimed.lease_token
}

fn current_daemon_fence(fixture: &Fixture) -> DaemonFence {
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO hive_daemon_leases (
                 lease_name, owner_id, fencing_token, acquired_at,
                 heartbeat_at, expires_at
             ) VALUES (
                 'worker-workflow-test', 'executor-1', 7, ?1, ?1,
                 '2099-08-25T00:10:00.000000Z'
             )",
            [NOW],
        )
        .unwrap();
    DaemonFence {
        lease_name: "worker-workflow-test".into(),
        owner_id: "executor-1".into(),
        fencing_token: 7,
    }
}

fn progressed_input(
    activation: &WorkerWorkflowActivation,
    lease_token: &str,
    owner_user_id: Option<String>,
    call_id: &str,
) -> WorkerGoalOutcomeCommitInput {
    progressed_input_with_origin(
        activation,
        lease_token,
        owner_user_id,
        call_id,
        WorkerRunOrigin::UserWorkflowActivation,
    )
}

fn progressed_input_with_origin(
    activation: &WorkerWorkflowActivation,
    lease_token: &str,
    owner_user_id: Option<String>,
    call_id: &str,
    origin: WorkerRunOrigin,
) -> WorkerGoalOutcomeCommitInput {
    WorkerGoalOutcomeCommitInput::from_validated_run(
        "worker-1".into(),
        1,
        owner_user_id,
        "worker-dm".into(),
        activation.run_id.clone(),
        lease_token.into(),
        7,
        origin,
        "goal-1".into(),
        activation.goal_revision,
        activation.workflow_aggregate_revision,
        activation.workflow_attempt_id.clone(),
        activation.plan_revision_id.clone(),
        activation.plan_revision_number,
        activation.step_id.clone(),
        activation.step_revision,
        PathBuf::from(WORKSPACE),
        vec![call_id.into()],
        WorkerGoalAttemptOutcome::Progressed,
        vec![WorkerGoalEvidence::new(
            WorkerGoalEvidenceKind::WorkspaceMutation,
            "applied a governed workspace change",
        )
        .unwrap()],
        WorkerGoalEffectSummary::new("governed workspace change observed", true).unwrap(),
        WorkerGoalOutcomeCounters {
            provider_calls: 1,
            turns: 1,
            tool_calls: 1,
            successful_tool_calls: 1,
            failed_tool_calls: 0,
            research_actions: 0,
        },
    )
    .unwrap()
}

fn acknowledge_provider_call(fixture: &Fixture, call_id: &str) {
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO hive_worker_provider_call_outcomes (
                 provider_call_id, state, outcome, remote_acceptance,
                 finished_at
             ) VALUES (?1, 'completed', 'completed', 'acknowledged', ?2)",
            params![call_id, NOW],
        )
        .unwrap();
}

fn stage_alice_progressed_acceptance(
    fixture: &Fixture,
    operation_id: &str,
    call_id: &str,
) -> (WorkerWorkflowActivation, String, DaemonFence, String) {
    let (activation, lease_token) = running_activation(fixture, operation_id);
    insert_provider_started(
        fixture,
        &activation,
        call_id,
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    let fence = current_daemon_fence(fixture);
    SqliteWorkerGoalOutcomeStore::new(&fixture.path, fence.clone())
        .commit_outcome(&progressed_input(
            &activation,
            &lease_token,
            Some("alice".into()),
            call_id,
        ))
        .unwrap();
    let acceptance_run_id = fixture
        .db
        .conn()
        .query_row(
            "SELECT acceptance_run_id
             FROM hive_worker_goal_acceptance_candidates
             WHERE source_run_id = ?1",
            [activation.run_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    (activation, lease_token, fence, acceptance_run_id)
}

#[test]
fn local_owner_progress_stages_one_isolated_acceptance_and_resolves_exactly_once() {
    let fixture = fixture();
    fixture
        .db
        .conn()
        .execute_batch(
            "UPDATE sessions SET user_id = NULL WHERE id = 'worker-dm';
             UPDATE hive_workers SET user_id = NULL WHERE id = 'worker-1';
             UPDATE hive_controllers SET user_id = NULL WHERE id = 'controller-1';",
        )
        .unwrap();
    let mut request = activation_request("activate-local-acceptance");
    request.owner_user_id = None;
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let activation = activate_or_resume_worker_workflow_in_transaction(&tx, &request).unwrap();
    tx.commit().unwrap();
    let lease_token = claim_activation(&fixture, &activation);
    insert_provider_started_with_owner(
        &fixture,
        &activation,
        "local-progress-call",
        "agent_turn",
        &lease_token,
        7,
        10,
        None,
    );
    let fence = current_daemon_fence(&fixture);
    let before_messages: i64 = fixture
        .db
        .conn()
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    let before_episodes: i64 = fixture
        .db
        .conn()
        .query_row("SELECT COUNT(*) FROM conversation_episodes", [], |row| {
            row.get(0)
        })
        .unwrap();
    let outcome_store = SqliteWorkerGoalOutcomeStore::new(&fixture.path, fence.clone());
    outcome_store
        .commit_outcome(&progressed_input(
            &activation,
            &lease_token,
            None,
            "local-progress-call",
        ))
        .unwrap();

    let acceptance_run_id: String = fixture
        .db
        .conn()
        .query_row(
            "SELECT acceptance_run_id
             FROM hive_worker_goal_acceptance_candidates
             WHERE source_run_id = ?1",
            [activation.run_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let acceptance_store = SqliteWorkerGoalAcceptanceStore::new(&fixture.path);
    let candidate = acceptance_store
        .candidate(&acceptance_run_id, None)
        .unwrap()
        .expect("local null owner can see its candidate");
    assert_eq!(candidate.owner_user_id, None);
    assert_eq!(candidate.source_run_id, activation.run_id);
    assert_eq!(
        candidate.source_summary.outcome,
        WorkerGoalAttemptOutcome::Progressed
    );
    assert_eq!(candidate.source_summary.evidence.len(), 1);
    assert_eq!(
        candidate.source_summary.evidence[0].summary(),
        "applied a governed workspace change"
    );
    assert!(candidate.source_summary.effect.workspace_mutated());
    assert_eq!(candidate.source_summary.counters.successful_tool_calls, 1);
    assert!(acceptance_store
        .candidate(&acceptance_run_id, Some("another-owner"))
        .unwrap()
        .is_none());
    let acceptance_run = HiveRunStore::new(Database::new(&fixture.path).unwrap())
        .get_run(&acceptance_run_id)
        .unwrap()
        .unwrap();
    assert_eq!(acceptance_run.kind, HiveRunKind::WorkerWorkflowAcceptance);
    assert_eq!(acceptance_run.status, HiveRunStatus::AwaitingInput);
    assert!(matches!(
        acceptance_run.execution_context.unwrap().mode,
        HiveRunExecutionModeV1::WorkerGoalAcceptance { tool_allowlist, .. }
            if tool_allowlist.is_empty()
    ));
    let staged: (String, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT attempt.status, attempt.stop_reason, step.status
             FROM workflow_execution_attempts attempt
             JOIN workflow_plan_steps step ON step.id = attempt.step_id
             WHERE attempt.id = ?1",
            [activation.workflow_attempt_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        staged,
        (
            "paused".into(),
            "awaiting_acceptance".into(),
            "in_progress".into()
        )
    );

    acknowledge_provider_call(&fixture, "local-progress-call");
    let run_store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    assert_eq!(
        run_store
            .finish_claimed_fenced(
                &activation.run_id,
                &lease_token,
                7,
                &completion(HiveRunStatus::Succeeded, 3),
                &fence,
            )
            .unwrap(),
        Some(HiveRunStatus::Succeeded)
    );
    let decision = UserWorkerGoalAcceptanceRequest {
        acceptance_run_id: acceptance_run_id.clone(),
        expected_goal_revision: candidate.goal_revision,
        decision: UserWorkerGoalAcceptanceDecision::Accept,
        reason: "The exact staged result satisfies the approved criterion".into(),
        criteria: vec![UserGoalCriterionAcceptance {
            criterion_id: "criterion-1".into(),
            decision: UserGoalCriterionDecision::Passed,
            evidence: vec!["Owner verified the exact staged workspace result".into()],
        }],
    };
    assert!(matches!(
        acceptance_store.resolve_user(Some("another-owner"), &decision),
        Err(WorkerGoalAcceptanceStoreError::NotFound(_))
    ));
    let resolved = acceptance_store.resolve_user(None, &decision).unwrap();
    assert_eq!(
        resolved.disposition,
        WorkerGoalAcceptanceCommitDisposition::Inserted
    );
    assert_eq!(resolved.goal_status, "completed");
    assert_eq!(resolved.step_status, "completed");
    // Simulate the two-phase daemon crash window: the core result committed,
    // no outer idempotency receipt was finalized, and later maintenance moved
    // the live Workflow projection before the exact command was retried.
    fixture
        .db
        .conn()
        .execute_batch(
            "UPDATE workflow_goals
             SET revision = revision + 10, status = 'active',
                 status_reason = 'simulated_post_commit_rollover'
             WHERE id = 'goal-1';
             UPDATE workflow_plan_steps SET status = 'pending'
             WHERE id = 'step-1';",
        )
        .unwrap();
    let replay = acceptance_store.resolve_user(None, &decision).unwrap();
    assert_eq!(
        replay.disposition,
        WorkerGoalAcceptanceCommitDisposition::AdoptedExact
    );
    assert_eq!(replay.acceptance_run_id, resolved.acceptance_run_id);
    assert_eq!(replay.source_run_id, resolved.source_run_id);
    assert_eq!(replay.workflow_goal_id, resolved.workflow_goal_id);
    assert_eq!(replay.source_attempt_id, resolved.source_attempt_id);
    assert_eq!(replay.step_id, resolved.step_id);
    assert_eq!(replay.decision, resolved.decision);
    assert_eq!(replay.goal_revision, resolved.goal_revision);
    assert_eq!(replay.goal_status, resolved.goal_status);
    assert_eq!(replay.step_status, resolved.step_status);
    let mut conflicting = decision;
    conflicting.reason = "A different replay payload".into();
    assert!(matches!(
        acceptance_store.resolve_user(None, &conflicting),
        Err(WorkerGoalAcceptanceStoreError::Conflict(_))
    ));

    let after_messages: i64 = fixture
        .db
        .conn()
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    let after_episodes: i64 = fixture
        .db
        .conn()
        .query_row("SELECT COUNT(*) FROM conversation_episodes", [], |row| {
            row.get(0)
        })
        .unwrap();
    let acceptance_provider_calls: i64 = fixture
        .db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_worker_provider_calls WHERE run_id = ?1",
            [acceptance_run_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after_messages, before_messages);
    assert_eq!(after_episodes, before_episodes);
    assert_eq!(acceptance_provider_calls, 0);
}

#[test]
fn pending_acceptance_rejects_goal_pause_and_goal_cancel_terminalizes_atomically() {
    let fixture = fixture();
    let (activation, _lease_token, _fence, acceptance_run_id) = stage_alice_progressed_acceptance(
        &fixture,
        "activate-acceptance-lifecycle",
        "acceptance-lifecycle-call",
    );
    assert!(fixture
        .db
        .conn()
        .execute(
            "UPDATE hive_runs SET status = 'cancelled' WHERE id = ?1",
            [acceptance_run_id.as_str()],
        )
        .is_err());
    let candidate_goal_revision: u64 = fixture
        .db
        .conn()
        .query_row(
            "SELECT goal_revision
             FROM hive_worker_goal_acceptance_candidates
             WHERE acceptance_run_id = ?1",
            [acceptance_run_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let pause_request = lifecycle_request(
        "pause-pending-acceptance",
        candidate_goal_revision,
        "owner requested pause",
    );
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    assert!(matches!(
        pause_worker_workflow_in_transaction(&tx, &pause_request),
        Err(WorkflowError::InvalidTransition(_))
    ));
    tx.rollback().unwrap();

    let cancel_request = lifecycle_request(
        "cancel-pending-acceptance",
        candidate_goal_revision,
        "owner cancelled the pending acceptance",
    );
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let first = cancel_worker_workflow_in_transaction(&tx, &cancel_request).unwrap();
    tx.commit().unwrap();
    assert!(first.changed);
    assert!(first.affected_run_ids.contains(&acceptance_run_id));
    assert!(first.affected_run_ids.contains(&activation.run_id));
    let terminal: (String, String, String, String, String, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT candidate.state, result.authority, acceptance.status,
                    source.status, attempt.status, step.status, goal.status
             FROM hive_worker_goal_acceptance_candidates candidate
             JOIN hive_worker_goal_acceptance_results result
               ON result.acceptance_run_id = candidate.acceptance_run_id
             JOIN hive_runs acceptance ON acceptance.id = candidate.acceptance_run_id
             JOIN hive_runs source ON source.id = candidate.source_run_id
             JOIN workflow_execution_attempts attempt
               ON attempt.id = candidate.source_attempt_id
             JOIN workflow_plan_steps step ON step.id = candidate.step_id
             JOIN workflow_goals goal ON goal.id = candidate.workflow_goal_id
             WHERE candidate.acceptance_run_id = ?1",
            [acceptance_run_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        terminal,
        (
            "stale".into(),
            "lifecycle".into(),
            "cancelled".into(),
            "cancelled".into(),
            "cancelled".into(),
            "cancelled".into(),
            "cancelled".into(),
        )
    );
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    assert_eq!(
        cancel_worker_workflow_in_transaction(&tx, &cancel_request).unwrap(),
        first
    );
    tx.commit().unwrap();
}

#[test]
fn worker_archive_terminalizes_pending_acceptance_goal_and_plan() {
    let fixture = fixture();
    let (_activation, _lease_token, _fence, acceptance_run_id) = stage_alice_progressed_acceptance(
        &fixture,
        "activate-acceptance-worker-archive",
        "acceptance-worker-archive-call",
    );
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    assert_eq!(
        archive_worker_goal_acceptances_in_transaction(&tx, "worker-1", NOW).unwrap(),
        vec![acceptance_run_id.clone()]
    );
    tx.commit().unwrap();

    let terminal: (String, String, String, String, String, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT candidate.state, result.reason, acceptance.status,
                    attempt.status, step.status, plan.status, goal.status
             FROM hive_worker_goal_acceptance_candidates candidate
             JOIN hive_worker_goal_acceptance_results result
               ON result.acceptance_run_id = candidate.acceptance_run_id
             JOIN hive_runs acceptance ON acceptance.id = candidate.acceptance_run_id
             JOIN workflow_execution_attempts attempt
               ON attempt.id = candidate.source_attempt_id
             JOIN workflow_plan_steps step ON step.id = candidate.step_id
             JOIN workflow_plan_revisions plan ON plan.id = candidate.plan_revision_id
             JOIN workflow_goals goal ON goal.id = candidate.workflow_goal_id
             WHERE candidate.acceptance_run_id = ?1",
            [acceptance_run_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        terminal,
        (
            "stale".into(),
            "worker_archived".into(),
            "cancelled".into(),
            "cancelled".into(),
            "cancelled".into(),
            "cancelled".into(),
            "cancelled".into(),
        )
    );
}

fn completion(target_status: HiveRunStatus, second: u32) -> RunCompletion {
    RunCompletion {
        target_status,
        now: instant(second),
        available_at: None,
        wake_at: None,
        stop_reason: Some("executor returned".into()),
        error: (target_status == HiveRunStatus::Failed).then(|| "backend failure".into()),
        outcome: Some(serde_json::json!({"backend": "terminal"})),
        trace_sequence_end: Some(7),
    }
}

fn expire_run(fixture: &Fixture, run_id: &str) {
    fixture
        .db
        .conn()
        .execute(
            "UPDATE hive_runs SET lease_expires_at = ?2 WHERE id = ?1",
            params![run_id, NOW],
        )
        .unwrap();
}

fn insert_committed_outcome(
    fixture: &Fixture,
    activation: &WorkerWorkflowActivation,
    call_id: &str,
) {
    insert_committed_outcome_with_evidence(fixture, activation, call_id, &[]);
}

fn insert_committed_outcome_with_evidence(
    fixture: &Fixture,
    activation: &WorkerWorkflowActivation,
    call_id: &str,
    evidence: &[WorkerGoalEvidence],
) {
    let evidence_json = serde_json::to_string(evidence).unwrap();
    fixture
        .db
        .conn()
        .execute(
            r#"INSERT INTO hive_worker_goal_outcomes (
                 run_id, worker_id, owner_user_id, session_id, workflow_goal_id,
                 workflow_attempt_id, plan_revision_id, step_id, workspace_dir,
                 provider_call_ids_json, outcome, evidence_json, effect_json,
                 counters_json, no_progress_streak, committed_at
             ) VALUES (
                 ?1, 'worker-1', 'alice', 'worker-dm', 'goal-1', ?2,
                 'plan-1', 'step-1', ?3, json_array(?4), 'blocked',
                 ?5, '{"summary":"observed","workspace_mutated":false}',
                 '{"provider_calls":1,"turns":1,"tool_calls":1,"successful_tool_calls":1,"failed_tool_calls":0,"research_actions":1}',
                 1, ?6
             )"#,
            params![
                activation.run_id,
                activation.workflow_attempt_id,
                WORKSPACE,
                call_id,
                evidence_json,
                NOW,
            ],
        )
        .unwrap();
}

#[test]
fn expired_pre_provider_run_requeues_safely() {
    let fixture = fixture();
    let (activation, _) = running_activation(&fixture, "activate-safe-expiry");
    expire_run(&fixture, &activation.run_id);
    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    let result = store.reconcile_expired_leases(instant(20)).unwrap();
    assert_eq!(result.requeued_unstarted, 1);
    assert_eq!(
        store.get_run(&activation.run_id).unwrap().unwrap().status,
        HiveRunStatus::Queued
    );
}

#[test]
fn expired_started_without_outcome_becomes_unknown_and_recovery_required() {
    let fixture = fixture();
    let (activation, lease_token) = running_activation(&fixture, "activate-uncertain");
    insert_provider_started(
        &fixture,
        &activation,
        "uncertain-call",
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    expire_run(&fixture, &activation.run_id);
    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    let result = store.reconcile_expired_leases(instant(20)).unwrap();
    assert_eq!(result.recovery_required, 1);
    assert_eq!(
        store.get_run(&activation.run_id).unwrap().unwrap().status,
        HiveRunStatus::RecoveryRequired
    );
    let state: String = fixture
        .db
        .conn()
        .query_row(
            "SELECT state FROM hive_worker_provider_call_outcomes
             WHERE provider_call_id = 'uncertain-call'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "unknown");
}

#[test]
fn committed_outcome_adopts_the_exact_held_final_call_after_crash() {
    let fixture = fixture();
    let (activation, lease_token) = running_activation(&fixture, "activate-adopt");
    insert_provider_started(
        &fixture,
        &activation,
        "final-call",
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    insert_committed_outcome(&fixture, &activation, "final-call");
    expire_run(&fixture, &activation.run_id);
    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    let result = store.reconcile_expired_leases(instant(20)).unwrap();
    assert_eq!(result.recovered_succeeded, 1);
    assert_eq!(
        store.get_run(&activation.run_id).unwrap().unwrap().status,
        HiveRunStatus::Succeeded
    );
    let terminal: (String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT state, remote_acceptance
             FROM hive_worker_provider_call_outcomes
             WHERE provider_call_id = 'final-call'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(terminal, ("completed".into(), "acknowledged".into()));
}

#[test]
fn normal_finish_forces_recovery_for_succeeded_without_provider_or_outcome() {
    let fixture = fixture();
    let (activation, lease_token) = running_activation(&fixture, "finish-empty-success");
    let fence = current_daemon_fence(&fixture);
    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    assert_eq!(
        store
            .finish_claimed_fenced(
                &activation.run_id,
                &lease_token,
                7,
                &completion(HiveRunStatus::Succeeded, 2),
                &fence,
            )
            .unwrap(),
        Some(HiveRunStatus::RecoveryRequired)
    );
}

#[test]
fn normal_finish_marks_started_without_outcome_unknown_and_requires_recovery() {
    let fixture = fixture();
    let (activation, lease_token) = running_activation(&fixture, "finish-uncertain");
    insert_provider_started(
        &fixture,
        &activation,
        "finish-uncertain-call",
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    let fence = current_daemon_fence(&fixture);
    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    assert_eq!(
        store
            .finish_claimed_fenced(
                &activation.run_id,
                &lease_token,
                7,
                &completion(HiveRunStatus::Failed, 2),
                &fence,
            )
            .unwrap(),
        Some(HiveRunStatus::RecoveryRequired)
    );
    let state: String = fixture
        .db
        .conn()
        .query_row(
            "SELECT state FROM hive_worker_provider_call_outcomes
             WHERE provider_call_id = 'finish-uncertain-call'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "unknown");
}

#[test]
fn normal_finish_adopts_committed_outcome_despite_backend_failure() {
    let fixture = fixture();
    let (activation, lease_token) = running_activation(&fixture, "finish-adopt");
    insert_provider_started(
        &fixture,
        &activation,
        "finish-final-call",
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    insert_committed_outcome(&fixture, &activation, "finish-final-call");
    let fence = current_daemon_fence(&fixture);
    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    assert_eq!(
        store
            .finish_claimed_fenced(
                &activation.run_id,
                &lease_token,
                7,
                &completion(HiveRunStatus::Failed, 2),
                &fence,
            )
            .unwrap(),
        Some(HiveRunStatus::Succeeded)
    );
    let terminal: (String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT state, remote_acceptance
             FROM hive_worker_provider_call_outcomes
             WHERE provider_call_id = 'finish-final-call'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(terminal, ("completed".into(), "acknowledged".into()));
}

#[test]
fn recovered_rollover_no_progress_advances_idle_backoff_exactly_once() {
    let fixture = fixture();
    let activation = activate_with_source(
        &fixture,
        "rollover-idle-recovery",
        WorkerWorkflowActivationSource::WorkflowRollover,
    );
    let lease_token = claim_activation(&fixture, &activation);
    insert_provider_started_with_owner_and_origin(
        &fixture,
        &activation,
        "rollover-idle-call",
        "agent_turn",
        &lease_token,
        7,
        10,
        Some("alice"),
        WorkerRunOrigin::WorkflowRollover,
    );
    insert_committed_outcome(&fixture, &activation, "rollover-idle-call");
    expire_run(&fixture, &activation.run_id);

    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    let recovered = store.reconcile_expired_leases(instant(20)).unwrap();
    assert_eq!(recovered.recovered_succeeded, 1);
    let first: (i64, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT idle_streak, not_before, last_outcome_run_id
             FROM hive_worker_idle_state
             WHERE worker_id = 'worker-1' AND lane_key = 'dm'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(first.0, 1);
    assert_eq!(first.2, activation.run_id);
    assert_eq!(
        crate::hive::parse_utc_timestamp(&first.1).unwrap(),
        instant(20) + chrono::Duration::minutes(15)
    );

    let replay = store.reconcile_expired_leases(instant(21)).unwrap();
    assert_eq!(replay.recovered_succeeded, 0);
    let after_replay: (i64, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT idle_streak, not_before, last_outcome_run_id
             FROM hive_worker_idle_state
             WHERE worker_id = 'worker-1' AND lane_key = 'dm'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(after_replay, first);
}

#[test]
fn recovered_rollover_with_opaque_runtime_effect_leaves_idle_projection_unchanged() {
    let fixture = fixture();
    let activation = activate_with_source(
        &fixture,
        "rollover-opaque-recovery",
        WorkerWorkflowActivationSource::WorkflowRollover,
    );
    let lease_token = claim_activation(&fixture, &activation);
    insert_provider_started_with_owner_and_origin(
        &fixture,
        &activation,
        "rollover-opaque-call",
        "agent_turn",
        &lease_token,
        7,
        10,
        Some("alice"),
        WorkerRunOrigin::WorkflowRollover,
    );
    let evidence = [WorkerGoalEvidence::new(
        WorkerGoalEvidenceKind::Runtime,
        "opaque governed command completed without trusted change evidence",
    )
    .unwrap()];
    insert_committed_outcome_with_evidence(
        &fixture,
        &activation,
        "rollover-opaque-call",
        &evidence,
    );
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO hive_worker_idle_state (
                 worker_id, lane_key, idle_streak, not_before,
                 last_material_at, last_outcome_run_id, updated_at
             ) VALUES (
                 'worker-1', 'dm', 2, '2026-08-25T01:00:00.000000Z',
                 '2026-08-25T00:00:00.000000Z', 'prior-idle-run', ?1
             )",
            [NOW],
        )
        .unwrap();
    let before: (i64, String, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT idle_streak, not_before, last_material_at, last_outcome_run_id
             FROM hive_worker_idle_state
             WHERE worker_id = 'worker-1' AND lane_key = 'dm'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    expire_run(&fixture, &activation.run_id);

    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    let recovered = store.reconcile_expired_leases(instant(20)).unwrap();
    assert_eq!(recovered.recovered_succeeded, 1);
    assert_eq!(
        store.get_run(&activation.run_id).unwrap().unwrap().status,
        HiveRunStatus::Succeeded
    );
    let after: (i64, String, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT idle_streak, not_before, last_material_at, last_outcome_run_id
             FROM hive_worker_idle_state
             WHERE worker_id = 'worker-1' AND lane_key = 'dm'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn material_rollover_finish_resets_idle_from_typed_goal_effect_and_replays_idempotently() {
    let fixture = fixture();
    let activation = activate_with_source(
        &fixture,
        "rollover-material-finish",
        WorkerWorkflowActivationSource::WorkflowRollover,
    );
    let lease_token = claim_activation(&fixture, &activation);
    insert_provider_started_with_owner_and_origin(
        &fixture,
        &activation,
        "rollover-material-call",
        "agent_turn",
        &lease_token,
        7,
        10,
        Some("alice"),
        WorkerRunOrigin::WorkflowRollover,
    );
    let fence = current_daemon_fence(&fixture);
    SqliteWorkerGoalOutcomeStore::new(&fixture.path, fence.clone())
        .commit_outcome(&progressed_input_with_origin(
            &activation,
            &lease_token,
            Some("alice".into()),
            "rollover-material-call",
            WorkerRunOrigin::WorkflowRollover,
        ))
        .unwrap();
    acknowledge_provider_call(&fixture, "rollover-material-call");
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO hive_worker_idle_state (
                 worker_id, lane_key, idle_streak, not_before,
                 last_outcome_run_id, updated_at
             ) VALUES (
                 'worker-1', 'dm', 2, '2026-08-25T01:00:00.000000Z',
                 'prior-idle-run', ?1
             )",
            [NOW],
        )
        .unwrap();

    let store = HiveRunStore::new(Database::new(&fixture.path).unwrap());
    assert_eq!(
        store
            .finish_claimed_fenced(
                &activation.run_id,
                &lease_token,
                7,
                &completion(HiveRunStatus::Succeeded, 3),
                &fence,
            )
            .unwrap(),
        Some(HiveRunStatus::Succeeded)
    );
    let material: (i64, Option<String>, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT idle_streak, not_before, last_material_at, last_outcome_run_id
             FROM hive_worker_idle_state
             WHERE worker_id = 'worker-1' AND lane_key = 'dm'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(material.0, 0);
    assert!(material.1.is_none());
    assert_eq!(material.2, crate::hive::canonical_timestamp(instant(3)));
    assert_eq!(material.3, activation.run_id);

    let replay_db = Database::new(&fixture.path).unwrap();
    let replay =
        Transaction::new_unchecked(replay_db.conn(), TransactionBehavior::Immediate).unwrap();
    crate::storage::update_derived_state_for_run_in_transaction(
        &replay,
        &activation.run_id,
        HiveRunStatus::Succeeded,
        &crate::hive::canonical_timestamp(instant(4)),
    )
    .unwrap();
    replay.commit().unwrap();
    let after_replay: (i64, Option<String>, String, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT idle_streak, not_before, last_material_at, last_outcome_run_id
             FROM hive_worker_idle_state
             WHERE worker_id = 'worker-1' AND lane_key = 'dm'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(after_replay, material);
}

#[test]
fn missing_typed_rollover_outcome_rejects_derived_success_and_rolls_back() {
    let fixture = fixture();
    let activation = activate_with_source(
        &fixture,
        "rollover-missing-outcome",
        WorkerWorkflowActivationSource::WorkflowRollover,
    );
    // Simulate a legacy/corrupt database that predates the schema-level
    // success guard. The production derived-state boundary must still fail
    // closed instead of resetting or advancing idle state from prose.
    fixture
        .db
        .conn()
        .execute_batch("DROP TRIGGER hive_runs_worker_workflow_success_guard")
        .unwrap();
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    tx.execute(
        "UPDATE hive_runs
         SET status = 'succeeded', finished_at = ?2, updated_at = ?2
         WHERE id = ?1",
        params![
            activation.run_id,
            crate::hive::canonical_timestamp(instant(2))
        ],
    )
    .unwrap();
    let error = crate::storage::update_derived_state_for_run_in_transaction(
        &tx,
        &activation.run_id,
        HiveRunStatus::Succeeded,
        &crate::hive::canonical_timestamp(instant(2)),
    )
    .unwrap_err();
    assert!(error.to_string().contains("has no exact typed outcome"));
    tx.rollback().unwrap();
    assert_eq!(
        HiveRunStore::new(Database::new(&fixture.path).unwrap())
            .get_run(&activation.run_id)
            .unwrap()
            .unwrap()
            .status,
        HiveRunStatus::Queued
    );
    let idle_rows: i64 = fixture
        .db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_worker_idle_state
             WHERE worker_id = 'worker-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(idle_rows, 0);
}

#[test]
fn unresolved_auxiliary_call_cannot_be_silently_accounted() {
    let fixture = fixture();
    let (activation, lease_token) = running_activation(&fixture, "activate-aux");
    insert_provider_started(
        &fixture,
        &activation,
        "agent-call",
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    insert_provider_started(
        &fixture,
        &activation,
        "classifier-call",
        "classifier",
        &lease_token,
        7,
        2,
    );
    insert_committed_outcome(&fixture, &activation, "agent-call");
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let disposition = reconcile_worker_workflow_provider_boundary_in_transaction(
        &tx,
        &activation.run_id,
        &lease_token,
        7,
        NOW,
    )
    .unwrap();
    assert_eq!(
        disposition,
        WorkerWorkflowProviderRecovery::CommittedOutcomeUnaccounted
    );
    assert!(!worker_goal_outcome_is_accounted_in_transaction(&tx, &activation.run_id).unwrap());
    tx.rollback().unwrap();
}

#[test]
fn cancel_facade_is_transactional_and_receipted() {
    let fixture = fixture();
    let activation = activate(&fixture, "activate-cancel");
    let request = lifecycle_request("cancel-1", activation.goal_revision, "user cancelled");
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let first = cancel_worker_workflow_in_transaction(&tx, &request).unwrap();
    assert!(first.changed);
    tx.commit().unwrap();
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    assert_eq!(
        cancel_worker_workflow_in_transaction(&tx, &request).unwrap(),
        first
    );
    tx.commit().unwrap();
}

fn no_progress_input(activation: &WorkerWorkflowActivation) -> WorkerGoalOutcomeCommitInput {
    WorkerGoalOutcomeCommitInput::from_validated_run(
        "worker-1".into(),
        1,
        Some("alice".into()),
        "worker-dm".into(),
        activation.run_id.clone(),
        "test-lease".into(),
        7,
        WorkerRunOrigin::UserWorkflowActivation,
        "goal-1".into(),
        activation.goal_revision,
        activation.workflow_aggregate_revision,
        activation.workflow_attempt_id.clone(),
        activation.plan_revision_id.clone(),
        activation.plan_revision_number,
        activation.step_id.clone(),
        activation.step_revision,
        PathBuf::from(WORKSPACE),
        vec![format!("call:{}", activation.run_id)],
        WorkerGoalAttemptOutcome::Blocked,
        vec![WorkerGoalEvidence::new(
            WorkerGoalEvidenceKind::WorkspaceObservation,
            "inspected the same canonical target",
        )
        .unwrap()],
        WorkerGoalEffectSummary::new("same read-only observation", false).unwrap(),
        WorkerGoalOutcomeCounters {
            provider_calls: 1,
            turns: 1,
            tool_calls: 1,
            successful_tool_calls: 1,
            failed_tool_calls: 0,
            research_actions: 1,
        },
    )
    .unwrap()
}

#[test]
fn third_identical_durable_no_progress_attempt_blocks_goal() {
    let fixture = fixture();
    let mut activation = activate(&fixture, "no-progress-1");
    for streak in 1..=3 {
        let tx =
            Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
        apply_trusted_workflow_outcome(&tx, &no_progress_input(&activation), streak, false, NOW)
            .unwrap();
        tx.execute(
            "UPDATE hive_runs SET status = 'cancelled' WHERE id = ?1",
            [activation.run_id.as_str()],
        )
        .unwrap();
        tx.commit().unwrap();

        if streak < 3 {
            let goal_revision: u64 = fixture
                .db
                .conn()
                .query_row(
                    "SELECT revision FROM workflow_goals WHERE id = 'goal-1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let mut request = activation_request(&format!("no-progress-{}", streak + 1));
            request.expected_goal_revision = goal_revision;
            let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate)
                .unwrap();
            activation = activate_or_resume_worker_workflow_in_transaction(&tx, &request).unwrap();
            tx.commit().unwrap();
        }
    }
    let (goal_status, reason, step_status): (String, Option<String>, String) = fixture
        .db
        .conn()
        .query_row(
            "SELECT goal.status, goal.status_reason, step.status
             FROM workflow_goals goal
             JOIN workflow_plan_revisions plan ON plan.goal_id = goal.id
             JOIN workflow_plan_steps step ON step.plan_revision_id = plan.id
             WHERE goal.id = 'goal-1' AND plan.status = 'active'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(goal_status, "blocked");
    assert_eq!(reason.as_deref(), Some("repeated_no_progress"));
    assert_eq!(step_status, "blocked");
}

fn completed_source_for_due_rollover(
    fixture: &Fixture,
    operation_id: &str,
    call_id: &str,
) -> (WorkerWorkflowActivation, DaemonFence) {
    let (activation, lease_token) = running_activation(fixture, operation_id);
    insert_provider_started(
        fixture,
        &activation,
        call_id,
        "agent_turn",
        &lease_token,
        7,
        10,
    );
    insert_committed_outcome(fixture, &activation, call_id);
    fixture
        .db
        .conn()
        .execute(
            "UPDATE workflow_execution_attempts
             SET status = 'paused', stop_reason = 'bounded_attempt_complete',
                 ended_at = ?2, updated_at = ?2
             WHERE id = ?1",
            params![activation.workflow_attempt_id, NOW],
        )
        .unwrap();
    fixture
        .db
        .conn()
        .execute(
            "UPDATE workflow_plan_steps
             SET status = 'pending', claimed_attempt_id = NULL,
                 revision = revision + 1
             WHERE id = 'step-1'",
            [],
        )
        .unwrap();
    expire_run(fixture, &activation.run_id);
    HiveRunStore::new(Database::new(&fixture.path).unwrap())
        .reconcile_expired_leases(instant(20))
        .unwrap();
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO hive_daemon_leases (
                 lease_name, owner_id, fencing_token, acquired_at,
                 heartbeat_at, expires_at
             ) VALUES ('hive', 'executor-2', 8, ?1, ?1,
                       '2099-08-25T00:10:00.000000Z')",
            [NOW],
        )
        .unwrap();
    let fence = DaemonFence {
        lease_name: "hive".into(),
        owner_id: "executor-2".into(),
        fencing_token: 8,
    };
    (activation, fence)
}

#[test]
fn due_rollover_sweep_materializes_once_with_runtime_actor() {
    let fixture = fixture();
    let (activation, fence) =
        completed_source_for_due_rollover(&fixture, "activate-due-rollover", "rollover-final-call");
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let created =
        materialize_due_worker_workflow_rollovers_in_transaction(&tx, &fence, 4, instant(21))
            .unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(
        created[0].governor_origin,
        WorkerRunOrigin::WorkflowRollover
    );
    let replay =
        materialize_due_worker_workflow_rollovers_in_transaction(&tx, &fence, 4, instant(22))
            .unwrap();
    assert!(replay.is_empty());
    let actor: String = tx
        .query_row(
            "SELECT actor FROM workflow_events WHERE operation_id = ?1",
            [format!("worker-workflow-rollover:{}", activation.run_id)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actor, "hive_runtime");
    tx.commit().unwrap();
}

#[test]
fn due_rollover_skips_paused_worker_without_poisoning_the_sweep() {
    let fixture = fixture();
    let (activation, fence) = completed_source_for_due_rollover(
        &fixture,
        "activate-paused-rollover",
        "paused-rollover-final-call",
    );
    fixture
        .db
        .conn()
        .execute_batch(
            "UPDATE hive_workers SET status = 'paused' WHERE id = 'worker-1';
             UPDATE hive_controllers SET status = 'paused'
             WHERE id = 'controller-1';",
        )
        .unwrap();
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let skipped =
        materialize_due_worker_workflow_rollovers_in_transaction(&tx, &fence, 1, instant(21))
            .expect("a lifecycle-ineligible source must not abort the rollover batch");
    assert!(skipped.is_empty());
    tx.commit().unwrap();

    fixture
        .db
        .conn()
        .execute_batch(
            "UPDATE hive_workers SET status = 'active' WHERE id = 'worker-1';
             UPDATE hive_controllers SET status = 'active'
             WHERE id = 'controller-1';",
        )
        .unwrap();
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let resumed =
        materialize_due_worker_workflow_rollovers_in_transaction(&tx, &fence, 1, instant(22))
            .unwrap();
    assert_eq!(resumed.len(), 1);
    assert_ne!(resumed[0].run_id, activation.run_id);
    tx.commit().unwrap();
}

#[test]
fn due_rollover_reconciles_token_cap_before_materializing() {
    let fixture = fixture();
    let (_activation, fence) = completed_source_for_due_rollover(
        &fixture,
        "activate-capped-rollover",
        "capped-rollover-final-call",
    );
    fixture
        .db
        .conn()
        .execute(
            "UPDATE workflow_goals SET token_budget = 10 WHERE id = 'goal-1'",
            [],
        )
        .unwrap();
    let tx = Transaction::new_unchecked(fixture.db.conn(), TransactionBehavior::Immediate).unwrap();
    let created =
        materialize_due_worker_workflow_rollovers_in_transaction(&tx, &fence, 4, instant(21))
            .unwrap();
    assert!(created.is_empty());
    let projection: (String, Option<String>, u64) = tx
        .query_row(
            "SELECT status, status_reason, tokens_used
             FROM workflow_goals WHERE id = 'goal-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(projection.0, "paused");
    assert_eq!(projection.1.as_deref(), Some("token_budget_exhausted"));
    assert_eq!(projection.2, 10);
    tx.commit().unwrap();
}
