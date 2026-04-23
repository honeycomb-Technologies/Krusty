use super::{create_test_db, create_test_user};
use crate::storage::sessions::SessionManager;

#[test]
fn test_session_ownership_single_tenant_mode() {
    // In single-tenant mode (user_id = None), any session is accessible
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    // Create session without user (single-tenant mode)
    let session_id = manager
        .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    // Verify ownership - should succeed (no user check)
    let result = manager
        .verify_session_ownership(&session_id, None)
        .expect("Failed to verify ownership");

    assert!(
        result,
        "Single-tenant mode should allow access to any session"
    );
}

#[test]
fn test_session_ownership_multi_tenant_mode_success() {
    // In multi-tenant mode, users can only access their own sessions
    let (db, _temp) = create_test_db();

    let user_id = "user-123";

    // Create the user first (required for foreign key constraint)
    create_test_user(&db, user_id);

    let manager = SessionManager::new(db);

    // Create session with user ownership
    let session_id = manager
        .create_session_for_user(
            "Test Session",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user_id),
        )
        .expect("Failed to create session");

    // Verify ownership with correct user - should succeed
    let result = manager
        .verify_session_ownership(&session_id, Some(user_id))
        .expect("Failed to verify ownership");

    assert!(result, "User should have access to their own session");
}

#[test]
fn test_session_ownership_multi_tenant_mode_cross_user_denied() {
    // Users cannot access sessions belonging to other users
    let (db, _temp) = create_test_db();

    let user_id = "user-123";
    let other_user_id = "user-456";

    // Create the users first
    create_test_user(&db, user_id);
    create_test_user(&db, other_user_id);

    let manager = SessionManager::new(db);

    // Create session with user ownership
    let session_id = manager
        .create_session_for_user(
            "Test Session",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user_id),
        )
        .expect("Failed to create session");

    // Verify ownership with different user - should fail
    let result = manager
        .verify_session_ownership(&session_id, Some(other_user_id))
        .expect("Failed to verify ownership");

    assert!(
        !result,
        "User should NOT have access to another user's session"
    );
}

#[test]
fn test_session_ownership_nonexistent_session() {
    // Non-existent sessions should fail ownership verification
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let fake_session_id = uuid::Uuid::new_v4().to_string();

    // Single-tenant mode - nonexistent session
    let result_single = manager
        .verify_session_ownership(&fake_session_id, None)
        .expect("Failed to verify ownership");

    assert!(
        !result_single,
        "Non-existent session should not pass ownership check"
    );

    // Multi-tenant mode - nonexistent session
    let result_multi = manager
        .verify_session_ownership(&fake_session_id, Some("user-123"))
        .expect("Failed to verify ownership");

    assert!(
        !result_multi,
        "Non-existent session should not pass ownership check"
    );
}

#[test]
fn test_session_ownership_mixed_users_isolation() {
    // Multiple users should have complete isolation
    let (db, _temp) = create_test_db();

    let user1 = "alice";
    let user2 = "bob";

    // Create the users first
    create_test_user(&db, user1);
    create_test_user(&db, user2);

    let manager = SessionManager::new(db);

    // Create sessions for different users
    let session1 = manager
        .create_session_for_user(
            "Alice's Session",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user1),
        )
        .expect("Failed to create session for user1");

    let session2 = manager
        .create_session_for_user(
            "Bob's Session",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user2),
        )
        .expect("Failed to create session for user2");

    // User 1 can only access their own sessions
    let user1_access_1 = manager
        .verify_session_ownership(&session1, Some(user1))
        .expect("Failed to verify ownership");
    let user1_access_2 = manager
        .verify_session_ownership(&session2, Some(user1))
        .expect("Failed to verify ownership");

    assert!(user1_access_1, "Alice should access her own session");
    assert!(!user1_access_2, "Alice should NOT access Bob's session");

    // User 2 can only access their own sessions
    let user2_access_1 = manager
        .verify_session_ownership(&session1, Some(user2))
        .expect("Failed to verify ownership");
    let user2_access_2 = manager
        .verify_session_ownership(&session2, Some(user2))
        .expect("Failed to verify ownership");

    assert!(!user2_access_1, "Bob should NOT access Alice's session");
    assert!(user2_access_2, "Bob should access his own session");
}
