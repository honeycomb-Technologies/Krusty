use super::create_test_db;
use crate::storage::sessions::SessionManager;

#[test]
fn test_agent_state_management() {
    // Test agent state tracking and updates
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    // Initially idle
    let state = manager.get_agent_state(&session_id);
    assert!(state.is_some(), "Should have agent state");
    assert_eq!(state.unwrap().state, "idle", "Initial state should be idle");

    // Update to streaming
    manager
        .set_agent_state(&session_id, "streaming")
        .expect("Failed to update agent state");

    let state = manager.get_agent_state(&session_id).unwrap();
    assert_eq!(state.state, "streaming");
    assert!(state.started_at.is_some(), "Should have started_at");
    assert!(state.last_event_at.is_some(), "Should have last_event_at");

    // Update back to idle
    manager
        .set_agent_state(&session_id, "idle")
        .expect("Failed to update agent state");

    let state = manager.get_agent_state(&session_id).unwrap();
    assert_eq!(state.state, "idle");
    assert!(state.started_at.is_none(), "Idle should clear started_at");
}

#[test]
fn test_list_active_sessions() {
    // Test filtering sessions by agent activity
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session1 = manager
        .create_session("Active 1", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");
    let session2 = manager
        .create_session("Active 2", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");
    let _session3 = manager
        .create_session("Idle", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    // Set two sessions to active states
    manager
        .set_agent_state(&session1, "streaming")
        .expect("Failed to update state");
    manager
        .set_agent_state(&session2, "tool_executing")
        .expect("Failed to update state");

    // List active sessions
    let active = manager
        .list_active_sessions()
        .expect("Failed to list active sessions");

    assert_eq!(active.len(), 2, "Should have 2 active sessions");

    let active_ids: Vec<&str> = active.iter().map(|(id, _)| id.as_str()).collect();
    assert!(active_ids.contains(&session1.as_str()));
    assert!(active_ids.contains(&session2.as_str()));
}

#[test]
fn test_touch_agent_event() {
    // Test updating agent activity timestamp
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);

    let session_id = manager
        .create_session("Test Session", Some("claude-3-5-sonnet"), Some("/tmp"))
        .expect("Failed to create session");

    manager
        .set_agent_state(&session_id, "streaming")
        .expect("Failed to set streaming");

    // Touch the event
    manager
        .touch_agent_event(&session_id)
        .expect("Failed to touch event");

    let state = manager.get_agent_state(&session_id).unwrap();
    assert!(state.last_event_at.is_some(), "Should have last_event_at");
}

#[test]
fn test_context_continuation_state_round_trip() {
    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);
    let session_id = manager
        .create_session("Test Session", Some("gpt-5"), Some("/tmp"))
        .expect("Failed to create session");

    manager
        .update_context_continuation_state(
            &session_id,
            r#"{"schema_version":1,"canonical_messages":2}"#,
            r#"{"schema_version":1,"decision":{"kind":"resumable","latest_user_objective":"ship fix"}}"#,
        )
        .expect("Failed to persist context continuation state");

    let loaded = manager
        .load_context_continuation_state(&session_id)
        .expect("Failed to load context continuation state");

    let (ledger, continuation) = loaded.expect("Expected persisted state");
    assert!(ledger.contains("\"schema_version\":1"));
    assert!(continuation.contains("\"kind\":\"resumable\""));
}

#[test]
fn test_recovery_state_round_trip() {
    use crate::agent::loop_events::LoopStopReason;
    use crate::storage::{
        PartialAssistantState, RecoveryDecision, RecoveryStatus, RecoveryToolCall,
        SessionRecoveryState,
    };

    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);
    let session_id = manager
        .create_session("Interrupted Session", Some("gpt-5"), Some("/tmp"))
        .expect("Failed to create session");

    let recovery = SessionRecoveryState::new(
        RecoveryStatus::Interrupted,
        Some(LoopStopReason::StreamIdleTimeout),
        Some("stream timeout".to_string()),
        PartialAssistantState {
            text: "partial answer".to_string(),
            thinking: String::new(),
            tool_calls: vec![RecoveryToolCall::summary("call-1", "read")],
        },
        RecoveryDecision::NonResumable {
            reason: crate::storage::RecoveryNonResumableReason::PendingToolCall,
        },
    );

    manager
        .update_recovery_state(&session_id, &recovery)
        .expect("Failed to persist recovery state");

    let loaded = manager
        .load_recovery_state(&session_id)
        .expect("Failed to load recovery state")
        .expect("Expected persisted recovery state");

    assert_eq!(loaded, recovery);

    manager
        .clear_recovery_state(&session_id)
        .expect("Failed to clear recovery state");
    assert!(
        manager
            .load_recovery_state(&session_id)
            .expect("Failed to reload recovery state")
            .is_none(),
        "Expected recovery state to be cleared"
    );
}

