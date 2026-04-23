use super::{create_test_db, create_test_user};
use crate::storage::sessions::SessionManager;
use crate::storage::{SessionType, WorkspaceMode};

#[test]
fn test_list_sessions_for_user_filters_by_user_id() {
    // list_sessions_for_user should only return sessions owned by the user
    let (db, _temp) = create_test_db();

    let user1 = "alice";
    let user2 = "bob";

    // Create the users first
    create_test_user(&db, user1);
    create_test_user(&db, user2);

    let manager = SessionManager::new(db);

    // Create sessions for different users
    let _session1 = manager
        .create_session_for_user(
            "Alice's Session 1",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user1),
        )
        .expect("Failed to create session");
    let _session2 = manager
        .create_session_for_user(
            "Alice's Session 2",
            Some("claude-3-5-sonnet"),
            Some("/home"),
            Some(user1),
        )
        .expect("Failed to create session");
    let session3 = manager
        .create_session_for_user(
            "Bob's Session",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user2),
        )
        .expect("Failed to create session");

    // User 1 should see only their 2 sessions
    let user1_sessions = manager
        .list_sessions_for_user(None, Some(user1))
        .expect("Failed to list sessions");

    assert_eq!(
        user1_sessions.len(),
        2,
        "User 1 should see exactly 2 sessions"
    );

    // User 2 should see only their 1 session
    let user2_sessions = manager
        .list_sessions_for_user(None, Some(user2))
        .expect("Failed to list sessions");

    assert_eq!(
        user2_sessions.len(),
        1,
        "User 2 should see exactly 1 session"
    );
    assert_eq!(
        user2_sessions[0].id, session3,
        "User 2 should see their own session"
    );
}

#[test]
fn test_list_sessions_for_user_filters_by_working_dir_and_user() {
    // Combined filtering by working_dir AND user_id
    let (db, _temp) = create_test_db();

    let user = "alice";

    // Create the user first
    create_test_user(&db, user);

    let manager = SessionManager::new(db);

    // Create sessions in different directories
    let session1 = manager
        .create_session_for_user(
            "Session in /tmp",
            Some("claude-3-5-sonnet"),
            Some("/tmp"),
            Some(user),
        )
        .expect("Failed to create session");
    let _session2 = manager
        .create_session_for_user(
            "Session in /home",
            Some("claude-3-5-sonnet"),
            Some("/home"),
            Some(user),
        )
        .expect("Failed to create session");

    // Filter by both user and directory
    let tmp_sessions = manager
        .list_sessions_for_user(Some("/tmp"), Some(user))
        .expect("Failed to list sessions");

    assert_eq!(
        tmp_sessions.len(),
        1,
        "Should see exactly 1 session in /tmp"
    );
    assert_eq!(tmp_sessions[0].id, session1, "Should be the /tmp session");
}

#[test]
fn test_list_sessions_for_user_by_type_filters_surface() {
    let (db, _temp) = create_test_db();
    let user = "alice";
    create_test_user(&db, user);

    let manager = SessionManager::new(db);
    let mako_session = manager
        .create_session_for_user_with_config(
            "Mako Session",
            None,
            Some("/tmp"),
            Some("/tmp"),
            WorkspaceMode::Selected,
            Some(user),
            None,
            SessionType::Mako,
        )
        .expect("Failed to create Mako session");
    manager
        .create_session_for_user_with_config(
            "Code Session",
            None,
            Some("/tmp"),
            Some("/tmp"),
            WorkspaceMode::Selected,
            Some(user),
            None,
            SessionType::Code,
        )
        .expect("Failed to create Code session");

    let sessions = manager
        .list_sessions_for_user_by_type(Some("/tmp"), Some(user), SessionType::Mako)
        .expect("Failed to list Mako sessions");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, mako_session);
    assert_eq!(sessions[0].session_type, SessionType::Mako);
}

#[test]
fn test_list_active_session_details_for_user_filters_ownership() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    manager
        .db()
        .conn()
        .execute(
            "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
            rusqlite::params!["user-a", "user-a@example.com", "free"],
        )
        .expect("seed user a");
    manager
        .db()
        .conn()
        .execute(
            "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
            rusqlite::params!["user-b", "user-b@example.com", "free"],
        )
        .expect("seed user b");

    let session_a = manager
        .create_session_for_user("A", None, Some("/tmp/a"), Some("user-a"))
        .expect("session a should create");
    let session_b = manager
        .create_session_for_user("B", None, Some("/tmp/b"), Some("user-b"))
        .expect("session b should create");

    manager
        .set_agent_state(&session_a, "streaming")
        .expect("session a should become active");
    manager
        .set_agent_state(&session_b, "tool_executing")
        .expect("session b should become active");

    let active = manager
        .list_active_session_details_for_user(Some("user-a"))
        .expect("active sessions should load");

    assert_eq!(active.len(), 1);
    assert_eq!(active[0].0.id, session_a);
    assert_eq!(active[0].0.title, "A");
    assert_eq!(active[0].1.state, "streaming");
}
