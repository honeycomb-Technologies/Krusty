use mitsuro_core::{
    agent::{loop_events::LoopStopReason, LoopEvent},
    ai::types::{Content, ModelMessage, Role},
};
use serde_json::json;

use crate::tui_v2::model::{
    artifact::ArtifactContent,
    conversation::{PendingInteraction, TimelinePart, ToolStatus, TurnState},
};

use super::{tool_output::LIVE_ARTIFACT_BYTES, ConversationProjection, PersistedMessage};

#[test]
fn live_and_persisted_text_tool_text_have_identical_turns() {
    let mut live = ConversationProjection::new("session");
    live.push_user_prompt("u1", "Inspect this.".to_owned(), Vec::new(), false);
    for event in [
        LoopEvent::TextDelta {
            delta: "I’ll inspect the current state.".to_owned(),
        },
        LoopEvent::ToolCallStart {
            id: "read-1".to_owned(),
            name: "read".to_owned(),
        },
        LoopEvent::ToolCallComplete {
            id: "read-1".to_owned(),
            name: "read".to_owned(),
            arguments: json!({"path": "src/main.rs"}),
        },
        LoopEvent::ToolExecuting {
            id: "read-1".to_owned(),
            name: "read".to_owned(),
        },
        LoopEvent::ToolResult {
            id: "read-1".to_owned(),
            output: json!({"ok": true, "data": "42"}).to_string(),
            is_error: false,
        },
        LoopEvent::TextDelta {
            delta: "The state is stored in parallel vectors.".to_owned(),
        },
        LoopEvent::TurnComplete {
            turn: 1,
            has_more: false,
        },
        LoopEvent::Finished {
            session_id: "session".to_owned(),
            stop_reason: LoopStopReason::Completed,
        },
    ] {
        live.apply_event(event);
    }

    let persisted = ConversationProjection::from_persisted(
        "session",
        &[
            PersistedMessage::new(
                "u1",
                ModelMessage {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: "Inspect this.".to_owned(),
                    }],
                },
            ),
            PersistedMessage::without_id(ModelMessage {
                role: Role::Assistant,
                content: vec![
                    Content::Text {
                        text: "I’ll inspect the current state.".to_owned(),
                    },
                    Content::ToolUse {
                        id: "read-1".to_owned(),
                        name: "read".to_owned(),
                        input: json!({"path": "src/main.rs"}),
                    },
                ],
            }),
            PersistedMessage::without_id(ModelMessage {
                role: Role::User,
                content: vec![Content::ToolResult {
                    tool_use_id: "read-1".to_owned(),
                    output: json!({"ok": true, "data": "42"}),
                    is_error: Some(false),
                }],
            }),
            PersistedMessage::without_id(ModelMessage {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "The state is stored in parallel vectors.".to_owned(),
                }],
            }),
        ],
    );

    assert_eq!(live.presentation().turns, persisted.presentation().turns);
    let parts = &live.presentation().turns[0].parts;
    assert!(matches!(
        parts.as_slice(),
        [
            TimelinePart::AgentText(_),
            TimelinePart::Tool(_),
            TimelinePart::AgentText(_)
        ]
    ));
}