#[test]
fn test_clear_stale_transient_recovery_states_preserves_pending_interactions() {
    use crate::agent::loop_events::LoopStopReason;
    use crate::storage::{
        PartialAssistantState, PendingInteractionSnapshot, RecoveryDecision,
        RecoveryNonResumableReason, RecoveryStatus, SessionRecoveryState,
    };
    use serde_json::json;

    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);
    let stale_session = manager
        .create_session("Stale", Some("gpt-5"), Some("/tmp"))
        .expect("Failed to create stale session");
    let pending_session = manager
        .create_session("Pending", Some("gpt-5"), Some("/tmp"))
        .expect("Failed to create pending session");

    manager
        .update_recovery_state(
            &stale_session,
            &SessionRecoveryState::new(
                RecoveryStatus::Interrupted,
                Some(LoopStopReason::ProviderError),
                None,
                PartialAssistantState::default(),
                RecoveryDecision::NonResumable {
                    reason: RecoveryNonResumableReason::EmptyConversation,
                },
            ),
        )
        .expect("Failed to persist stale recovery");
    manager
        .update_recovery_state(
            &pending_session,
            &SessionRecoveryState::new_with_pending_interactions(
                RecoveryStatus::AwaitingInput,
                Some(LoopStopReason::AwaitingInput),
                None,
                PartialAssistantState::default(),
                vec![PendingInteractionSnapshot::tool_approval_from_call(
                    "call-edit",
                    "edit",
                    &json!({"file_path": "src/lib.rs"}),
                )],
                RecoveryDecision::NonResumable {
                    reason: RecoveryNonResumableReason::AwaitingHumanInput,
                },
            ),
        )
        .expect("Failed to persist pending recovery");

    let cleared = manager
        .clear_stale_transient_recovery_states()
        .expect("Failed to clear stale recovery states");

    assert_eq!(cleared, 1);
    assert!(manager
        .load_recovery_state(&stale_session)
        .expect("Failed to load stale session")
        .is_none());
    assert!(manager
        .load_recovery_state(&pending_session)
        .expect("Failed to load pending session")
        .is_some());
}

#[test]
fn startup_repair_preserves_daemon_owned_mako_state() {
    use crate::agent::loop_events::LoopStopReason;
    use crate::storage::{
        PartialAssistantState, RecoveryDecision, RecoveryNonResumableReason, RecoveryStatus,
        SessionRecoveryState, SessionType, WorkspaceMode,
    };

    let (db, _temp) = create_test_db();
    let manager = SessionManager::new(db);
    let code_session = manager
        .create_session("Code", Some("gpt-5"), Some("/tmp"))
        .expect("create code session");
    let chat_session = manager
        .create_session_for_user_with_config(
            "Chat",
            Some("gpt-5"),
            Some("/tmp"),
            Some("/tmp"),
            WorkspaceMode::Selected,
            None,
            None,
            SessionType::Chat,
        )
        .expect("create chat session");
    let mako_session = manager
        .create_session_for_user_with_config(
            "Mako",
            Some("gpt-5"),
            Some("/tmp"),
            Some("/tmp"),
            WorkspaceMode::Selected,
            None,
            None,
            SessionType::Mako,
        )
        .expect("create Mako session");

    let stale_recovery = SessionRecoveryState::new(
        RecoveryStatus::Interrupted,
        Some(LoopStopReason::ProviderError),
        None,
        PartialAssistantState::default(),
        RecoveryDecision::NonResumable {
            reason: RecoveryNonResumableReason::EmptyConversation,
        },
    );
    for session_id in [&code_session, &chat_session, &mako_session] {
        manager
            .set_agent_state(session_id, "streaming")
            .expect("set active state");
        manager
            .update_recovery_state(session_id, &stale_recovery)
            .expect("persist stale recovery");
    }

    assert_eq!(
        manager
            .reset_transient_agent_states()
            .expect("repair transient states"),
        2
    );
    assert_eq!(
        manager
            .clear_stale_transient_recovery_states()
            .expect("clear stale recovery"),
        2
    );

    for session_id in [&code_session, &chat_session] {
        assert_eq!(manager.get_agent_state(session_id).unwrap().state, "idle");
        assert!(manager
            .load_recovery_state(session_id)
            .expect("load repaired recovery")
            .is_none());
    }
    assert_eq!(
        manager.get_agent_state(&mako_session).unwrap().state,
        "streaming"
    );
    assert_eq!(
        manager
            .load_recovery_state(&mako_session)
            .expect("load Mako recovery"),
        Some(stale_recovery)
    );
}
