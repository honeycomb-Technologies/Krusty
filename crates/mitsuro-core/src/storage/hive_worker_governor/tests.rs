use std::sync::{Arc, Barrier};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use tempfile::TempDir;

use crate::ai::models::{ApiFormat, ModelKey};
use crate::ai::providers::ProviderId;
use crate::ai::types::Usage;
use crate::hive::{DstFoldPolicy, DstGapPolicy};
use crate::storage::Database;
use crate::tools::registry::PermissionMode;

use super::*;

const OWNER: &str = "alice";
const WORKER_ID: &str = "worker-1";
const DM_SESSION: &str = "worker-dm";
const GROUP_SESSION: &str = "worker-group-lane";
const GROUP_ID: &str = "group-1";
const CONTROLLER_ID: &str = "controller-1";

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("valid fixture time")
}

fn model_key() -> ModelKey {
    ModelKey::new(
        ProviderId::Grok,
        "grok-worker-test",
        ApiFormat::OpenAIResponses,
    )
}

fn fixture() -> (HiveWorkerGovernorStore, TempDir) {
    let temp = TempDir::new().expect("temp dir");
    let db = Database::new(&temp.path().join("governor.db")).expect("create database");
    seed_identity(db.conn());
    (HiveWorkerGovernorStore::new(db), temp)
}

fn seed_identity(conn: &Connection) {
    let model_key_json = serde_json::to_string(&model_key()).unwrap();
    conn.execute_batch(
        "INSERT INTO users (id, email)
         VALUES ('alice', 'alice@example.test');
         INSERT INTO sessions (
             id, user_id, title, created_at, updated_at, session_type, token_count
         ) VALUES
             ('worker-dm', 'alice', 'Worker DM',
              '2026-08-25T00:00:00.000000Z', '2026-08-25T00:00:00.000000Z',
              'hive', 777),
             ('worker-group-lane', 'alice', 'Worker group lane',
              '2026-08-25T00:00:00.000000Z', '2026-08-25T00:00:00.000000Z',
              'hive', 333);
         INSERT INTO hive_groups (
             id, user_id, title, execution_mode, max_rounds,
             max_member_messages_per_turn, parallelism, context_window_messages,
             status, created_at, updated_at
         ) VALUES (
             'group-1', 'alice', 'Test group', 'workbench', 3, 2, 2, 24,
             'active', '2026-08-25T00:00:00.000000Z',
             '2026-08-25T00:00:00.000000Z'
         );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO hive_workers (
             id, user_id, slug, display_name, model, model_key_json,
             model_catalog_revision, permission_mode, autonomy, status,
             dm_session_id, memory_namespace_id, created_at, updated_at
         ) VALUES (
             ?1, ?2, 'worker-1', 'Worker 1', 'grok-worker-test', ?3,
             'catalog-v1', 'autonomous', 'always_on', 'active', ?4,
             'worker-1', '2026-08-25T00:00:00.000000Z',
             '2026-08-25T00:00:00.000000Z'
         )",
        params![WORKER_ID, OWNER, model_key_json, DM_SESSION],
    )
    .unwrap();
    conn.execute_batch(
        "INSERT INTO hive_group_members (group_id, worker_id, position, added_at)
         VALUES ('group-1', 'worker-1', 0, '2026-08-25T00:00:00.000000Z');
         INSERT INTO hive_group_worker_lanes (
             group_id, worker_id, session_id, created_at, updated_at
         ) VALUES (
             'group-1', 'worker-1', 'worker-group-lane',
             '2026-08-25T00:00:00.000000Z',
             '2026-08-25T00:00:00.000000Z'
         );
         INSERT INTO hive_controllers (
             id, scope_key, user_id, session_id, status, timezone,
             max_concurrent_runs, worker_id, created_at, updated_at
         ) VALUES (
             'controller-1', 'worker:test', 'alice', 'worker-dm', 'active',
             'UTC', 8, 'worker-1', '2026-08-25T00:00:00.000000Z',
             '2026-08-25T00:00:00.000000Z'
         );",
    )
    .unwrap();
}

fn seed_running_run(
    conn: &Connection,
    run_id: &str,
    session_id: &str,
    origin: WorkerRunOrigin,
    lane_key: &str,
    started_at: &DateTime<Utc>,
) {
    let started = crate::hive::canonical_timestamp(*started_at);
    let expires = crate::hive::canonical_timestamp(*started_at + chrono::Duration::minutes(10));
    let lane = if lane_key == "dm" {
        WorkerConversationLane::DirectMessage
    } else {
        WorkerConversationLane::Group {
            group_id: lane_key
                .strip_prefix("group:")
                .expect("group lane key")
                .to_string(),
        }
    };
    let execution_context = serde_json::to_string(
        &crate::storage::HiveRunExecutionContextV1::worker_conversation_neutral(WORKER_ID, 1, lane)
            .unwrap(),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO hive_runs (
             id, controller_id, session_id, kind, objective, config_json, status,
             priority, available_at, attempt_count, max_attempts, lease_owner,
             lease_token, lease_epoch, lease_expires_at, created_at, started_at,
             updated_at, worker_id, governor_origin, governor_lane_key,
             execution_context_json
         ) VALUES (
             ?1, ?2, ?3, 'worker_heartbeat', 'test Worker run', '{}', 'running',
             0, ?4, 1, 3, 'executor-1', ?5, 7, ?6, ?4, ?4, ?4, ?7, ?8, ?9,
             ?10
         )",
        params![
            run_id,
            CONTROLLER_ID,
            session_id,
            started,
            format!("lease-{run_id}"),
            expires,
            WORKER_ID,
            origin.as_str(),
            lane_key,
            execution_context,
        ],
    )
    .unwrap();
}

fn seed_queued_run(
    conn: &Connection,
    run_id: &str,
    kind: &str,
    session_id: &str,
    origin: WorkerRunOrigin,
    lane: WorkerConversationLane,
    created_at: &DateTime<Utc>,
) {
    let created = crate::hive::canonical_timestamp(*created_at);
    let lane_key = lane.canonical_lane_key().unwrap();
    let group_id = match &lane {
        WorkerConversationLane::DirectMessage => None,
        WorkerConversationLane::Group { group_id } => Some(group_id.as_str()),
    };
    let execution_context = serde_json::to_string(
        &crate::storage::HiveRunExecutionContextV1::worker_conversation_neutral(
            WORKER_ID,
            1,
            lane.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages (
             session_id, role, content, created_at, idempotency_key
         ) VALUES (?1, 'user', ?2, ?3, ?4)",
        params![
            session_id,
            serde_json::json!([{"type": "text", "text": "test Worker input"}]).to_string(),
            created,
            format!("test-input:{run_id}"),
        ],
    )
    .unwrap();
    let objective_message_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO hive_runs (
             id, controller_id, session_id, kind, objective, config_json, status,
             priority, available_at, attempt_count, max_attempts, created_at,
             updated_at, worker_id, group_id, governor_origin,
             governor_lane_key, execution_context_json, objective_message_id,
             conversation_through_message_id
         ) VALUES (
             ?1, ?2, ?3, ?4, 'test Worker run', '{}', 'queued',
             0, ?5, 0, 1, ?5, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11
         )",
        params![
            run_id,
            CONTROLLER_ID,
            session_id,
            kind,
            created,
            WORKER_ID,
            group_id,
            origin.as_str(),
            lane_key,
            execution_context,
            objective_message_id,
        ],
    )
    .unwrap();
}

fn seed_unresolved_provider_call(
    store: &HiveWorkerGovernorStore,
    run_id: &str,
    call_id: &str,
    started_at: &DateTime<Utc>,
) {
    seed_running_run(
        store.conn(),
        run_id,
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        started_at,
    );
    store
        .conn()
        .execute(
            "UPDATE hive_runs SET kind = 'worker_conversation' WHERE id = ?1",
            [run_id],
        )
        .unwrap();
    assert!(matches!(
        store
            .begin_provider_call(&begin(run_id, call_id, started_at))
            .unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'recovery_required', lease_owner = NULL,
                 lease_token = NULL, lease_epoch = NULL, lease_expires_at = NULL
             WHERE id = ?1",
            [run_id],
        )
        .unwrap();
}

fn transaction(conn: &Connection) -> Transaction<'_> {
    Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap()
}

