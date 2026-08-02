use tempfile::TempDir;

use crate::storage::{Database, PushDeliveryAttemptInput, PushDeliveryAttemptStore};

fn create_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let db = Database::new(&db_path).expect("Failed to create database");
    (db, temp_dir)
}

#[test]
fn test_record_and_summarize_attempts() {
    let (db, _temp) = create_test_db();
    let store = PushDeliveryAttemptStore::new(&db);

    store
        .record_attempt(PushDeliveryAttemptInput {
            user_id: None,
            session_id: Some("session-1"),
            endpoint: "https://web.push.apple.com/test-endpoint",
            event_type: "completion",
            outcome: "success",
            http_status: Some(201),
            error_message: None,
            latency_ms: Some(30),
        })
        .expect("Failed to record success attempt");

    store
        .record_attempt(PushDeliveryAttemptInput {
            user_id: None,
            session_id: Some("session-1"),
            endpoint: "https://web.push.apple.com/test-endpoint",
            event_type: "completion",
            outcome: "failure",
            http_status: Some(500),
            error_message: Some("temporary failure"),
            latency_ms: Some(45),
        })
        .expect("Failed to record failure attempt");

    let latest = store
        .latest_for_user(None)
        .expect("Failed to fetch latest attempt")
        .expect("Expected latest attempt");
    assert_eq!(latest.outcome, "failure");
    assert_eq!(latest.provider_host, "web.push.apple.com");
    assert_eq!(latest.endpoint_hash.len(), 64);

    let summary = store
        .summary_for_user(None)
        .expect("Failed to summarize attempts");
    assert!(summary.last_attempt_at.is_some());
    assert!(summary.last_success_at.is_some());
    assert!(summary.last_failure_at.is_some());
    assert_eq!(
        summary.last_failure_reason.as_deref(),
        Some("temporary failure")
    );
    assert_eq!(summary.recent_failures_24h, 1);
}
