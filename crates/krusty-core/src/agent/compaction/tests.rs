use tempfile::TempDir;

use super::{
    cut_point::find_cut_point, microcompact::microcompact_messages, run_compaction_pipeline,
    CompactionManager, CompactionRequest, CompactionTrigger,
};
use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::{CompactionStore, Database, MessageStore, SessionManager};

fn text_message(role: Role, text: &str) -> ModelMessage {
    ModelMessage {
        role,
        content: vec![Content::Text {
            text: text.to_string(),
        }],
    }
}

fn create_persisted_conversation(
    temp: &TempDir,
    name: &str,
) -> (std::path::PathBuf, String, Vec<ModelMessage>) {
    let db_path = temp.path().join(format!("{name}.db"));
    let db = Database::new(&db_path).expect("db");
    let session_manager = SessionManager::new(db);
    let session_id = session_manager
        .create_session("Compaction test", None, None)
        .expect("session");

    let conversation = vec![
        text_message(Role::User, "start task"),
        text_message(Role::Assistant, "working"),
        text_message(Role::User, "continue"),
        text_message(Role::Assistant, "recent work"),
    ];

    for message in &conversation {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            _ => continue,
        };
        let content = serde_json::to_string(&message.content).expect("content json");
        session_manager
            .save_message(&session_id, role, &content)
            .expect("save");
    }

    (db_path, session_id, conversation)
}

#[tokio::test]
async fn run_compaction_pipeline_replaces_history_in_place() {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("compaction.db");
    let db = Database::new(&db_path).expect("db");
    let session_manager = SessionManager::new(db);
    let session_id = session_manager
        .create_session("Compaction test", None, None)
        .expect("session");

    for (role, text) in [
        ("user", "start task"),
        ("assistant", "working"),
        ("user", "continue"),
        ("assistant", "recent work"),
    ] {
        let content = serde_json::json!([{ "type": "text", "text": text }]).to_string();
        session_manager
            .save_message(&session_id, role, &content)
            .expect("save");
    }

    let conversation = vec![
        text_message(Role::User, "start task"),
        text_message(Role::Assistant, "working"),
        text_message(Role::User, "continue"),
        text_message(Role::Assistant, "recent work"),
    ];

    let result = run_compaction_pipeline(CompactionRequest {
        db_path: &db_path,
        session_id: &session_id,
        conversation: &conversation,
        working_dir: temp.path(),
        ai_client: None,
        model: None,
        trigger: CompactionTrigger::Manual {
            preservation_hints: None,
            direction: Some("Keep going".to_string()),
        },
        compaction_manager: CompactionManager::for_model(
            crate::ai::providers::ProviderId::MiniMax,
            crate::ai::models::ApiFormat::Anthropic,
            crate::constants::ai::DEFAULT_MODEL,
            200_000,
        ),
        triggering_token_estimate: None,
        last_usage_prompt_tokens: None,
        messages_after_usage: 0,
        project_dir: None,
        user_id: None,
        summary_override: Some(crate::agent::SummarizationResult {
            work_summary: "Did important work".to_string(),
            key_decisions: vec!["Use SQLite".to_string()],
            pending_tasks: vec!["Finish compaction".to_string()],
            important_files: vec!["src/main.rs".to_string()],
        }),
    })
    .await
    .expect("compact");

    assert!(result.replaced_messages > 0);
    assert!(result.compacted_conversation.len() < conversation.len() + 2);
    assert!(result
        .compacted_conversation
        .iter()
        .any(|message| message.content.iter().any(|content| {
            matches!(content, Content::Text { text } if text.contains("Conversation Compacted"))
        })));

    let db = Database::new(&db_path).expect("db");
    let records = MessageStore::new(&db)
        .load_session_message_records(&session_id)
        .expect("records");
    assert_eq!(records.len(), result.compacted_conversation.len());

    let checkpoints = CompactionStore::new(&db)
        .count_checkpoints(&session_id)
        .expect("checkpoint count");
    assert_eq!(checkpoints, 1);
    let compacted_history_json: String = db
        .conn()
        .query_row(
            "SELECT compacted_history_json FROM compaction_checkpoints WHERE session_id = ?1",
            [&session_id],
            |row| row.get(0),
        )
        .expect("checkpoint history");
    assert_eq!(compacted_history_json, "[]");
}

