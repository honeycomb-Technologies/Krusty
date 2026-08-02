use chrono::{NaiveDate, NaiveTime};
use tempfile::TempDir;

use crate::ai::models::{ApiFormat, ModelAuthScope, ModelKey};
use crate::ai::providers::ProviderId;
use crate::hive::{DstPolicy, MisfireConfig, RecurrenceV1, RetryJitter, RetryPolicy};
use crate::storage::Database;

use super::{
    HiveSchedule, HiveScheduleOccurrence, HiveScheduleOccurrenceStatus, HiveScheduleStatus,
    HiveScheduleStore, OverlapPolicy,
};

fn store() -> (HiveScheduleStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("schedules.db")).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('session-1', 'Hive controller', '2026-07-01T00:00:00.000000Z',
                     '2026-07-01T00:00:00.000000Z', 'hive');
             INSERT INTO hive_controllers (
                 id, scope_key, user_id, session_id, status, timezone,
                 max_concurrent_runs, created_at, updated_at
             ) VALUES (
                 'controller-1', 'local:test', NULL, 'session-1', 'active',
                 'America/Los_Angeles', 2, '2026-07-01T00:00:00.000000Z',
                 '2026-07-01T00:00:00.000000Z'
             );",
        )
        .unwrap();
    (HiveScheduleStore::new(db), temp)
}

fn schedule() -> HiveSchedule {
    HiveSchedule {
        id: "schedule-1".into(),
        controller_id: "controller-1".into(),
        title: "Morning health check".into(),
        summary: "Inspect the repository".into(),
        objective: "Run repository health checks".into(),
        recurrence: RecurrenceV1::Weekdays {
            start_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            time: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
        },
        timezone: "America/Los_Angeles".into(),
        dst_policy: DstPolicy::default(),
        next_fire_at: Some("2026-07-01T16:30:00Z".into()),
        last_scheduled_for: None,
        status: HiveScheduleStatus::Enabled,
        priority: 0,
        project_dir: Some("/work/repo".into()),
        model: Some("grok-code-fast-1".into()),
        model_key: Some(
            ModelKey::new(
                ProviderId::Grok,
                "grok-code-fast-1",
                ApiFormat::OpenAIResponses,
            )
            .with_auth_scope(ModelAuthScope::OAuth),
        ),
        model_catalog_revision: Some("catalog-42".into()),
        crew_slug: Some("reviewer".into()),
        misfire: MisfireConfig::default(),
        overlap_policy: OverlapPolicy::QueueOne,
        retry: RetryPolicy {
            jitter: RetryJitter::Full,
            ..RetryPolicy::default()
        },
        revision: 0,
        created_by: "user".into(),
        created_at: "2026-07-01T00:00:00Z".into(),
        updated_at: "2026-07-01T00:00:00Z".into(),
    }
}

#[test]
fn schedule_round_trips_typed_recurrence_and_policies() {
    let (store, _temp) = store();
    store.insert_schedule(&schedule()).unwrap();
    let loaded = store.get_schedule("schedule-1").unwrap().unwrap();
    assert_eq!(loaded.recurrence.kind_name(), "weekdays");
    assert_eq!(loaded.overlap_policy, OverlapPolicy::QueueOne);
    assert_eq!(loaded.timezone, "America/Los_Angeles");
    assert_eq!(loaded.model_key, schedule().model_key);
    assert_eq!(loaded.model_catalog_revision.as_deref(), Some("catalog-42"));
}

#[test]
fn occurrence_identity_is_idempotent_per_logical_fire_time() {
    let (store, _temp) = store();
    store.insert_schedule(&schedule()).unwrap();
    let occurrence = HiveScheduleOccurrence {
        id: "occurrence-1".into(),
        schedule_id: "schedule-1".into(),
        scheduled_for: "2026-07-01T16:30:00Z".into(),
        run_id: None,
        status: HiveScheduleOccurrenceStatus::Pending,
        decision_reason: None,
        coalesced_count: 0,
        created_at: "2026-07-01T16:30:01Z".into(),
        updated_at: "2026-07-01T16:30:01Z".into(),
    };
    assert!(store.insert_occurrence(&occurrence).unwrap());
    assert!(!store.insert_occurrence(&occurrence).unwrap());
}