fn begin(run_id: &str, call_id: &str, at: &DateTime<Utc>) -> BeginWorkerProviderCall {
    BeginWorkerProviderCall {
        provider_call_id: call_id.to_string(),
        worker_id: WORKER_ID.to_string(),
        expected_worker_revision: 1,
        owner_user_id: Some(OWNER.to_string()),
        session_id: DM_SESSION.to_string(),
        conversation_lane: WorkerConversationLane::DirectMessage,
        run_id: run_id.to_string(),
        run_lease_token: format!("lease-{run_id}"),
        run_lease_epoch: 7,
        expected_model_key: model_key(),
        expected_model_catalog_revision: Some("catalog-v1".to_string()),
        expected_permission_mode: PermissionMode::Autonomous,
        origin: WorkerRunOrigin::UserDm,
        lane_key: "dm".to_string(),
        call_kind: "conversation".to_string(),
        workflow_goal_id: None,
        workflow_attempt_id: None,
        reserved_tokens: 100,
        pricing: Some(FrozenModelPriceSnapshot {
            currency: Some("USD".to_string()),
            input_microunits_per_million: Some(1_000_000),
            output_microunits_per_million: Some(2_000_000),
            cache_creation_microunits_per_million: None,
            cache_read_microunits_per_million: None,
            catalog_source: "live_dynamic".to_string(),
            catalog_revision: Some("catalog-v1".to_string()),
        }),
        override_grant_id: None,
        started_at: *at,
    }
}

fn finish(call_id: &str, run_id: &str, at: &DateTime<Utc>) -> FinishWorkerProviderCall {
    FinishWorkerProviderCall {
        provider_call_id: call_id.to_string(),
        worker_id: WORKER_ID.to_string(),
        run_id: run_id.to_string(),
        state: ProviderCallTerminalState::Completed,
        outcome: "success".to_string(),
        remote_acceptance: ProviderCallRemoteAcceptance::Acknowledged,
        usage: Some(Usage {
            prompt_tokens: 20,
            completion_tokens: 10,
            reasoning_tokens: 0,
            total_tokens: 30,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }),
        estimated_cost_microunits: Some(40),
        unknown_reason: None,
        finished_at: *at,
    }
}

fn set_policy(
    store: &HiveWorkerGovernorStore,
    mut update: HiveWorkerGovernorPolicyUpdate,
) -> HiveWorkerGovernorPolicy {
    let current = store.get_policy(WORKER_ID, Some(OWNER)).unwrap().unwrap();
    if update.timezone.is_empty() {
        update.timezone = current.timezone;
    }
    match store
        .compare_and_swap_policy(
            WORKER_ID,
            Some(OWNER),
            current.revision,
            &update,
            at(2026, 8, 25, 0, 0, 1),
        )
        .unwrap()
    {
        WorkerGovernorPolicyCas::Updated(policy) => policy,
        WorkerGovernorPolicyCas::Conflict(_) => panic!("unexpected policy conflict"),
    }
}

#[test]
fn migration_defaults_are_visible_and_separate_from_conversation_tokens() {
    let (store, _temp) = fixture();
    let policy = store.get_policy(WORKER_ID, Some(OWNER)).unwrap().unwrap();
    assert_eq!(policy.revision, 1);
    assert_eq!(policy.daily_call_limit, DEFAULT_WORKER_DAILY_CALL_LIMIT);
    assert_eq!(policy.daily_token_limit, DEFAULT_WORKER_DAILY_TOKEN_LIMIT);
    assert_eq!(policy.timezone, DEFAULT_WORKER_GOVERNOR_TIMEZONE);
    assert_eq!(policy.idle_base_secs, DEFAULT_WORKER_IDLE_BASE_SECS);
    assert_eq!(policy.idle_max_secs, DEFAULT_WORKER_IDLE_MAX_SECS);
    assert!(policy.quiet_start_minute.is_none());
    assert!(!policy.tracking_started_at.is_empty());

    let decision = store
        .evaluate_worker(
            WORKER_ID,
            Some(OWNER),
            WorkerRunOrigin::UserDm,
            "dm",
            1,
            at(2026, 8, 25, 1, 0, 0),
        )
        .unwrap();
    assert_eq!(decision.daily.tokens_used_or_reserved, 0);
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT token_count FROM sessions WHERE id = ?1",
                [DM_SESSION],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        777
    );
}

