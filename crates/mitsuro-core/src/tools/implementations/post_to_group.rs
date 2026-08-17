use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::hive_groups::CappedGroupAppend;
use crate::storage::{
    Database, HiveGroupSenderKind, HiveGroupStore, NewHiveGroupMessage,
    MAX_HIVE_GROUP_MESSAGE_BYTES,
};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

/// The only way a group member run can speak in its room. Ordinary assistant
/// output and tool use stay private to the run; this tool appends a durable
/// worker message to the group timeline, capped per run by the turn's frozen
/// policy.
pub struct PostToGroupTool;

#[derive(Deserialize)]
struct Params {
    message: String,
    #[serde(default)]
    reply_to_message_id: Option<String>,
}

#[async_trait]
impl Tool for PostToGroupTool {
    fn name(&self) -> &str {
        "post_to_group"
    }

    fn description(&self) -> &str {
        "Post a message to the group room this run belongs to. This is the ONLY output other group members and the user see in the room; regular replies stay private to this run."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use post_to_group to speak in the group room when you were addressed there.

Rules:
- Your run transcript is private scratch space. Only post_to_group messages reach the room.
- You have a small per-turn posting budget; consolidate your contribution into one clear message instead of streaming fragments.
- Mention members with @slug when addressing them; pass reply_to_message_id to answer one specific room message.
- Post conclusions, decisions, questions for the room, or concise findings. Do not post raw logs or partial thinking.
- If you have nothing useful to add, finish without posting."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to post to the group room"
                },
                "reply_to_message_id": {
                    "type": "string",
                    "description": "Optional id of the room message this replies to"
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(params) => params,
            Err(error) => return error,
        };
        let message = params.message.trim();
        if message.is_empty() {
            return ToolResult::invalid_parameters("message must not be empty");
        }
        if message.len() > MAX_HIVE_GROUP_MESSAGE_BYTES {
            return ToolResult::invalid_parameters(format!(
                "message exceeds {MAX_HIVE_GROUP_MESSAGE_BYTES} bytes"
            ));
        }
        let Some(group_run) = ctx.hive_group_run.as_ref() else {
            return ToolResult::error_with_code(
                "not_a_group_run",
                "post_to_group is only available while executing a group turn",
            );
        };
        let Some(db_path) = ctx.db_path.as_ref() else {
            return ToolResult::error_with_code(
                "group_post_failed",
                "this execution has no database attached",
            );
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(error) => {
                return ToolResult::error_with_code(
                    "group_post_failed",
                    format!("could not open the group store: {error}"),
                )
            }
        };
        let append = HiveGroupStore::new(db).append_worker_message_capped(
            &NewHiveGroupMessage {
                group_id: group_run.group_id.clone(),
                sender_kind: HiveGroupSenderKind::Worker,
                sender_worker_id: Some(group_run.worker_id.clone()),
                sender_run_id: Some(group_run.run_id.clone()),
                content: message.to_string(),
                reply_to_message_id: params
                    .reply_to_message_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                turn_id: Some(group_run.group_turn_id.clone()),
                idempotency_key: None,
            },
            group_run.max_member_messages_per_turn,
        );
        match append {
            Ok(CappedGroupAppend::Appended { message, posted }) => {
                ToolResult::success_data(json!({
                    "posted": true,
                    "message_id": message.id,
                    "seq": message.seq,
                    "group_id": message.group_id,
                    "turn_id": message.turn_id,
                    "remaining_posts_this_run": group_run
                        .max_member_messages_per_turn
                        .saturating_sub(posted),
                }))
            }
            Ok(CappedGroupAppend::CapExceeded { cap, posted }) => ToolResult::error_with_code(
                "group_message_cap_exceeded",
                format!(
                    "this run already posted {posted} of {cap} allowed group message(s); finish your work without further posts"
                ),
            ),
            Err(error) => {
                ToolResult::error_with_code("group_post_failed", bounded_error(&error.to_string()))
            }
        }
    }
}