#[test]
fn optimistic_revision_prevents_stale_schedule_advance() {
    let (store, _temp) = store();
    store.insert_schedule(&schedule()).unwrap();
    assert!(store
        .advance_schedule(
            "schedule-1",
            0,
            "2026-07-01T16:30:00Z",
            Some("2026-07-02T16:30:00Z"),
            "2026-07-01T16:30:01Z",
        )
        .unwrap());
    assert!(!store
        .advance_schedule(
            "schedule-1",
            0,
            "2026-07-02T16:30:00Z",
            None,
            "2026-07-02T16:30:01Z",
        )
        .unwrap());
}

#[test]
fn cancelled_schedule_cannot_be_reenabled() {
    let (store, _temp) = store();
    store.insert_schedule(&schedule()).unwrap();
    assert!(store
        .set_status(
            "schedule-1",
            0,
            HiveScheduleStatus::Cancelled,
            "2026-07-01T00:00:01Z",
        )
        .unwrap());
    assert!(store
        .set_status(
            "schedule-1",
            1,
            HiveScheduleStatus::Enabled,
            "2026-07-01T00:00:02Z",
        )
        .is_err());
}

#[test]
fn list_for_user_is_scoped_by_controller_owner() {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("schedules-user.db")).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES
               ('session-a', 'A', '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z', 'hive'),
               ('session-b', 'B', '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z', 'hive'),
               ('session-c', 'C', '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z', 'hive');
             INSERT INTO hive_controllers (
                 id, scope_key, user_id, session_id, status, timezone,
                 max_concurrent_runs, created_at, updated_at
             ) VALUES
               ('ctrl-a', 'local:a', 'user-1', 'session-a', 'active', 'UTC', 2,
                '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z'),
               ('ctrl-b', 'local:b', 'user-1', 'session-b', 'active', 'UTC', 2,
                '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z'),
               ('ctrl-c', 'local:c', 'user-2', 'session-c', 'active', 'UTC', 2,
                '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z');",
        )
        .unwrap();
    let store = HiveScheduleStore::new(db);

    let mut owned_a = schedule();
    owned_a.id = "schedule-a".into();
    owned_a.controller_id = "ctrl-a".into();
    owned_a.next_fire_at = Some("2026-07-03T10:00:00Z".into());
    store.insert_schedule(&owned_a).unwrap();

    let mut owned_b = schedule();
    owned_b.id = "schedule-b".into();
    owned_b.controller_id = "ctrl-b".into();
    owned_b.next_fire_at = Some("2026-07-02T10:00:00Z".into());
    store.insert_schedule(&owned_b).unwrap();

    let mut other = schedule();
    other.id = "schedule-c".into();
    other.controller_id = "ctrl-c".into();
    other.next_fire_at = Some("2026-07-01T10:00:00Z".into());
    store.insert_schedule(&other).unwrap();

    let for_user_1 = store.list_for_user(Some("user-1"), 50).unwrap();
    assert_eq!(
        for_user_1
            .iter()
            .map(|s| s.schedule.id.as_str())
            .collect::<Vec<_>>(),
        vec!["schedule-b", "schedule-a"]
    );
    assert_eq!(for_user_1[0].controller_session_id, "session-b");
    assert_eq!(for_user_1[1].controller_session_id, "session-a");

    let for_user_2 = store.list_for_user(Some("user-2"), 50).unwrap();
    assert_eq!(for_user_2.len(), 1);
    assert_eq!(for_user_2[0].schedule.id, "schedule-c");
    assert_eq!(for_user_2[0].controller_session_id, "session-c");

    assert!(store
        .list_for_user(Some("user-missing"), 50)
        .unwrap()
        .is_empty());
    assert!(store.list_for_user(None, 50).unwrap().is_empty());
}