#[test]
fn policy_cas_is_exact_owner_revisioned_and_bounded() {
    let (store, _temp) = fixture();
    let current = store.get_policy(WORKER_ID, Some(OWNER)).unwrap().unwrap();
    let worker_revision_before: i64 = store
        .conn()
        .query_row(
            "SELECT revision FROM hive_workers WHERE id = ?1",
            [WORKER_ID],
            |row| row.get(0),
        )
        .unwrap();
    let update = HiveWorkerGovernorPolicyUpdate {
        daily_call_limit: 12,
        daily_token_limit: 42_000,
        timezone: "America/Los_Angeles".to_string(),
        quiet_start_minute: Some(22 * 60),
        quiet_end_minute: Some(6 * 60),
        ..HiveWorkerGovernorPolicyUpdate::default()
    };
    let updated = store
        .compare_and_swap_policy(
            WORKER_ID,
            Some(OWNER),
            current.revision,
            &update,
            at(2026, 8, 25, 1, 0, 0),
        )
        .unwrap();
    assert!(matches!(updated, WorkerGovernorPolicyCas::Updated(_)));
    let conflict = store
        .compare_and_swap_policy(
            WORKER_ID,
            Some(OWNER),
            current.revision,
            &update,
            at(2026, 8, 25, 1, 0, 1),
        )
        .unwrap();
    assert!(matches!(conflict, WorkerGovernorPolicyCas::Conflict(_)));
    let worker_revision_after: i64 = store
        .conn()
        .query_row(
            "SELECT revision FROM hive_workers WHERE id = ?1",
            [WORKER_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(worker_revision_after, worker_revision_before);
    assert!(store
        .compare_and_swap_policy(
            WORKER_ID,
            Some("mallory"),
            current.revision + 1,
            &update,
            at(2026, 8, 25, 1, 0, 2),
        )
        .is_err());
    let invalid = HiveWorkerGovernorPolicyUpdate {
        timezone: "Not/AZone".to_string(),
        ..HiveWorkerGovernorPolicyUpdate::default()
    };
    assert!(store
        .compare_and_swap_policy(
            WORKER_ID,
            Some(OWNER),
            current.revision + 1,
            &invalid,
            at(2026, 8, 25, 1, 0, 3),
        )
        .is_err());
}

#[test]
fn started_and_terminal_rows_are_exactly_once_and_immutable() {
    let (store, _temp) = fixture();
    let now = at(2026, 8, 25, 2, 0, 0);
    seed_running_run(
        store.conn(),
        "run-1",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &now,
    );
    let input = begin("run-1", "call-1", &now);
    let mut stale_revision = input.clone();
    stale_revision.expected_worker_revision = 2;
    assert!(store.begin_provider_call(&stale_revision).is_err());
    let first = store.begin_provider_call(&input).unwrap();
    assert!(matches!(first, BeginWorkerProviderCallResult::Started(_)));
    let mut later_replay = input.clone();
    later_replay.started_at = now + chrono::Duration::seconds(30);
    let replay = store.begin_provider_call(&later_replay).unwrap();
    assert!(matches!(
        replay,
        BeginWorkerProviderCallResult::AlreadyStarted(_)
    ));
    let mut conflict = input;
    conflict.reserved_tokens += 1;
    assert!(store.begin_provider_call(&conflict).is_err());

    let terminal = finish("call-1", "run-1", &(now + chrono::Duration::seconds(5)));
    assert!(matches!(
        store.finish_provider_call(&terminal).unwrap(),
        FinishWorkerProviderCallResult::Inserted(_)
    ));
    assert!(matches!(
        {
            let mut later_terminal = terminal;
            later_terminal.finished_at = now + chrono::Duration::seconds(45);
            store.finish_provider_call(&later_terminal).unwrap()
        },
        FinishWorkerProviderCallResult::AlreadyRecorded(_)
    ));
    let mut conflicting_terminal = finish("call-1", "run-1", &(now + chrono::Duration::seconds(5)));
    conflicting_terminal.outcome = "different".to_string();
    assert!(store.finish_provider_call(&conflicting_terminal).is_err());
    assert!(store
        .conn()
        .execute(
            "UPDATE hive_worker_provider_calls SET reserved_tokens = 1
             WHERE provider_call_id = 'call-1'",
            [],
        )
        .is_err());
    assert!(store
        .conn()
        .execute(
            "DELETE FROM hive_worker_provider_call_outcomes
             WHERE provider_call_id = 'call-1'",
            [],
        )
        .is_err());
}

#[test]
fn admission_fences_owner_lane_model_permission_and_run_lease() {
    let (store, _temp) = fixture();
    let now = at(2026, 8, 25, 3, 0, 0);
    seed_running_run(
        store.conn(),
        "run-1",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &now,
    );
    let mut input = begin("run-1", "call-owner", &now);
    input.owner_user_id = Some("mallory".to_string());
    assert!(store.begin_provider_call(&input).is_err());

    let mut input = begin("run-1", "call-lease", &now);
    input.run_lease_token = "forged".to_string();
    assert!(store.begin_provider_call(&input).is_err());

    let mut input = begin("run-1", "call-model", &now);
    input.expected_model_key = ModelKey::new(
        ProviderId::OpenAI,
        "grok-worker-test",
        ApiFormat::OpenAIResponses,
    );
    assert!(store.begin_provider_call(&input).is_err());

    let mut input = begin("run-1", "call-permission", &now);
    input.expected_permission_mode = PermissionMode::Supervised;
    assert!(store.begin_provider_call(&input).is_err());

    seed_running_run(
        store.conn(),
        "group-run",
        GROUP_SESSION,
        WorkerRunOrigin::UserGroup,
        "group:group-1",
        &now,
    );
    let mut group = begin("group-run", "group-call", &now);
    group.session_id = GROUP_SESSION.to_string();
    group.conversation_lane = WorkerConversationLane::Group {
        group_id: GROUP_ID.to_string(),
    };
    group.origin = WorkerRunOrigin::UserGroup;
    group.lane_key = "group:group-1".to_string();
    assert!(matches!(
        store.begin_provider_call(&group).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
}

#[test]
fn call_and_token_caps_use_started_reservations_not_session_tokens() {
    let (store, _temp) = fixture();
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            daily_call_limit: 1,
            daily_token_limit: 100,
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    let now = at(2026, 8, 25, 4, 0, 0);
    seed_running_run(
        store.conn(),
        "run-1",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &now,
    );
    assert!(matches!(
        store
            .begin_provider_call(&begin("run-1", "call-1", &now))
            .unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    let mut second = begin("run-1", "call-2", &(now + chrono::Duration::seconds(1)));
    second.reserved_tokens = 1;
    let BeginWorkerProviderCallResult::Gated(decision) =
        store.begin_provider_call(&second).unwrap()
    else {
        panic!("second call should be capped");
    };
    assert_eq!(
        decision.primary_reason,
        Some(WorkerGovernorGateReason::DailyCallCapReached)
    );
    assert_eq!(decision.daily.calls_used, 1);
    assert_eq!(decision.daily.tokens_used_or_reserved, 100);
}

#[test]
fn concurrent_immediate_admission_cannot_overshoot_call_cap() {
    let (store, temp) = fixture();
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            daily_call_limit: 1,
            daily_token_limit: 10_000,
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    let now = at(2026, 8, 25, 5, 0, 0);
    seed_running_run(
        store.conn(),
        "race-a",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &now,
    );
    seed_running_run(
        store.conn(),
        "race-b",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &now,
    );
    let path = temp.path().join("governor.db");
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for (run_id, call_id) in [("race-a", "race-call-a"), ("race-b", "race-call-b")] {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        let started_at = now;
        handles.push(std::thread::spawn(move || {
            let db = Database::new(&path).unwrap();
            let store = HiveWorkerGovernorStore::new(db);
            barrier.wait();
            store
                .begin_provider_call(&begin(run_id, call_id, &started_at))
                .unwrap()
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginWorkerProviderCallResult::Started(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginWorkerProviderCallResult::Gated(_)))
            .count(),
        1
    );
}

#[test]
fn one_call_override_is_consumed_atomically_and_never_bypasses_identity() {
    let (store, _temp) = fixture();
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            daily_call_limit: 1,
            daily_token_limit: 10_000,
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    let now = at(2026, 8, 25, 6, 0, 0);
    for run_id in ["run-1", "run-2", "run-3"] {
        seed_running_run(
            store.conn(),
            run_id,
            DM_SESSION,
            WorkerRunOrigin::UserDm,
            "dm",
            &now,
        );
    }
    assert!(matches!(
        store
            .begin_provider_call(&begin("run-1", "call-1", &now))
            .unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    let grant = store
        .grant_one_call_override(&GrantWorkerGovernorOverride {
            id: "override-1".to_string(),
            operation_id: "operation-1".to_string(),
            worker_id: WORKER_ID.to_string(),
            owner_user_id: Some(OWNER.to_string()),
            bypass_unresolved_provider_call: false,
            bypass_daily_call_cap: true,
            bypass_daily_token_cap: false,
            bypass_quiet_hours: false,
            bypass_idle_backoff: false,
            reason: "Owner explicitly allowed one additional provider call".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(5),
        })
        .unwrap();
    store
        .conn()
        .execute(
            "UPDATE hive_runs SET governor_override_id = ?2 WHERE id = ?1",
            rusqlite::params!["run-2", grant.id],
        )
        .unwrap();
    let mut second = begin("run-2", "call-2", &(now + chrono::Duration::seconds(1)));
    second.override_grant_id = Some(grant.id.clone());
    assert!(matches!(
        store.begin_provider_call(&second).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    let mut third = begin("run-3", "call-3", &(now + chrono::Duration::seconds(2)));
    third.override_grant_id = Some(grant.id);
    assert!(store.begin_provider_call(&third).is_err());

    store
        .conn()
        .execute(
            "UPDATE hive_workers SET status = 'paused' WHERE id = ?1",
            [WORKER_ID],
        )
        .unwrap();
    let mut paused = begin(
        "run-3",
        "call-paused",
        &(now + chrono::Duration::seconds(3)),
    );
    paused.override_grant_id = Some("override-1".to_string());
    assert!(store.begin_provider_call(&paused).is_err());
}

#[test]
fn recovery_grant_is_owner_scoped_next_direct_run_only_and_acknowledges_older_unknown() {
    let (store, _temp) = fixture();
    let started_at = at(2026, 8, 25, 6, 20, 0);
    seed_unresolved_provider_call(&store, "uncertain-run", "uncertain-call", &started_at);
    let recovery_at = started_at + chrono::Duration::minutes(11);
    let recovery_at_text = crate::hive::canonical_timestamp(recovery_at);

    let tx = transaction(store.conn());
    let wrong_owner = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some("mallory"),
        "wrong-owner-recovery",
        recovery_at,
    )
    .unwrap_err();
    assert!(wrong_owner.to_string().contains("owner mismatch"));
    let (grant, created) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "recovery-operation",
        recovery_at,
    )
    .unwrap();
    assert!(created);
    assert!(grant.bypass_unresolved_provider_call);
    assert!(!grant.bypass_daily_call_cap);
    assert!(!grant.bypass_daily_token_cap);
    assert!(!grant.bypass_quiet_hours);
    assert!(!grant.bypass_idle_backoff);
    assert_eq!(
        crate::hive::parse_utc_timestamp(&grant.expires_at).unwrap()
            - crate::hive::parse_utc_timestamp(&grant.created_at).unwrap(),
        chrono::Duration::seconds(WORKER_GOVERNOR_RECOVERY_GRANT_TTL_SECS)
    );
    let (replayed, replay_created) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "recovery-operation",
        recovery_at,
    )
    .unwrap();
    assert!(!replay_created);
    assert_eq!(replayed.id, grant.id);
    let (coalesced, coalesced_created) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "second-click-operation",
        recovery_at,
    )
    .unwrap();
    assert!(!coalesced_created);
    assert_eq!(coalesced.id, grant.id);
    tx.commit().unwrap();

    seed_queued_run(
        store.conn(),
        "group-next",
        "group_turn",
        GROUP_SESSION,
        WorkerRunOrigin::UserGroup,
        WorkerConversationLane::Group {
            group_id: GROUP_ID.to_string(),
        },
        &recovery_at,
    );
    seed_queued_run(
        store.conn(),
        "background-next",
        "worker_heartbeat",
        DM_SESSION,
        WorkerRunOrigin::Heartbeat,
        WorkerConversationLane::DirectMessage,
        &recovery_at,
    );
    let tx = transaction(store.conn());
    assert!(bind_worker_governor_recovery_grant_to_run_in_transaction(
        &tx,
        "group-next",
        &recovery_at_text,
    )
    .unwrap()
    .is_none());
    assert!(bind_worker_governor_recovery_grant_to_run_in_transaction(
        &tx,
        "background-next",
        &recovery_at_text,
    )
    .unwrap()
    .is_none());
    tx.commit().unwrap();

    seed_queued_run(
        store.conn(),
        "occurrence-next",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &recovery_at,
    );
    store
        .conn()
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    store
        .conn()
        .execute(
            "UPDATE hive_runs SET occurrence_id = 'surviving-occurrence-link'
             WHERE id = 'occurrence-next'",
            [],
        )
        .unwrap();
    store
        .conn()
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let tx = transaction(store.conn());
    assert!(bind_worker_governor_recovery_grant_to_run_in_transaction(
        &tx,
        "occurrence-next",
        &recovery_at_text,
    )
    .unwrap()
    .is_none());
    tx.commit().unwrap();

    seed_queued_run(
        store.conn(),
        "direct-next",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &(recovery_at + chrono::Duration::seconds(1)),
    );
    seed_queued_run(
        store.conn(),
        "direct-after",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &(recovery_at + chrono::Duration::seconds(2)),
    );
    let tx = transaction(store.conn());
    assert_eq!(
        bind_worker_governor_recovery_grant_to_run_in_transaction(
            &tx,
            "direct-next",
            &recovery_at_text,
        )
        .unwrap()
        .as_deref(),
        Some(grant.id.as_str())
    );
    assert!(bind_worker_governor_recovery_grant_to_run_in_transaction(
        &tx,
        "direct-after",
        &recovery_at_text,
    )
    .unwrap()
    .is_none());
    tx.commit().unwrap();

    let call_at = recovery_at + chrono::Duration::seconds(3);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', lease_owner = 'executor-recovery',
                 lease_token = ?2, lease_epoch = 7, lease_expires_at = ?3,
                 started_at = ?4, updated_at = ?4
             WHERE id = ?1",
            params![
                "direct-next",
                "lease-direct-next",
                crate::hive::canonical_timestamp(call_at + chrono::Duration::minutes(10)),
                crate::hive::canonical_timestamp(call_at),
            ],
        )
        .unwrap();
    let mut admitted = begin("direct-next", "recovery-call", &call_at);
    admitted.override_grant_id = Some(grant.id.clone());
    assert!(matches!(
        store.begin_provider_call(&admitted).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    assert!(matches!(
        store.begin_provider_call(&admitted).unwrap(),
        BeginWorkerProviderCallResult::AlreadyStarted(_)
    ));
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_worker_governor_override_consumptions
                 WHERE grant_id = ?1 AND provider_call_id = 'recovery-call'",
                [&grant.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert!(matches!(
        store
            .finish_provider_call(&finish(
                "recovery-call",
                "direct-next",
                &(call_at + chrono::Duration::seconds(1)),
            ))
            .unwrap(),
        FinishWorkerProviderCallResult::Inserted(_)
    ));

    let second_call_at = call_at + chrono::Duration::seconds(2);
    let mut second_call = begin("direct-next", "recovery-second-call", &second_call_at);
    second_call.override_grant_id = Some(grant.id.clone());
    let BeginWorkerProviderCallResult::Started(second_started) =
        store.begin_provider_call(&second_call).unwrap()
    else {
        panic!("consumed recovery provenance must not block a later same-run call")
    };
    assert!(second_started.override_grant_id.is_none());
    assert!(matches!(
        store
            .finish_provider_call(&finish(
                "recovery-second-call",
                "direct-next",
                &(second_call_at + chrono::Duration::seconds(1)),
            ))
            .unwrap(),
        FinishWorkerProviderCallResult::Inserted(_)
    ));

    let post_expiry_at =
        crate::hive::parse_utc_timestamp(&grant.expires_at).unwrap() + chrono::Duration::seconds(1);
    let mut post_expiry = begin("direct-next", "recovery-post-expiry-call", &post_expiry_at);
    post_expiry.override_grant_id = Some(grant.id.clone());
    let BeginWorkerProviderCallResult::Started(post_expiry_started) =
        store.begin_provider_call(&post_expiry).unwrap()
    else {
        panic!("same-run consumed recovery provenance must survive grant expiry")
    };
    assert!(post_expiry_started.override_grant_id.is_none());
    assert!(matches!(
        store
            .finish_provider_call(&finish(
                "recovery-post-expiry-call",
                "direct-next",
                &(post_expiry_at + chrono::Duration::seconds(1)),
            ))
            .unwrap(),
        FinishWorkerProviderCallResult::Inserted(_)
    ));
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT governor_override_id FROM hive_runs WHERE id = 'direct-next'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        grant.id
    );
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_worker_governor_override_consumptions
                 WHERE grant_id = ?1",
                [&grant.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            daily_call_limit: 4,
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    let mut capped_continuation = begin(
        "direct-next",
        "recovery-passive-cap-call",
        &(post_expiry_at + chrono::Duration::seconds(2)),
    );
    capped_continuation.override_grant_id = Some(grant.id.clone());
    let BeginWorkerProviderCallResult::Gated(capped) =
        store.begin_provider_call(&capped_continuation).unwrap()
    else {
        panic!("passive recovery provenance must not bypass a new daily cap")
    };
    assert_eq!(
        capped.reasons,
        vec![WorkerGovernorGateReason::DailyCallCapReached]
    );
    assert_eq!(
        store
            .get_run_governor_projection("direct-next")
            .unwrap()
            .unwrap()
            .override_grant_id
            .as_deref(),
        Some(grant.id.as_str())
    );

    let later = store
        .evaluate_worker(
            WORKER_ID,
            Some(OWNER),
            WorkerRunOrigin::UserDm,
            "dm",
            1,
            call_at + chrono::Duration::seconds(1),
        )
        .unwrap();
    assert!(!later
        .reasons
        .contains(&WorkerGovernorGateReason::UnresolvedProviderCall));

    seed_queued_run(
        store.conn(),
        "cross-run-consumed-reuse",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &post_expiry_at,
    );
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', governor_override_id = ?2,
                 lease_owner = 'cross-run-executor', lease_token = ?3,
                 lease_epoch = 7, lease_expires_at = ?4,
                 started_at = ?5, updated_at = ?5
             WHERE id = ?1",
            params![
                "cross-run-consumed-reuse",
                grant.id,
                "lease-cross-run-consumed-reuse",
                crate::hive::canonical_timestamp(post_expiry_at + chrono::Duration::minutes(10)),
                crate::hive::canonical_timestamp(post_expiry_at),
            ],
        )
        .unwrap();
    let mut cross_run = begin(
        "cross-run-consumed-reuse",
        "cross-run-consumed-call",
        &(post_expiry_at + chrono::Duration::seconds(1)),
    );
    cross_run.override_grant_id = Some(grant.id);
    assert!(store
        .begin_provider_call(&cross_run)
        .unwrap_err()
        .to_string()
        .contains("not bound to exactly this run"));

    seed_queued_run(
        store.conn(),
        "direct-too-late",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &(call_at + chrono::Duration::seconds(2)),
    );
    let tx = transaction(store.conn());
    assert!(bind_worker_governor_recovery_grant_to_run_in_transaction(
        &tx,
        "direct-too-late",
        &crate::hive::canonical_timestamp(call_at),
    )
    .unwrap()
    .is_none());
    tx.commit().unwrap();
}

#[test]
fn recovery_consumption_never_acknowledges_its_own_same_timestamp_unknown() {
    let (store, _temp) = fixture();
    let old_started_at = at(2026, 8, 25, 6, 30, 0);
    seed_unresolved_provider_call(
        &store,
        "same-time-old-run",
        "same-time-old-call",
        &old_started_at,
    );
    let recovery_at = old_started_at + chrono::Duration::minutes(11);
    let recovery_at_text = crate::hive::canonical_timestamp(recovery_at);
    let tx = transaction(store.conn());
    let (grant, _) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "same-time-recovery",
        recovery_at,
    )
    .unwrap();
    tx.commit().unwrap();

    seed_queued_run(
        store.conn(),
        "same-time-next-run",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &recovery_at,
    );
    let tx = transaction(store.conn());
    assert_eq!(
        bind_worker_governor_recovery_grant_to_run_in_transaction(
            &tx,
            "same-time-next-run",
            &recovery_at_text,
        )
        .unwrap()
        .as_deref(),
        Some(grant.id.as_str())
    );
    tx.commit().unwrap();
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', lease_owner = 'executor-same-time',
                 lease_token = 'lease-same-time-next-run', lease_epoch = 7,
                 lease_expires_at = ?2, started_at = ?3, updated_at = ?3
             WHERE id = ?1",
            params![
                "same-time-next-run",
                crate::hive::canonical_timestamp(recovery_at + chrono::Duration::minutes(10)),
                recovery_at_text,
            ],
        )
        .unwrap();
    let mut admitted = begin("same-time-next-run", "same-time-new-call", &recovery_at);
    admitted.override_grant_id = Some(grant.id);
    assert!(matches!(
        store.begin_provider_call(&admitted).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    let unknown_at = recovery_at + chrono::Duration::minutes(11);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'recovery_required', lease_owner = NULL,
                 lease_token = NULL, lease_epoch = NULL,
                 lease_expires_at = NULL, heartbeat_at = NULL,
                 updated_at = ?2
             WHERE id = ?1",
            params![
                "same-time-next-run",
                crate::hive::canonical_timestamp(unknown_at),
            ],
        )
        .unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO hive_daemon_leases (
                 lease_name, owner_id, fencing_token, acquired_at,
                 heartbeat_at, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            params![
                "same-time-reconciler",
                "same-time-daemon",
                17_u64,
                crate::hive::canonical_timestamp(unknown_at),
                crate::hive::canonical_timestamp(unknown_at + chrono::Duration::minutes(1)),
            ],
        )
        .unwrap();
    assert!(matches!(
        store
            .reconcile_unknown_provider_call(&ReconcileUnknownProviderCall {
                provider_call_id: "same-time-new-call".to_string(),
                worker_id: WORKER_ID.to_string(),
                run_id: "same-time-next-run".to_string(),
                daemon_lease_name: "same-time-reconciler".to_string(),
                daemon_owner_id: "same-time-daemon".to_string(),
                daemon_fencing_token: 17,
                reason: "same-timestamp recovery call lost its terminal response".to_string(),
                reconciled_at: unknown_at,
            })
            .unwrap(),
        FinishWorkerProviderCallResult::Inserted(_)
    ));

    let projection = store
        .get_worker_dm_projection(
            WORKER_ID,
            Some(OWNER),
            unknown_at + chrono::Duration::seconds(1),
        )
        .unwrap()
        .expect("exact owner projection");
    assert_eq!(projection.unresolved_started_count, 1);
    assert!(projection
        .foreground_dm
        .decision
        .reasons
        .contains(&WorkerGovernorGateReason::UnresolvedProviderCall));
}

#[test]
fn unresolved_only_recovery_never_bypasses_daily_caps() {
    let (store, _temp) = fixture();
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            daily_call_limit: 1,
            daily_token_limit: 10_000,
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    let started_at = at(2026, 8, 25, 6, 40, 0);
    seed_unresolved_provider_call(&store, "capped-old-run", "capped-old-call", &started_at);
    let recovery_at = started_at + chrono::Duration::minutes(11);
    let tx = transaction(store.conn());
    let (grant, _) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "capped-recovery",
        recovery_at,
    )
    .unwrap();
    tx.commit().unwrap();
    seed_queued_run(
        store.conn(),
        "capped-next-run",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &recovery_at,
    );
    let tx = transaction(store.conn());
    bind_worker_governor_recovery_grant_to_run_in_transaction(
        &tx,
        "capped-next-run",
        &crate::hive::canonical_timestamp(recovery_at),
    )
    .unwrap()
    .expect("the direct run should bind the narrow recovery grant");
    tx.commit().unwrap();
    let call_at = recovery_at + chrono::Duration::seconds(1);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', lease_owner = 'executor-capped',
                 lease_token = 'lease-capped-next-run', lease_epoch = 7,
                 lease_expires_at = ?2, started_at = ?3, updated_at = ?3
             WHERE id = ?1",
            params![
                "capped-next-run",
                crate::hive::canonical_timestamp(call_at + chrono::Duration::minutes(10)),
                crate::hive::canonical_timestamp(call_at),
            ],
        )
        .unwrap();
    let mut input = begin("capped-next-run", "capped-recovery-call", &call_at);
    input.reserved_tokens = 1;
    input.override_grant_id = Some(grant.id.clone());
    let BeginWorkerProviderCallResult::Gated(decision) = store.begin_provider_call(&input).unwrap()
    else {
        panic!("the unresolved-only recovery must not bypass the daily call cap");
    };
    assert_eq!(
        decision.reasons,
        vec![WorkerGovernorGateReason::DailyCallCapReached]
    );
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_worker_governor_override_consumptions
                 WHERE grant_id = ?1",
                [&grant.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn expired_bound_recovery_is_replaced_after_cap_sleep_and_later_admits() {
    let (store, _temp) = fixture();
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            daily_call_limit: 1,
            daily_token_limit: 10_000,
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    let old_started_at = at(2026, 8, 25, 23, 40, 0);
    seed_unresolved_provider_call(
        &store,
        "cap-sleep-old-run",
        "cap-sleep-old-call",
        &old_started_at,
    );
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'cancelled', finished_at = ?2, updated_at = ?2
             WHERE id = ?1 AND status = 'recovery_required'",
            params![
                "cap-sleep-old-run",
                crate::hive::canonical_timestamp(old_started_at + chrono::Duration::minutes(10)),
            ],
        )
        .unwrap();
    let first_grant_at = at(2026, 8, 25, 23, 51, 0);
    let tx = transaction(store.conn());
    let (first_grant, _) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "cap-sleep-first-recovery",
        first_grant_at,
    )
    .unwrap();
    tx.commit().unwrap();
    seed_queued_run(
        store.conn(),
        "cap-sleep-next-run",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &first_grant_at,
    );
    let tx = transaction(store.conn());
    assert_eq!(
        bind_worker_governor_recovery_grant_to_run_in_transaction(
            &tx,
            "cap-sleep-next-run",
            &crate::hive::canonical_timestamp(first_grant_at),
        )
        .unwrap()
        .as_deref(),
        Some(first_grant.id.as_str())
    );
    tx.commit().unwrap();
    let gate_at = first_grant_at + chrono::Duration::seconds(1);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', lease_owner = 'cap-sleep-executor',
                 lease_token = 'lease-cap-sleep-next-run', lease_epoch = 7,
                 lease_expires_at = ?2, started_at = ?3, updated_at = ?3
             WHERE id = ?1",
            params![
                "cap-sleep-next-run",
                crate::hive::canonical_timestamp(gate_at + chrono::Duration::minutes(10)),
                crate::hive::canonical_timestamp(gate_at),
            ],
        )
        .unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO hive_run_attempts (
                 id, run_id, attempt_no, executor_id, lease_token, lease_epoch,
                 started_at, finished_at, outcome
             ) VALUES (
                 'cap-sleep-attempt', 'cap-sleep-next-run', 1,
                 'cap-sleep-executor', 'lease-cap-sleep-next-run', 7,
                 ?1, NULL, 'leased'
             )",
            [crate::hive::canonical_timestamp(gate_at)],
        )
        .unwrap();
    let mut gated = begin("cap-sleep-next-run", "cap-sleep-gated-call", &gate_at);
    gated.override_grant_id = Some(first_grant.id.clone());
    let BeginWorkerProviderCallResult::Gated(decision) = store.begin_provider_call(&gated).unwrap()
    else {
        panic!("the narrow recovery grant must preserve the daily call cap")
    };
    assert_eq!(
        decision.reasons,
        vec![WorkerGovernorGateReason::DailyCallCapReached]
    );
    let fresh_grant_at = at(2026, 8, 25, 23, 57, 0);
    let tx = transaction(store.conn());
    let (blocked_grant, created) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "cap-sleep-fresh-recovery",
        fresh_grant_at,
    )
    .unwrap();
    assert!(created);
    assert_eq!(
        refresh_worker_governor_recovery_run_binding_in_transaction(
            &tx,
            WORKER_ID,
            Some(OWNER),
            &blocked_grant.id,
            &crate::hive::canonical_timestamp(fresh_grant_at),
        )
        .unwrap(),
        WorkerGovernorRecoveryRunBinding::BlockedInFlight {
            run_id: "cap-sleep-next-run".to_string(),
        }
    );
    tx.rollback().unwrap();
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT governor_override_id FROM hive_runs
                 WHERE id = 'cap-sleep-next-run'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        first_grant.id
    );
    let next_day = at(2026, 8, 26, 0, 0, 0);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'sleeping', lease_owner = NULL, lease_token = NULL,
                 lease_epoch = NULL, lease_expires_at = NULL,
                 heartbeat_at = NULL, wake_at = ?2,
                 governor_gate_reason = 'daily_call_cap_reached',
                 governor_next_eligible_at = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 'running'",
            params![
                "cap-sleep-next-run",
                crate::hive::canonical_timestamp(next_day),
                crate::hive::canonical_timestamp(gate_at),
            ],
        )
        .unwrap();
    store
        .conn()
        .execute(
            "UPDATE hive_run_attempts
             SET finished_at = ?2, outcome = 'sleeping'
             WHERE id = ?1 AND finished_at IS NULL",
            params![
                "cap-sleep-attempt",
                crate::hive::canonical_timestamp(gate_at),
            ],
        )
        .unwrap();

    let tx = transaction(store.conn());
    let (fresh_grant, created) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "cap-sleep-fresh-recovery",
        fresh_grant_at,
    )
    .unwrap();
    assert!(created);
    assert!(matches!(
        refresh_worker_governor_recovery_run_binding_in_transaction(
            &tx,
            WORKER_ID,
            Some(OWNER),
            &fresh_grant.id,
            &crate::hive::canonical_timestamp(fresh_grant_at),
        )
        .unwrap(),
        WorkerGovernorRecoveryRunBinding::Rebound {
            ref run_id,
            ref replaced_grant_id,
        } if run_id == "cap-sleep-next-run" && replaced_grant_id == &first_grant.id
    ));
    let (status, wake_at, bound_grant): (String, Option<String>, Option<String>) = tx
        .query_row(
            "SELECT status, wake_at, governor_override_id
             FROM hive_runs WHERE id = 'cap-sleep-next-run'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "sleeping");
    assert_eq!(
        wake_at.as_deref(),
        Some(crate::hive::canonical_timestamp(next_day).as_str())
    );
    assert_eq!(bound_grant.as_deref(), Some(fresh_grant.id.as_str()));
    tx.commit().unwrap();

    let admitted_at = next_day + chrono::Duration::seconds(1);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', wake_at = NULL,
                 lease_owner = 'cap-sleep-restarted-executor',
                 lease_token = 'lease-cap-sleep-next-run', lease_epoch = 7,
                 lease_expires_at = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 'sleeping'",
            params![
                "cap-sleep-next-run",
                crate::hive::canonical_timestamp(admitted_at + chrono::Duration::minutes(10)),
                crate::hive::canonical_timestamp(admitted_at),
            ],
        )
        .unwrap();
    let mut admitted = begin(
        "cap-sleep-next-run",
        "cap-sleep-eventual-call",
        &admitted_at,
    );
    admitted.override_grant_id = Some(fresh_grant.id.clone());
    assert!(matches!(
        store.begin_provider_call(&admitted).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT provider_call_id
                 FROM hive_worker_governor_override_consumptions
                 WHERE grant_id = ?1",
                [&fresh_grant.id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "cap-sleep-eventual-call"
    );
}

#[test]
fn terminal_provider_free_dm_transfers_even_expired_recovery_grant_to_successor() {
    let (store, _temp) = fixture();
    let old_started_at = at(2026, 8, 25, 8, 0, 0);
    seed_unresolved_provider_call(
        &store,
        "transfer-old-run",
        "transfer-old-call",
        &old_started_at,
    );
    let recovery_at = old_started_at + chrono::Duration::minutes(11);
    let tx = transaction(store.conn());
    let (grant, _) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "transfer-recovery",
        recovery_at,
    )
    .unwrap();
    tx.commit().unwrap();
    seed_queued_run(
        store.conn(),
        "transfer-predecessor",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &recovery_at,
    );
    let tx = transaction(store.conn());
    assert_eq!(
        bind_worker_governor_recovery_grant_to_run_in_transaction(
            &tx,
            "transfer-predecessor",
            &crate::hive::canonical_timestamp(recovery_at),
        )
        .unwrap()
        .as_deref(),
        Some(grant.id.as_str())
    );
    tx.commit().unwrap();
    seed_queued_run(
        store.conn(),
        "transfer-successor",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &recovery_at,
    );
    store
        .conn()
        .execute(
            "INSERT INTO hive_run_attempts (
                 id, run_id, attempt_no, executor_id, lease_token, lease_epoch,
                 started_at, finished_at, outcome
             ) VALUES (
                 'transfer-open-attempt', 'transfer-predecessor', 1,
                 'transfer-executor', 'transfer-lease', 7, ?1, NULL, 'leased'
             )",
            [crate::hive::canonical_timestamp(recovery_at)],
        )
        .unwrap();
    let finished_at =
        crate::hive::parse_utc_timestamp(&grant.expires_at).unwrap() + chrono::Duration::seconds(1);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'failed', finished_at = ?2, updated_at = ?2
             WHERE id = ?1",
            params![
                "transfer-predecessor",
                crate::hive::canonical_timestamp(finished_at),
            ],
        )
        .unwrap();
    let tx = transaction(store.conn());
    assert!(
        transfer_worker_governor_recovery_grant_to_successor_in_transaction(
            &tx,
            "transfer-predecessor",
            "transfer-successor",
        )
        .unwrap()
        .is_none()
    );
    tx.commit().unwrap();
    store
        .conn()
        .execute(
            "UPDATE hive_run_attempts
             SET finished_at = ?2, outcome = 'failed'
             WHERE id = ?1 AND finished_at IS NULL",
            params![
                "transfer-open-attempt",
                crate::hive::canonical_timestamp(finished_at),
            ],
        )
        .unwrap();
    let tx = transaction(store.conn());
    assert_eq!(
        transfer_worker_governor_recovery_grant_to_successor_in_transaction(
            &tx,
            "transfer-predecessor",
            "transfer-successor",
        )
        .unwrap()
        .as_deref(),
        Some(grant.id.as_str())
    );
    let bindings: (Option<String>, Option<String>) = tx
        .query_row(
            "SELECT
                 (SELECT governor_override_id FROM hive_runs
                  WHERE id = 'transfer-predecessor'),
                 (SELECT governor_override_id FROM hive_runs
                  WHERE id = 'transfer-successor')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(bindings, (None, Some(grant.id)));
    tx.commit().unwrap();
}

