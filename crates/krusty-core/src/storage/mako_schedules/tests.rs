use chrono::{NaiveDate, NaiveTime};
use tempfile::TempDir;

use crate::mako::{DstPolicy, MisfireConfig, RecurrenceV1, RetryJitter, RetryPolicy};
use crate::storage::Database;

use super::{
    MakoSchedule, MakoScheduleOccurrence, MakoScheduleOccurrenceStatus, MakoScheduleStatus,
    MakoScheduleStore, OverlapPolicy,
};

fn store() -> (MakoScheduleStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("schedules.db")).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('session-1', 'Mako controller', '2026-07-01T00:00:00.000000Z',
                     '2026-07-01T00:00:00.000000Z', 'mako');
             INSERT INTO mako_controllers (
                 id, scope_key, user_id, session_id, status, timezone,
                 max_concurrent_runs, created_at, updated_at
             ) VALUES (
                 'controller-1', 'local:test', NULL, 'session-1', 'active',
                 'America/Los_Angeles', 2, '2026-07-01T00:00:00.000000Z',
                 '2026-07-01T00:00:00.000000Z'
             );",
        )
        .unwrap();
    (MakoScheduleStore::new(db), temp)
}

fn schedule() -> MakoSchedule {
    MakoSchedule {
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
        status: MakoScheduleStatus::Enabled,
        priority: 0,
        project_dir: Some("/work/repo".into()),
        model: None,
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
}

#[test]
fn occurrence_identity_is_idempotent_per_logical_fire_time() {
    let (store, _temp) = store();
    store.insert_schedule(&schedule()).unwrap();
    let occurrence = MakoScheduleOccurrence {
        id: "occurrence-1".into(),
        schedule_id: "schedule-1".into(),
        scheduled_for: "2026-07-01T16:30:00Z".into(),
        run_id: None,
        status: MakoScheduleOccurrenceStatus::Pending,
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
            MakoScheduleStatus::Cancelled,
            "2026-07-01T00:00:01Z",
        )
        .unwrap());
    assert!(store
        .set_status(
            "schedule-1",
            1,
            MakoScheduleStatus::Enabled,
            "2026-07-01T00:00:02Z",
        )
        .is_err());
}