#[tokio::test]
async fn auto_compaction_uses_caller_trigger_estimate() {
    let temp = TempDir::new().expect("temp dir");
    let (db_path, session_id, conversation) = create_persisted_conversation(&temp, "auto-trigger");

    let result = run_compaction_pipeline(CompactionRequest {
        db_path: &db_path,
        session_id: &session_id,
        conversation: &conversation,
        working_dir: temp.path(),
        ai_client: None,
        model: None,
        trigger: CompactionTrigger::Auto,
        compaction_manager: CompactionManager::for_model(
            crate::ai::providers::ProviderId::MiniMax,
            crate::ai::models::ApiFormat::Anthropic,
            crate::constants::ai::DEFAULT_MODEL,
            2_000,
        ),
        triggering_token_estimate: Some(2_000),
        last_usage_prompt_tokens: None,
        messages_after_usage: 0,
        project_dir: None,
        user_id: None,
        summary_override: Some(crate::agent::SummarizationResult {
            work_summary: "Triggered by injected context".to_string(),
            key_decisions: Vec::new(),
            pending_tasks: Vec::new(),
            important_files: Vec::new(),
        }),
    })
    .await
    .expect("auto compaction should use caller estimate");

    assert_eq!(result.estimated_tokens_before, 2_000);
    assert!(result.replaced_messages > 0);
}

#[tokio::test]
async fn compaction_without_ai_uses_deterministic_summary() {
    let temp = TempDir::new().expect("temp dir");
    let (db_path, session_id, conversation) = create_persisted_conversation(&temp, "fallback");

    let result = run_compaction_pipeline(CompactionRequest {
        db_path: &db_path,
        session_id: &session_id,
        conversation: &conversation,
        working_dir: temp.path(),
        ai_client: None,
        model: None,
        trigger: CompactionTrigger::Manual {
            preservation_hints: Some("Preserve the continuation objective".to_string()),
            direction: None,
        },
        compaction_manager: CompactionManager::default(),
        triggering_token_estimate: None,
        last_usage_prompt_tokens: None,
        messages_after_usage: 0,
        project_dir: None,
        user_id: None,
        summary_override: None,
    })
    .await
    .expect("manual compaction should have deterministic fallback");

    assert!(result
        .summary
        .work_summary
        .contains("deterministic compaction summary"));
    assert_ne!(result.summary.work_summary, "No summary available.");
}

#[test]
fn find_cut_point_keeps_tail_messages() {
    let messages = vec![
        super::cut_point::IndexedMessage {
            id: 1,
            message: text_message(Role::User, "old"),
        },
        super::cut_point::IndexedMessage {
            id: 2,
            message: text_message(Role::Assistant, "old reply"),
        },
        super::cut_point::IndexedMessage {
            id: 3,
            message: text_message(Role::User, "recent"),
        },
        super::cut_point::IndexedMessage {
            id: 4,
            message: text_message(Role::Assistant, "latest"),
        },
    ];

    let cut = find_cut_point(&messages, 0, 1).expect("cut");
    assert_eq!(cut.first_kept_message_id, 4);
    assert_eq!(cut.kept_messages.len(), 1);
}

#[test]
fn microcompact_strips_old_thinking_blocks() {
    let mut conversation = Vec::new();
    for index in 0..8 {
        conversation.push(text_message(Role::User, &format!("question {index}")));
        conversation.push(ModelMessage {
            role: Role::Assistant,
            content: vec![
                Content::Thinking {
                    thinking: format!("reasoning {index}"),
                    signature: String::new(),
                },
                Content::Text {
                    text: format!("answer {index}"),
                },
            ],
        });
    }

    let result = microcompact_messages(&conversation);
    assert!(result.changed);
    let first_assistant = &result.messages[1];
    assert!(!first_assistant
        .content
        .iter()
        .any(|content| matches!(content, Content::Thinking { .. })));
}

#[test]
fn microcompact_truncates_unicode_tool_results_safely() {
    let mut conversation = Vec::new();
    conversation.push(ModelMessage {
        role: Role::User,
        content: vec![Content::ToolResult {
            tool_use_id: "tool-1".to_string(),
            output: serde_json::json!({
                "retention": "summarize_after_turn",
                "summary": "unicode output",
                "result": "🦀".repeat(600),
            }),
            is_error: None,
        }],
    });
    for index in 0..7 {
        conversation.push(text_message(Role::Assistant, &format!("assistant {index}")));
    }

    let result = microcompact_messages(&conversation);
    assert!(result.changed);
    let Content::ToolResult { output, .. } = &result.messages[0].content[0] else {
        panic!("expected tool result");
    };
    let truncated = output
        .get("result")
        .and_then(|value| value.as_str())
        .expect("truncated result");
    assert!(truncated.contains("[microcompact truncated]"));
}