#[test]
fn projection_exposes_only_exact_acknowledged_agent_turn_response_loss() {
    let (store, _temp) = fixture();
    let started_at = at(2026, 8, 25, 9, 0, 0);
    seed_queued_run(
        store.conn(),
        "response-loss-run",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &started_at,
    );
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', lease_owner = 'response-loss-executor',
                 lease_token = 'lease-response-loss-run', lease_epoch = 7,
                 lease_expires_at = ?2, started_at = ?3, updated_at = ?3
             WHERE id = ?1",
            params![
                "response-loss-run",
                crate::hive::canonical_timestamp(started_at + chrono::Duration::minutes(10)),
                crate::hive::canonical_timestamp(started_at),
            ],
        )
        .unwrap();
    let mut call = begin("response-loss-run", "response-loss-call", &started_at);
    call.call_kind = "agent_turn".to_string();
    assert!(matches!(
        store.begin_provider_call(&call).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    let mut outcome = finish(
        "response-loss-call",
        "response-loss-run",
        &(started_at + chrono::Duration::seconds(1)),
    );
    outcome.outcome = "completed".to_string();
    assert!(matches!(
        store.finish_provider_call(&outcome).unwrap(),
        FinishWorkerProviderCallResult::Inserted(_)
    ));
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'recovery_required', lease_owner = NULL,
                 lease_token = NULL, lease_epoch = NULL,
                 lease_expires_at = NULL, heartbeat_at = NULL,
                 updated_at = ?2
             WHERE id = ?1",
            params![
                "response-loss-run",
                crate::hive::canonical_timestamp(started_at + chrono::Duration::seconds(2)),
            ],
        )
        .unwrap();
    store
        .conn()
        .execute(
            "UPDATE hive_controllers SET status = 'paused' WHERE id = ?1",
            [CONTROLLER_ID],
        )
        .unwrap();
    let tx = transaction(store.conn());
    assert!(worker_governor_response_loss_recovery_required_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
    )
    .unwrap());
    assert!(
        !worker_governor_response_loss_recovery_required_in_transaction(
            &tx,
            WORKER_ID,
            Some("mallory"),
        )
        .unwrap()
    );
    tx.commit().unwrap();
    let projection = store
        .get_worker_dm_projection(
            WORKER_ID,
            Some(OWNER),
            started_at + chrono::Duration::seconds(3),
        )
        .unwrap()
        .unwrap();
    assert_eq!(projection.unresolved_started_count, 0);
    assert!(projection.response_loss_recovery_required);
}

