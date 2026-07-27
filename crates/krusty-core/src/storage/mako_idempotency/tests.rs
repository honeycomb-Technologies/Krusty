use std::time::Duration;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use crate::storage::Database;

use super::{hash_request_bytes, IdempotencyClaim, MakoIdempotencyStore};

fn store() -> (MakoIdempotencyStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("idempotency.db")).unwrap();
    (MakoIdempotencyStore::new(db), temp)
}

fn instant(second: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, second)
        .single()
        .unwrap()
}

#[test]
fn equivalent_request_is_in_progress_then_replayed() {
    let (store, _temp) = store();
    let request_hash = hash_request_bytes(br#"{"objective":"audit"}"#);
    assert!(matches!(
        store
            .claim(
                "user:alice",
                "dispatch",
                "request-1",
                &request_hash,
                instant(0),
                Duration::from_secs(30),
            )
            .unwrap(),
        IdempotencyClaim::Claimed(_)
    ));
    assert!(matches!(
        store
            .claim(
                "user:alice",
                "dispatch",
                "request-1",
                &request_hash,
                instant(1),
                Duration::from_secs(30),
            )
            .unwrap(),
        IdempotencyClaim::InProgress(_)
    ));
    assert!(store
        .complete(
            "user:alice",
            "dispatch",
            "request-1",
            &request_hash,
            Some("run-1"),
            &serde_json::json!({"run_id": "run-1"}),
            instant(2),
        )
        .unwrap());
    match store
        .claim(
            "user:alice",
            "dispatch",
            "request-1",
            &request_hash,
            instant(3),
            Duration::from_secs(30),
        )
        .unwrap()
    {
        IdempotencyClaim::Replay(record) => {
            assert_eq!(record.resource_id.as_deref(), Some("run-1"));
            assert_eq!(record.response.unwrap()["run_id"], "run-1");
        }
        disposition => panic!("expected replay, got {disposition:?}"),
    }
}

#[test]
fn key_reuse_with_a_different_payload_is_a_conflict() {
    let (store, _temp) = store();
    store
        .claim(
            "user:alice",
            "dispatch",
            "request-1",
            "hash-a",
            instant(0),
            Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(
        store
            .claim(
                "user:alice",
                "dispatch",
                "request-1",
                "hash-b",
                instant(1),
                Duration::from_secs(30),
            )
            .unwrap(),
        IdempotencyClaim::Conflict {
            existing_request_hash: "hash-a".into()
        }
    );
}

#[test]
fn uncompleted_claim_can_be_released_after_dispatch_rejection() {
    let (store, _temp) = store();
    store
        .claim(
            "session:one",
            "tool_approval",
            "approval-1",
            "hash-a",
            instant(0),
            Duration::from_secs(30),
        )
        .unwrap();
    assert!(store
        .release("session:one", "tool_approval", "approval-1", "hash-a",)
        .unwrap());
    assert!(matches!(
        store
            .claim(
                "session:one",
                "tool_approval",
                "approval-1",
                "hash-a",
                instant(1),
                Duration::from_secs(30),
            )
            .unwrap(),
        IdempotencyClaim::Claimed(_)
    ));
}

#[test]
fn expired_claim_can_be_reacquired_and_stale_owner_cannot_complete() {
    let (store, _temp) = store();
    store
        .claim(
            "user:alice",
            "dispatch",
            "request-1",
            "hash-a",
            instant(0),
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(matches!(
        store
            .claim(
                "user:alice",
                "dispatch",
                "request-1",
                "hash-b",
                instant(6),
                Duration::from_secs(5),
            )
            .unwrap(),
        IdempotencyClaim::Claimed(_)
    ));
    assert!(!store
        .complete(
            "user:alice",
            "dispatch",
            "request-1",
            "hash-a",
            None,
            &serde_json::json!({}),
            instant(7),
        )
        .unwrap());
    assert!(store
        .complete(
            "user:alice",
            "dispatch",
            "request-1",
            "hash-b",
            None,
            &serde_json::json!({"ok": true}),
            instant(7),
        )
        .unwrap());
}