fn bounded_error(error: &str) -> String {
    const MAX_ERROR_BYTES: usize = 512;
    if error.len() <= MAX_ERROR_BYTES {
        return error.to_string();
    }
    let mut end = MAX_ERROR_BYTES;
    while end > 0 && !error.is_char_boundary(end) {
        end -= 1;
    }
    error[..end].to_string()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::storage::{
        Database, HiveGroupRunContext, HiveGroupStore, HiveGroupTurnStatus, HiveWorkerStore,
        NewHiveGroup, NewHiveGroupMessage, NewHiveWorker,
    };
    use crate::tools::registry::{Tool, ToolContext};

    use super::PostToGroupTool;

    struct GroupFixture {
        db_path: std::path::PathBuf,
        group_id: String,
        turn_id: String,
        worker_id: String,
        _temp: TempDir,
    }

    fn fixture() -> GroupFixture {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("group-tool.db");
        let worker_store = HiveWorkerStore::new(Database::new(&db_path).unwrap());
        let worker = worker_store.create(&NewHiveWorker::new("builder")).unwrap();
        let store = HiveGroupStore::new(Database::new(&db_path).unwrap());
        let group = store
            .create(&NewHiveGroup {
                title: "Room".into(),
                max_member_messages_per_turn: Some(2),
                member_worker_ids: vec![worker.id.clone()],
                ..NewHiveGroup::default()
            })
            .unwrap();
        let trigger = store
            .append_message(&NewHiveGroupMessage::user(&group.id, "go"))
            .unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let turn = crate::storage::HiveGroupTurn {
            id: uuid::Uuid::new_v4().to_string(),
            group_id: group.id.clone(),
            trigger_message_id: trigger.id,
            execution_mode: group.execution_mode,
            policy: crate::storage::HiveGroupTurnPolicy::from(&group),
            speaker_plan: vec![worker.id.clone()],
            next_speaker_index: 0,
            status: HiveGroupTurnStatus::Running,
            member_outcomes: None,
            started_at: now.clone(),
            finished_at: None,
            created_at: now.clone(),
            updated_at: now,
        };
        let turn_db = Database::new(&db_path).unwrap();
        crate::storage::hive_groups::insert_turn_with_conn(turn_db.conn(), &turn).unwrap();
        GroupFixture {
            db_path,
            group_id: group.id,
            turn_id: turn.id,
            worker_id: worker.id,
            _temp: temp,
        }
    }

    fn group_ctx(fixture: &GroupFixture, run_id: &str) -> ToolContext {
        ToolContext {
            db_path: Some(fixture.db_path.clone()),
            hive_group_run: Some(HiveGroupRunContext {
                group_id: fixture.group_id.clone(),
                group_turn_id: fixture.turn_id.clone(),
                run_id: run_id.to_string(),
                worker_id: fixture.worker_id.clone(),
                max_member_messages_per_turn: 2,
                context_window_messages: 24,
            }),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn posting_outside_a_group_run_is_a_structured_error() {
        let result = PostToGroupTool
            .execute(
                serde_json::json!({"message": "hello"}),
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(
            result.output.contains("not_a_group_run"),
            "{}",
            result.output
        );
    }

    #[tokio::test]
    async fn posts_append_to_the_room_and_enforce_the_per_run_cap() {
        let fixture = fixture();
        let ctx = group_ctx(&fixture, "run-1");

        for expected_seq in [2_i64, 3] {
            let result = PostToGroupTool
                .execute(serde_json::json!({"message": "finding"}), &ctx)
                .await;
            assert!(!result.is_error, "{}", result.output);
            let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
            assert_eq!(parsed["data"]["seq"], expected_seq);
        }

        let capped = PostToGroupTool
            .execute(serde_json::json!({"message": "one too many"}), &ctx)
            .await;
        assert!(capped.is_error);
        assert!(
            capped.output.contains("group_message_cap_exceeded"),
            "{}",
            capped.output
        );

        // A different run (a later roundtable round) posts with a new budget.
        let next_round = PostToGroupTool
            .execute(
                serde_json::json!({"message": "round two"}),
                &group_ctx(&fixture, "run-2"),
            )
            .await;
        assert!(!next_round.is_error, "{}", next_round.output);

        let store = HiveGroupStore::new(Database::new(&fixture.db_path).unwrap());
        let messages = store.list_messages_after(&fixture.group_id, 0, 10).unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.turn_id.as_deref() == Some(fixture.turn_id.as_str()))
                .count(),
            3
        );
    }
}