#[test]
fn active_specialized_unknown_fails_closed_but_terminal_unknown_can_be_acknowledged() {
    let (store, _temp) = fixture();
    let started_at = at(2026, 8, 25, 7, 0, 0);
    seed_running_run(
        store.conn(),
        "background-uncertain",
        DM_SESSION,
        WorkerRunOrigin::Heartbeat,
        "dm",
        &started_at,
    );
    let mut background = begin(
        "background-uncertain",
        "background-uncertain-call",
        &started_at,
    );
    background.origin = WorkerRunOrigin::Heartbeat;
    assert!(matches!(
        store.begin_provider_call(&background).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'recovery_required', lease_owner = NULL,
                 lease_token = NULL, lease_epoch = NULL, lease_expires_at = NULL
             WHERE id = 'background-uncertain'",
            [],
        )
        .unwrap();
    let recovery_at = started_at + chrono::Duration::minutes(11);
    let tx = transaction(store.conn());
    let active_error = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "active-specialized-recovery",
        recovery_at,
    )
    .unwrap_err();
    assert!(active_error.to_string().contains("active background"));
    drop(tx);
    assert_eq!(
        store
            .conn()
            .execute(
                "UPDATE hive_runs
                 SET status = 'cancelled', finished_at = ?2, updated_at = ?2
                 WHERE id = ?1 AND status = 'recovery_required'",
                params![
                    "background-uncertain",
                    crate::hive::canonical_timestamp(recovery_at),
                ],
            )
            .unwrap(),
        1
    );
    let tx = transaction(store.conn());
    let (grant, created) = grant_worker_governor_recovery_in_transaction(
        &tx,
        WORKER_ID,
        Some(OWNER),
        "terminal-specialized-recovery",
        recovery_at,
    )
    .unwrap();
    assert!(created);
    tx.commit().unwrap();

    seed_queued_run(
        store.conn(),
        "terminal-specialized-next-dm",
        "worker_conversation",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        WorkerConversationLane::DirectMessage,
        &recovery_at,
    );
    let tx = transaction(store.conn());
    assert_eq!(
        bind_worker_governor_recovery_grant_to_run_in_transaction(
            &tx,
            "terminal-specialized-next-dm",
            &crate::hive::canonical_timestamp(recovery_at),
        )
        .unwrap()
        .as_deref(),
        Some(grant.id.as_str())
    );
    tx.commit().unwrap();
    let call_at = recovery_at + chrono::Duration::seconds(1);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'running', lease_owner = 'executor-terminal-specialized',
                 lease_token = 'lease-terminal-specialized-next-dm', lease_epoch = 7,
                 lease_expires_at = ?2, started_at = ?3, updated_at = ?3
             WHERE id = ?1",
            params![
                "terminal-specialized-next-dm",
                crate::hive::canonical_timestamp(call_at + chrono::Duration::minutes(10)),
                crate::hive::canonical_timestamp(call_at),
            ],
        )
        .unwrap();
    let mut direct = begin(
        "terminal-specialized-next-dm",
        "terminal-specialized-recovery-call",
        &call_at,
    );
    direct.override_grant_id = Some(grant.id);
    assert!(matches!(
        store.begin_provider_call(&direct).unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    let after = store
        .evaluate_worker(
            WORKER_ID,
            Some(OWNER),
            WorkerRunOrigin::UserDm,
            "dm",
            1,
            call_at + chrono::Duration::seconds(1),
        )
        .unwrap();
    assert!(!after
        .reasons
        .contains(&WorkerGovernorGateReason::UnresolvedProviderCall));
}

#[test]
fn expired_started_call_requires_fenced_unknown_and_never_replays() {
    let (store, _temp) = fixture();
    let now = at(2026, 8, 25, 7, 0, 0);
    seed_running_run(
        store.conn(),
        "run-1",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &now,
    );
    assert!(matches!(
        store
            .begin_provider_call(&begin("run-1", "call-1", &now))
            .unwrap(),
        BeginWorkerProviderCallResult::Started(_)
    ));
    let reconcile_at = now + chrono::Duration::minutes(11);
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET status = 'recovery_required', lease_token = NULL,
                 lease_epoch = NULL, lease_expires_at = NULL
             WHERE id = 'run-1'",
            [],
        )
        .unwrap();
    assert!(matches!(
        store
            .begin_provider_call(&begin("run-1", "call-1", &now))
            .unwrap(),
        BeginWorkerProviderCallResult::AlreadyStarted(_)
    ));
    seed_running_run(
        store.conn(),
        "run-2",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &reconcile_at,
    );
    let BeginWorkerProviderCallResult::Gated(stale_gate) = store
        .begin_provider_call(&begin("run-2", "call-2", &reconcile_at))
        .unwrap()
    else {
        panic!("a stale Started call must fail closed before reconciliation")
    };
    assert_eq!(
        stale_gate.primary_reason,
        Some(WorkerGovernorGateReason::UnresolvedProviderCall)
    );
    store
        .conn()
        .execute(
            "INSERT INTO hive_daemon_leases (
                 lease_name, owner_id, fencing_token, acquired_at,
                 heartbeat_at, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            params![
                "scheduler",
                "daemon-new",
                9_u64,
                crate::hive::canonical_timestamp(reconcile_at),
                crate::hive::canonical_timestamp(reconcile_at + chrono::Duration::minutes(1)),
            ],
        )
        .unwrap();
    let reconciliation = ReconcileUnknownProviderCall {
        provider_call_id: "call-1".to_string(),
        worker_id: WORKER_ID.to_string(),
        run_id: "run-1".to_string(),
        daemon_lease_name: "scheduler".to_string(),
        daemon_owner_id: "daemon-new".to_string(),
        daemon_fencing_token: 9,
        reason: "original executor lease expired without a terminal response".to_string(),
        reconciled_at: reconcile_at,
    };
    assert!(matches!(
        store
            .reconcile_unknown_provider_call(&reconciliation)
            .unwrap(),
        FinishWorkerProviderCallResult::Inserted(_)
    ));
    assert!(matches!(
        store
            .reconcile_unknown_provider_call(&reconciliation)
            .unwrap(),
        FinishWorkerProviderCallResult::AlreadyRecorded(_)
    ));
    assert!(store
        .finish_provider_call(&finish(
            "call-1",
            "run-1",
            &(reconcile_at + chrono::Duration::seconds(1)),
        ))
        .is_err());
}

