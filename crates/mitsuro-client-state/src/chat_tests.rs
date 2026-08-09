use mitsuro_client::{
    PartialAssistantState, PermissionMode, RecoveryToolCall, SessionRecoveryState,
    SessionStateResponse, WorkMode,
};
use serde_json::json;

use crate::{pending_approval_from_state, ChatStore, MessageRole, ToolStatus, TranscriptNode};

#[test]
fn recovery_partial_restores_assistant_thinking_and_tools() {
    let mut store = ChatStore::default();
    let snapshot = SessionStateResponse {
        id: "s1".to_owned(),
        agent_state: "streaming".to_owned(),
        started_at: None,
        last_event_at: None,
        mode: WorkMode::Plan,
        permission_mode: PermissionMode::Supervised,
        recovery: Some(SessionRecoveryState {
            schema_version: 1,
            status: "recoverable".to_owned(),
            stop_reason: Some("disconnect".to_owned()),
            last_error: Some("socket closed".to_owned()),
            partial_assistant: PartialAssistantState {
                text: "partial answer".to_owned(),
                thinking: Some("working it out".to_owned()),
                tool_calls: vec![RecoveryToolCall {
                    id: "tool-1".to_owned(),
                    name: "read".to_owned(),
                    arguments: None,
                }],
            },
            pending_interactions: Vec::new(),
            decision: serde_json::Value::Null,
        }),
        pending_interactions: Vec::new(),
        live_partial_assistant: None,
        delegated_tools: Vec::new(),
        recent_delegated_runs: Vec::new(),
        delegated_run_summaries: Vec::new(),
        delegation_groups: Vec::new(),
        delegation_events: Vec::new(),
        delegation_event_cursor: None,
        last_event_sequence: Some(7),
    };

    store.apply_session_state_snapshot(&snapshot);

    assert!(store.state.is_streaming);
    assert_eq!(
        store.state.controls.permission_mode,
        PermissionMode::Supervised
    );
    assert_eq!(store.state.controls.work_mode, WorkMode::Plan);
    assert_eq!(store.state.last_error.as_deref(), Some("socket closed"));
    assert!(store.state.transcript.iter().any(|node| matches!(
        node,
        TranscriptNode::Message(message)
            if message.role == MessageRole::Assistant && message.content == "partial answer"
    )));
    assert!(store.state.transcript.iter().any(|node| matches!(
        node,
        TranscriptNode::Thinking(thinking) if thinking.content == "working it out"
    )));
    assert!(store.state.transcript.iter().any(|node| matches!(
        node,
        TranscriptNode::Tool(tool)
            if tool.id == "tool-1" && tool.name == "read" && tool.status == ToolStatus::Pending
    )));
}

#[test]
fn pending_interaction_recovery_skips_unknown_kinds() {
    let state: SessionStateResponse = serde_json::from_value(json!({
        "id": "s1",
        "agent_state": "awaiting_input",
        "pending_interactions": [
            { "kind": "future_pending_kind", "id": "ignored" },
            {
                "kind": "plan_confirm",
                "tool_call_id": "plan-1",
                "title": "Mobile plan",
                "task_count": 2
            }
        ]
    }))
    .expect("state should deserialize with unknown pending interaction");

    let approval = pending_approval_from_state(&state).expect("plan approval");
    assert_eq!(approval.tool_call_id, "plan-1");
    assert_eq!(approval.tool_name, "PlanConfirm");
}