#[test]
fn a_thousand_bash_deltas_update_one_bounded_artifact() {
    let mut projection = ConversationProjection::new("session");
    projection.push_user_prompt("u1", "Run tests.".to_owned(), Vec::new(), false);
    projection.apply_event(LoopEvent::ToolCallComplete {
        id: "bash-1".to_owned(),
        name: "bash".to_owned(),
        arguments: json!({"command": "cargo test"}),
    });
    projection.apply_event(LoopEvent::ToolExecuting {
        id: "bash-1".to_owned(),
        name: "bash".to_owned(),
    });
    for index in 0..1_000 {
        projection.apply_event(LoopEvent::ToolOutputDelta {
            id: "bash-1".to_owned(),
            delta: format!("\rcompiling crate {index}: {}\n", "x".repeat(160)),
        });
    }
    let TimelinePart::Tool(running_tool) = &projection.presentation().turns[0].parts[0] else {
        panic!("tool");
    };
    assert_eq!(running_tool.status, ToolStatus::Running);
    assert!(matches!(
        &running_tool.artifact.content,
        ArtifactContent::Text(text)
            if text.text.len() <= LIVE_ARTIFACT_BYTES && text.omitted_bytes > 0
    ));

    projection.apply_event(LoopEvent::ToolResult {
        id: "bash-1".to_owned(),
        output: "finished".to_owned(),
        is_error: false,
    });

    let tool_parts = projection.presentation().turns[0]
        .parts
        .iter()
        .filter_map(|part| match part {
            TimelinePart::Tool(tool) => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_parts.len(), 1);
    assert_eq!(tool_parts[0].status, ToolStatus::Succeeded);
    assert!(matches!(
        &tool_parts[0].artifact.content,
        ArtifactContent::Text(text)
            if text.text.contains("compiling crate") && text.text.contains("finished")
    ));
}

#[test]
fn tool_events_do_not_split_a_streaming_word_across_agent_parts() {
    let mut projection = ConversationProjection::new("session");
    projection.push_user_prompt("u1", "Inspect this.".to_owned(), Vec::new(), false);
    projection.apply_event(LoopEvent::TextDelta {
        delta: "Checking Foo/B".to_owned(),
    });
    projection.apply_event(LoopEvent::ToolCallStart {
        id: "read-1".to_owned(),
        name: "read".to_owned(),
    });
    projection.apply_event(LoopEvent::TextDelta {
        delta: "ar next.".to_owned(),
    });

    let parts = &projection.presentation().turns[0].parts;
    assert_eq!(parts.len(), 2);
    assert!(matches!(
        &parts[0],
        TimelinePart::AgentText(part) if part.text == "Checking Foo/Bar next."
    ));
    assert!(matches!(&parts[1], TimelinePart::Tool(_)));
}

#[test]
fn usage_title_and_mode_update_metadata_without_transcript_noise() {
    let mut projection = ConversationProjection::new("session");
    projection.push_user_prompt("u1", "Hello".to_owned(), Vec::new(), false);
    projection.apply_event(LoopEvent::TitleGenerated {
        title: "Clean TUI".to_owned(),
    });
    projection.apply_event(LoopEvent::ModeChange {
        mode: "build".to_owned(),
        reason: None,
    });
    projection.apply_event(LoopEvent::Usage {
        prompt_tokens: 10,
        input_tokens: 30,
        completion_tokens: 5,
        reasoning_tokens: 2,
        cache_creation_input_tokens: 10,
        cache_read_input_tokens: 10,
        total_tokens: 35,
    });

    assert!(projection.presentation().turns[0].parts.is_empty());
    assert_eq!(
        projection.presentation().metadata.title.as_deref(),
        Some("Clean TUI")
    );
    assert_eq!(
        projection.presentation().metadata.mode.as_deref(),
        Some("build")
    );
    assert_eq!(
        projection
            .presentation()
            .metadata
            .usage
            .as_ref()
            .map(|usage| usage.total_tokens),
        Some(35)
    );
}

#[test]
fn canonical_steering_ack_deduplicates_an_optimistic_turn() {
    let mut projection = ConversationProjection::new("session");
    projection.apply_event(LoopEvent::SteeringInjected {
        pending_id: Some("pending-1".to_owned()),
        message: "Keep the layout compact.".to_owned(),
    });
    projection.apply_event(LoopEvent::SteeringInjected {
        pending_id: Some("pending-1".to_owned()),
        message: "Keep the layout compact.".to_owned(),
    });

    assert_eq!(projection.presentation().turns.len(), 1);
    assert!(projection.presentation().turns[0]
        .user
        .as_ref()
        .is_some_and(|user| user.steering));
}

#[test]
fn in_place_compaction_and_linked_continuation_stay_distinct() {
    let mut projection = ConversationProjection::new("session");
    projection.push_user_prompt("u1", "Continue".to_owned(), Vec::new(), false);
    projection.apply_event(LoopEvent::ContextCompactionStarted {
        reason: "threshold".to_owned(),
    });
    projection.apply_event(LoopEvent::ContextCompacted {
        reason: "threshold".to_owned(),
        estimated_tokens_before: 100_000,
        estimated_tokens_after: 40_000,
        replaced_messages: 20,
        checkpoint_id: "cp-1".to_owned(),
        compaction_count: 1,
    });
    projection.apply_event(LoopEvent::SessionPinched {
        reason: "explicit continuation".to_owned(),
        source_session_id: "session".to_owned(),
        new_session_id: "session-2".to_owned(),
        estimated_tokens_before: 50_000,
    });

    assert!(matches!(
        projection.presentation().turns[0].parts[0],
        TimelinePart::Compaction(ref part) if part.in_place
    ));
    assert!(matches!(
        projection.presentation().turns[0].parts[1],
        TimelinePart::Notice(ref part) if part.message.contains("session-2")
    ));
}

#[test]
fn approvals_and_questions_target_exact_tool_and_session_once() {
    let mut projection = ConversationProjection::new("session-42");
    projection.push_user_prompt("u1", "Proceed carefully.".to_owned(), Vec::new(), false);

    for _ in 0..2 {
        projection.apply_event(LoopEvent::ToolApprovalRequired {
            id: "bash-1".to_owned(),
            name: "bash".to_owned(),
            arguments: json!({"command": "git push", "token": "secret"}),
        });
    }
    assert_eq!(projection.presentation().pending_interactions.len(), 1);
    assert!(matches!(
        &projection.presentation().pending_interactions[0],
        PendingInteraction::ToolApproval(approval)
            if approval.session_id == "session-42"
                && approval.tool_call_id == "bash-1"
                && approval.arguments.redacted_fields == 1
    ));
    projection.apply_event(LoopEvent::ToolDenied {
        id: "bash-1".to_owned(),
    });
    assert!(projection.presentation().pending_interactions.is_empty());

    projection.apply_event(LoopEvent::ToolCallComplete {
        id: "question-1".to_owned(),
        name: "AskUserQuestion".to_owned(),
        arguments: json!({
            "questions": [{
                "header": "Style",
                "question": "Which density?",
                "options": [{"label": "Compact", "description": "One-line tools"}],
                "multi_select": false
            }]
        }),
    });
    projection.apply_event(LoopEvent::AwaitingInput {
        tool_call_id: "question-1".to_owned(),
        tool_name: "AskUserQuestion".to_owned(),
    });
    assert!(matches!(
        &projection.presentation().pending_interactions[0],
        PendingInteraction::Questions(questions)
            if questions.session_id == "session-42"
                && questions.tool_call_id == "question-1"
                && questions.questions[0].question == "Which density?"
    ));
    projection.apply_event(LoopEvent::ToolResult {
        id: "question-1".to_owned(),
        output: "Compact".to_owned(),
        is_error: false,
    });
    assert!(projection.presentation().pending_interactions.is_empty());
}

#[test]
fn server_tool_events_update_one_ordered_part() {
    let mut projection = ConversationProjection::new("session");
    projection.push_user_prompt("u1", "Research Ratatui.".to_owned(), Vec::new(), false);
    projection.apply_event(LoopEvent::ServerToolStart {
        id: "web-1".to_owned(),
        name: "web_search".to_owned(),
    });
    projection.apply_event(LoopEvent::WebSearchResults {
        tool_use_id: "web-1".to_owned(),
        results: vec![mitsuro_core::ai::types::WebSearchResult {
            url: "https://ratatui.rs".to_owned(),
            title: "Ratatui".to_owned(),
            encrypted_content: None,
            page_age: Some("recent".to_owned()),
        }],
    });
    projection.apply_event(LoopEvent::ServerToolComplete {
        id: "web-1".to_owned(),
        name: "web_search".to_owned(),
    });

    assert_eq!(projection.presentation().turns[0].parts.len(), 1);
    assert!(matches!(
        &projection.presentation().turns[0].parts[0],
        TimelinePart::Tool(tool)
            if tool.server_side
                && tool.status == ToolStatus::Succeeded
                && matches!(tool.artifact.content, ArtifactContent::WebResults(ref results) if results.len() == 1)
    ));
}

#[test]
fn interrupt_settles_every_live_visual_state_without_dropping_evidence() {
    let mut projection = ConversationProjection::new("session");
    projection.push_user_prompt("u1", "Run it.".to_owned(), Vec::new(), false);
    projection.apply_event(LoopEvent::ThinkingDelta {
        thinking: "Checking".to_owned(),
    });
    projection.apply_event(LoopEvent::ToolExecuting {
        id: "bash-1".to_owned(),
        name: "bash".to_owned(),
    });
    projection.apply_event(LoopEvent::ToolOutputDelta {
        id: "bash-1".to_owned(),
        delta: "partial evidence".to_owned(),
    });
    projection.apply_event(LoopEvent::Finished {
        session_id: "session".to_owned(),
        stop_reason: LoopStopReason::UserAbort,
    });

    assert_eq!(
        projection.presentation().turns[0].state,
        TurnState::Interrupted
    );
    assert!(projection.presentation().live_turn_id.is_none());
    assert!(matches!(
        &projection.presentation().turns[0].parts[0],
        TimelinePart::Thinking(thinking) if !thinking.streaming
    ));
    assert!(matches!(
        &projection.presentation().turns[0].parts[1],
        TimelinePart::Tool(tool)
            if tool.status == ToolStatus::Interrupted
                && matches!(tool.artifact.content, ArtifactContent::Text(ref text) if text.text.contains("partial evidence"))
    ));
}

#[test]
fn canonical_steering_event_opens_a_new_user_turn() {
    let mut projection = ConversationProjection::new("session");
    projection.push_user_prompt("u1", "Start here.".to_owned(), Vec::new(), false);
    projection.apply_event(LoopEvent::TextDelta {
        delta: "Working".to_owned(),
    });
    projection.apply_event(LoopEvent::SteeringInjected {
        pending_id: Some("steer-1".to_owned()),
        message: "Also update the tests.".to_owned(),
    });

    assert_eq!(projection.presentation().turns.len(), 2);
    let steering = projection.presentation().turns[1]
        .user
        .as_ref()
        .expect("canonical steering prompt");
    assert!(steering.steering);
    assert_eq!(steering.text, "Also update the tests.");
}