#[test]
fn foreground_bypasses_quiet_and_idle_but_autonomous_work_defers() {
    let (store, _temp) = fixture();
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            timezone: "UTC".to_string(),
            quiet_start_minute: Some(22 * 60),
            quiet_end_minute: Some(6 * 60),
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    store
        .conn()
        .execute(
            "INSERT INTO hive_worker_idle_state (
                 worker_id, lane_key, idle_streak, not_before,
                 last_outcome_run_id, updated_at
             ) VALUES (?1, 'heartbeat', 2, ?2, 'prior-run', ?3)",
            params![
                WORKER_ID,
                "2026-08-25T05:00:00.000000Z",
                "2026-08-25T01:00:00.000000Z",
            ],
        )
        .unwrap();
    let now = at(2026, 8, 25, 2, 0, 0);
    let autonomous = store
        .evaluate_worker(
            WORKER_ID,
            Some(OWNER),
            WorkerRunOrigin::Heartbeat,
            "heartbeat",
            1,
            now,
        )
        .unwrap();
    assert_eq!(autonomous.disposition, WorkerGovernorDisposition::Defer);
    assert_eq!(
        autonomous.reasons,
        vec![
            WorkerGovernorGateReason::QuietHours,
            WorkerGovernorGateReason::IdleBackoff,
        ]
    );
    assert_eq!(
        autonomous.next_eligible_at.as_deref(),
        Some("2026-08-25T06:00:00.000000Z")
    );
    let foreground = store
        .evaluate_worker(
            WORKER_ID,
            Some(OWNER),
            WorkerRunOrigin::UserDm,
            "heartbeat",
            1,
            now,
        )
        .unwrap();
    assert_eq!(foreground.disposition, WorkerGovernorDisposition::Allow);
    assert!(foreground.reasons.is_empty());
}

