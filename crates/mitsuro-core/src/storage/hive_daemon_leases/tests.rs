use std::time::Duration;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use crate::storage::Database;

use super::{DaemonLeaseAcquire, HiveDaemonLeaseStore};

fn instant(second: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, second)
        .single()
        .unwrap()
}

fn store() -> (HiveDaemonLeaseStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("daemon-leases.db")).unwrap();
    (HiveDaemonLeaseStore::new(db), temp)
}

#[test]
fn active_owner_is_renewed_without_changing_the_fence() {
    let (store, _temp) = store();
    let first = store
        .acquire("scheduler", "daemon-a", instant(0), Duration::from_secs(10))
        .unwrap();
    let renewed = store
        .acquire("scheduler", "daemon-a", instant(1), Duration::from_secs(10))
        .unwrap();
    match (first, renewed) {
        (DaemonLeaseAcquire::Acquired(first), DaemonLeaseAcquire::Acquired(renewed)) => {
            assert_eq!(first.fencing_token, renewed.fencing_token);
            assert!(renewed.expires_at > first.expires_at);
        }
        value => panic!("unexpected acquisition results: {value:?}"),
    }
}

#[test]
fn takeover_requires_expiry_and_increments_the_fence() {
    let (store, _temp) = store();
    store
        .acquire("scheduler", "daemon-a", instant(0), Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        store
            .acquire("scheduler", "daemon-b", instant(4), Duration::from_secs(5))
            .unwrap(),
        DaemonLeaseAcquire::HeldByOther { .. }
    ));
    let taken = store
        .acquire("scheduler", "daemon-b", instant(5), Duration::from_secs(5))
        .unwrap();
    match taken {
        DaemonLeaseAcquire::Acquired(lease) => {
            assert_eq!(lease.owner_id, "daemon-b");
            assert_eq!(lease.fencing_token, 2);
        }
        value => panic!("expected takeover, got {value:?}"),
    }
    assert!(!store
        .heartbeat(
            "scheduler",
            "daemon-a",
            1,
            instant(6),
            Duration::from_secs(5),
        )
        .unwrap());
}

#[test]
fn release_retains_generation_and_stable_owner_reacquire_increments_fence() {
    let (store, _temp) = store();
    let first = match store
        .acquire(
            "scheduler",
            "stable-daemon",
            instant(0),
            Duration::from_secs(10),
        )
        .unwrap()
    {
        DaemonLeaseAcquire::Acquired(lease) => lease,
        value => panic!("unexpected acquisition result: {value:?}"),
    };
    assert!(store
        .release("scheduler", "stable-daemon", first.fencing_token)
        .unwrap());
    let released = store.get("scheduler").unwrap().unwrap();
    assert_eq!(released.fencing_token, first.fencing_token);

    let second = match store
        .acquire(
            "scheduler",
            "stable-daemon",
            instant(1),
            Duration::from_secs(10),
        )
        .unwrap()
    {
        DaemonLeaseAcquire::Acquired(lease) => lease,
        value => panic!("unexpected reacquisition result: {value:?}"),
    };
    assert_eq!(second.fencing_token, first.fencing_token + 1);
}