#[test]
fn idle_outcome_is_run_fenced_exponential_idempotent_and_material_resettable() {
    let (store, _temp) = fixture();
    let first_at = at(2026, 8, 25, 8, 0, 0);
    for (run_id, finished_at) in [
        ("idle-1", first_at),
        ("idle-2", first_at + chrono::Duration::hours(1)),
        ("idle-3", first_at + chrono::Duration::hours(2)),
    ] {
        seed_running_run(
            store.conn(),
            run_id,
            DM_SESSION,
            WorkerRunOrigin::Heartbeat,
            "dm",
            &(finished_at - chrono::Duration::seconds(1)),
        );
        store
            .conn()
            .execute(
                "UPDATE hive_runs
                 SET status = 'succeeded', finished_at = ?2,
                     lease_owner = NULL, lease_token = NULL, lease_epoch = NULL,
                     lease_expires_at = NULL
                 WHERE id = ?1",
                params![run_id, crate::hive::canonical_timestamp(finished_at)],
            )
            .unwrap();
    }
    let first = RecordWorkerIdleOutcome {
        worker_id: WORKER_ID.to_string(),
        owner_user_id: Some(OWNER.to_string()),
        run_id: "idle-1".to_string(),
        lane_key: "dm".to_string(),
        origin: WorkerRunOrigin::Heartbeat,
        material: false,
        completed_at: first_at,
    };
    let WorkerIdleOutcome::Updated(first_projection) = store.record_idle_outcome(&first).unwrap()
    else {
        panic!("first outcome should update")
    };
    assert_eq!(first_projection.idle_streak, 1);
    assert_eq!(
        first_projection.not_before.as_deref(),
        Some("2026-08-25T08:15:00.000000Z")
    );
    assert!(matches!(
        store.record_idle_outcome(&first).unwrap(),
        WorkerIdleOutcome::AlreadyRecorded(_)
    ));

    let second = RecordWorkerIdleOutcome {
        run_id: "idle-2".to_string(),
        completed_at: first_at + chrono::Duration::hours(1),
        ..first.clone()
    };
    let WorkerIdleOutcome::Updated(second_projection) = store.record_idle_outcome(&second).unwrap()
    else {
        panic!("second outcome should update")
    };
    assert_eq!(second_projection.idle_streak, 2);
    assert_eq!(
        second_projection.not_before.as_deref(),
        Some("2026-08-25T09:30:00.000000Z")
    );
    assert!(store.record_idle_outcome(&first).is_err());

    let material = RecordWorkerIdleOutcome {
        run_id: "idle-3".to_string(),
        completed_at: first_at + chrono::Duration::hours(2),
        material: true,
        ..first
    };
    let WorkerIdleOutcome::Updated(material_projection) =
        store.record_idle_outcome(&material).unwrap()
    else {
        panic!("material outcome should update")
    };
    assert_eq!(material_projection.idle_streak, 0);
    assert!(material_projection.not_before.is_none());
    assert_eq!(
        material_projection.last_material_at.as_deref(),
        Some("2026-08-25T10:00:00.000000Z")
    );
}

#[test]
fn peer_response_without_typed_effect_is_not_guessed_idle_or_material() {
    let (store, _temp) = fixture();
    let finished_at = at(2026, 8, 25, 11, 0, 0);
    seed_running_run(
        store.conn(),
        "peer-success",
        DM_SESSION,
        WorkerRunOrigin::WorkerPeer,
        "dm",
        &(finished_at - chrono::Duration::seconds(1)),
    );
    store
        .conn()
        .execute(
            "UPDATE hive_runs
             SET kind = 'worker_message', status = 'succeeded', finished_at = ?2,
                 lease_owner = NULL, lease_token = NULL, lease_epoch = NULL,
                 lease_expires_at = NULL, updated_at = ?2
             WHERE id = ?1",
            params![
                "peer-success",
                crate::hive::canonical_timestamp(finished_at)
            ],
        )
        .unwrap();
    let tx = Transaction::new_unchecked(store.conn(), TransactionBehavior::Immediate).unwrap();
    assert!(
        record_trusted_worker_idle_outcome_in_transaction(&tx, "peer-success")
            .unwrap()
            .is_none()
    );
    tx.commit().unwrap();
    let rows: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_worker_idle_state
             WHERE worker_id = ?1",
            [WORKER_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

fn policy_for_time(
    timezone: &str,
    start: Option<u16>,
    end: Option<u16>,
) -> HiveWorkerGovernorPolicy {
    HiveWorkerGovernorPolicy {
        worker_id: WORKER_ID.to_string(),
        revision: 1,
        daily_call_limit: DEFAULT_WORKER_DAILY_CALL_LIMIT,
        daily_token_limit: DEFAULT_WORKER_DAILY_TOKEN_LIMIT,
        timezone: timezone.to_string(),
        quiet_start_minute: start,
        quiet_end_minute: end,
        quiet_gap_policy: DstGapPolicy::ShiftForward,
        quiet_fold_policy: DstFoldPolicy::First,
        idle_base_secs: DEFAULT_WORKER_IDLE_BASE_SECS,
        idle_max_secs: DEFAULT_WORKER_IDLE_MAX_SECS,
        tracking_started_at: "2026-01-01T00:00:00.000000Z".to_string(),
        created_at: "2026-01-01T00:00:00.000000Z".to_string(),
        updated_at: "2026-01-01T00:00:00.000000Z".to_string(),
    }
}

#[test]
fn local_day_and_quiet_windows_are_dst_and_overnight_safe() {
    let policy = policy_for_time("America/Los_Angeles", Some(22 * 60), Some(6 * 60));
    let spring = worker_local_day_window(&policy, at(2026, 3, 8, 18, 0, 0)).unwrap();
    assert_eq!(
        spring.ends_at - spring.starts_at,
        chrono::Duration::hours(23)
    );
    let fall = worker_local_day_window(&policy, at(2026, 11, 1, 18, 0, 0)).unwrap();
    assert_eq!(fall.ends_at - fall.starts_at, chrono::Duration::hours(25));

    let overnight = worker_quiet_window_at(&policy, at(2026, 1, 15, 13, 0, 0))
        .unwrap()
        .unwrap();
    assert_eq!(overnight.ends_at, at(2026, 1, 15, 14, 0, 0));

    let mut gap = policy_for_time("America/Los_Angeles", Some(2 * 60 + 30), Some(4 * 60));
    let shifted = worker_quiet_window_at(&gap, at(2026, 3, 8, 10, 15, 0))
        .unwrap()
        .unwrap();
    assert_eq!(shifted.starts_at, at(2026, 3, 8, 10, 0, 0));
    gap.quiet_gap_policy = DstGapPolicy::Skip;
    assert!(worker_quiet_window_at(&gap, at(2026, 3, 8, 10, 15, 0))
        .unwrap()
        .is_none());

    let fold = policy_for_time("America/Los_Angeles", Some(90), Some(150));
    let first_fold = worker_quiet_window_at(&fold, at(2026, 11, 1, 8, 45, 0))
        .unwrap()
        .unwrap();
    assert_eq!(first_fold.starts_at, at(2026, 11, 1, 8, 30, 0));
}

#[test]
fn frozen_pricing_survives_catalog_drift_and_trace_cleanup() {
    let (store, _temp) = fixture();
    let now = at(2026, 8, 25, 11, 0, 0);
    seed_running_run(
        store.conn(),
        "run-1",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &now,
    );
    let BeginWorkerProviderCallResult::Started(started) = store
        .begin_provider_call(&begin("run-1", "call-1", &now))
        .unwrap()
    else {
        panic!("call should start")
    };
    store
        .conn()
        .execute(
            "UPDATE hive_workers SET model_catalog_revision = 'catalog-v2'
             WHERE id = ?1",
            [WORKER_ID],
        )
        .unwrap();
    store
        .conn()
        .execute("DELETE FROM runtime_traces", [])
        .unwrap();
    let reloaded = store.get_provider_call("call-1").unwrap().unwrap();
    assert_eq!(
        reloaded.model_catalog_revision.as_deref(),
        Some("catalog-v1")
    );
    assert_eq!(reloaded.pricing, started.pricing);
    assert_eq!(
        reloaded.pricing.unwrap().input_microunits_per_million,
        Some(1_000_000)
    );
}

#[test]
fn dm_projection_is_owner_bound_and_reports_gates_usage_and_frozen_cost() {
    let (store, _temp) = fixture();
    set_policy(
        &store,
        HiveWorkerGovernorPolicyUpdate {
            timezone: "UTC".to_string(),
            quiet_start_minute: Some(22 * 60),
            quiet_end_minute: Some(6 * 60),
            ..HiveWorkerGovernorPolicyUpdate::default()
        },
    );
    store
        .conn()
        .execute(
            "INSERT INTO hive_worker_idle_state (
                 worker_id, lane_key, idle_streak, not_before,
                 last_outcome_run_id, updated_at
             ) VALUES (?1, 'dm', 2, ?2, 'prior-idle-run', ?3)",
            params![
                WORKER_ID,
                "2026-08-25T03:00:00.000000Z",
                "2026-08-25T01:00:00.000000Z",
            ],
        )
        .unwrap();

    let first_at = at(2026, 8, 25, 1, 0, 0);
    seed_running_run(
        store.conn(),
        "projection-priced-run",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &first_at,
    );
    store
        .begin_provider_call(&begin(
            "projection-priced-run",
            "projection-priced-call",
            &first_at,
        ))
        .unwrap();
    store
        .finish_provider_call(&finish(
            "projection-priced-call",
            "projection-priced-run",
            &(first_at + chrono::Duration::minutes(1)),
        ))
        .unwrap();

    let second_at = at(2026, 8, 25, 1, 10, 0);
    seed_running_run(
        store.conn(),
        "projection-unpriced-run",
        DM_SESSION,
        WorkerRunOrigin::UserDm,
        "dm",
        &second_at,
    );
    let mut unpriced = begin(
        "projection-unpriced-run",
        "projection-unpriced-call",
        &second_at,
    );
    unpriced.pricing = None;
    store.begin_provider_call(&unpriced).unwrap();

    let projection = store
        .get_worker_dm_projection(WORKER_ID, Some(OWNER), at(2026, 8, 25, 2, 0, 0))
        .unwrap()
        .expect("exact owner and DM should project");
    assert_eq!(projection.schema_version, 1);
    assert_eq!(projection.worker_id, WORKER_ID);
    assert_eq!(projection.worker_revision, 1);
    assert_eq!(projection.dm_session_id, DM_SESSION);
    assert_eq!(projection.policy.revision, 2);
    assert_eq!(projection.daily.calls_used, 2);
    assert_eq!(projection.daily.tokens_used_or_reserved, 130);
    assert_eq!(projection.unresolved_started_count, 1);
    assert_eq!(projection.estimated_daily_cost.by_currency.len(), 1);
    assert_eq!(
        projection.estimated_daily_cost.by_currency[0],
        WorkerGovernorCurrencyCost {
            currency: "USD".to_string(),
            estimated_cost_microunits: "40".to_string(),
            priced_call_count: 1,
        }
    );
    assert_eq!(projection.estimated_daily_cost.unpriced_call_count, 1);
    assert_eq!(
        projection.autonomous_dm.origin,
        WorkerRunOrigin::WorkflowRollover
    );
    assert_eq!(projection.autonomous_dm.lane_key, "dm");
    assert_eq!(projection.autonomous_dm.reservation_tokens, 1);
    assert_eq!(
        projection.autonomous_dm.decision.reasons,
        vec![
            WorkerGovernorGateReason::UnresolvedProviderCall,
            WorkerGovernorGateReason::QuietHours,
            WorkerGovernorGateReason::IdleBackoff,
        ]
    );
    assert_eq!(
        projection.autonomous_dm.decision.disposition,
        WorkerGovernorDisposition::Deny
    );
    assert!(projection.autonomous_dm.decision.next_eligible_at.is_none());
    assert_eq!(
        projection.foreground_dm.decision.reasons,
        vec![WorkerGovernorGateReason::UnresolvedProviderCall]
    );
    assert!(store
        .get_worker_dm_projection(WORKER_ID, Some("bob"), at(2026, 8, 25, 2, 0, 0))
        .unwrap()
        .is_none());
    assert!(store
        .get_worker_dm_projection(WORKER_ID, None, at(2026, 8, 25, 2, 0, 0))
        .unwrap()
        .is_none());

    store
        .conn()
        .execute(
            "UPDATE hive_workers SET dm_session_id = NULL WHERE id = ?1",
            [WORKER_ID],
        )
        .unwrap();
    assert!(store
        .get_worker_dm_projection(WORKER_ID, Some(OWNER), at(2026, 8, 25, 2, 0, 0),)
        .unwrap()
        .is_none());
}
