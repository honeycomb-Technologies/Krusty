use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{
    Database, HiveDeliveryPriority, HiveDeliveryStore, HiveWorkerStatus, HiveWorkerStore,
    NewHiveDelivery, MAX_HIVE_DELIVERY_BODY_BYTES,
};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

/// The only way a Hive Worker can send a durable private message to another
/// Worker. The row lands on the recipient's DM lane via the daemon pump;
/// this tool only writes the ledger.
pub struct SendToWorkerTool;

#[derive(Deserialize)]
struct Params {
    recipient: String,
    message: String,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    dedupe_key: Option<String>,
}

#[async_trait]
impl Tool for SendToWorkerTool {
    fn name(&self) -> &str {
        "send_to_worker"
    }

    fn description(&self) -> &str {
        "Send a durable private message to another Hive Worker. The message is queued on their DM lane; high priority may interrupt an active run."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use send_to_worker to message another Hive Worker privately.

Rules:
- recipient is the other Worker's slug (not a display name).
- The message is delivered to their private DM, never into a group room.
- Use priority "high" only when the recipient must see this before finishing current work.
- Pass the same dedupe_key when retrying an identical send so the ledger does not duplicate it.
- Do not message yourself."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "recipient": {
                    "type": "string",
                    "description": "Slug of the Worker to message"
                },
                "message": {
                    "type": "string",
                    "description": "Private message body"
                },
                "priority": {
                    "type": "string",
                    "enum": ["normal", "high"],
                    "description": "normal waits for an idle lane; high may steer an active run"
                },
                "dedupe_key": {
                    "type": "string",
                    "description": "Optional idempotency key scoped to this sender run"
                }
            },
            "required": ["recipient", "message"],
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
        if message.len() > MAX_HIVE_DELIVERY_BODY_BYTES {
            return ToolResult::invalid_parameters(format!(
                "message exceeds {MAX_HIVE_DELIVERY_BODY_BYTES} bytes"
            ));
        }
        let recipient_slug = params.recipient.trim();
        if recipient_slug.is_empty() {
            return ToolResult::invalid_parameters("recipient must not be empty");
        }
        let priority = match params
            .priority
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("normal")
        {
            "normal" => HiveDeliveryPriority::Normal,
            "high" => HiveDeliveryPriority::High,
            other => {
                return ToolResult::invalid_parameters(format!(
                    "priority must be normal or high, not {other}"
                ))
            }
        };
        if ctx.hive_group_run.is_none() && ctx.session_id.is_none() {
            return ToolResult::error_with_code(
                "not_a_worker_run",
                "send_to_worker is only available on a Worker DM or group turn",
            );
        }
        let Some(db_path) = ctx.db_path.as_ref() else {
            return ToolResult::error_with_code(
                "worker_send_failed",
                "this execution has no database attached",
            );
        };
        let workers = match Database::new(db_path) {
            Ok(opened) => HiveWorkerStore::new(opened),
            Err(error) => {
                return ToolResult::error_with_code(
                    "worker_send_failed",
                    format!("could not open the worker store: {error}"),
                )
            }
        };

        let sender = match resolve_sender(&workers, ctx) {
            Ok(sender) => sender,
            Err(result) => return result,
        };
        let recipient = match workers.get_by_slug(sender.user_id.as_deref(), recipient_slug) {
            Ok(Some(worker)) => worker,
            Ok(None) => {
                return match workers
                    .get_by_slug_any_status(sender.user_id.as_deref(), recipient_slug)
                {
                    Ok(Some(worker)) if worker.status == HiveWorkerStatus::Archived => {
                        ToolResult::error_with_code(
                            "recipient_archived",
                            format!("Worker @{recipient_slug} is archived"),
                        )
                    }
                    Ok(_) => ToolResult::error_with_code(
                        "recipient_unknown",
                        format!("Worker @{recipient_slug} was not found"),
                    ),
                    Err(error) => ToolResult::error_with_code(
                        "worker_send_failed",
                        bounded_error(&error.to_string()),
                    ),
                }
            }
            Err(error) => {
                return ToolResult::error_with_code(
                    "worker_send_failed",
                    bounded_error(&error.to_string()),
                )
            }
        };
        if recipient.id == sender.id {
            return ToolResult::error_with_code(
                "self_send",
                "send_to_worker cannot message the sending Worker",
            );
        }

        let scope = ctx
            .hive_group_run
            .as_ref()
            .map(|run| format!("group-run:{}", run.run_id))
            .or_else(|| ctx.session_id.as_ref().map(|id| format!("dm-session:{id}")))
            .unwrap_or_else(|| format!("worker:{}", sender.id));
        let key = params
            .dedupe_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| ctx.tool_use_id.clone())
            .unwrap_or_else(|| format!("{}:{}:{}", recipient.slug, priority.as_str(), message));
        let dedupe_key = format!("{scope}:{key}");

        let store = match Database::new(db_path) {
            Ok(opened) => HiveDeliveryStore::new(opened),
            Err(error) => {
                return ToolResult::error_with_code(
                    "worker_send_failed",
                    format!("could not open the delivery store: {error}"),
                )
            }
        };
        let group_id = ctx.hive_group_run.as_ref().and_then(|run| {
            Database::new(db_path).ok().and_then(|db| {
                db.conn()
                    .query_row(
                        "SELECT id FROM hive_groups WHERE id = ?1",
                        [&run.group_id],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
            })
        });
        match store.enqueue(&NewHiveDelivery {
            from_worker_id: Some(sender.id.clone()),
            group_id,
            priority,
            dedupe_key: Some(dedupe_key),
            ..NewHiveDelivery::worker_message(recipient.id.clone(), message)
        }) {
            Ok(enqueued) => ToolResult::success_data(json!({
                "queued": true,
                "delivery_id": enqueued.delivery.id,
                "status": enqueued.delivery.status.as_str(),
                "deduplicated": enqueued.deduplicated,
                "recipient": recipient.slug,
                "priority": priority.as_str(),
                "recipient_paused": recipient.status == HiveWorkerStatus::Paused,
            })),
            Err(error) => {
                ToolResult::error_with_code("worker_send_failed", bounded_error(&error.to_string()))
            }
        }
    }
}

fn resolve_sender(
    workers: &HiveWorkerStore,
    ctx: &ToolContext,
) -> Result<crate::storage::HiveWorker, ToolResult> {
    if let Some(group_run) = ctx.hive_group_run.as_ref() {
        return workers
            .get(&group_run.worker_id)
            .ok()
            .flatten()
            .ok_or_else(|| {
                ToolResult::error_with_code(
                    "sender_unknown",
                    "this group run is not bound to a Worker",
                )
            });
    }
    let Some(session_id) = ctx.session_id.as_deref() else {
        return Err(ToolResult::error_with_code(
            "not_a_worker_run",
            "send_to_worker is only available on a Worker DM or group turn",
        ));
    };
    workers
        .get_by_dm_session(session_id)
        .ok()
        .flatten()
        .ok_or_else(|| {
            ToolResult::error_with_code(
                "not_a_worker_run",
                "send_to_worker is only available on a Worker DM or group turn",
            )
        })
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
        Database, HiveDeliveryStatus, HiveDeliveryStore, HiveGroupRunContext, HiveWorkerStatus,
        HiveWorkerStore, NewHiveWorker,
    };
    use crate::tools::registry::{Tool, ToolContext};

    use super::SendToWorkerTool;

    struct Fixture {
        db_path: std::path::PathBuf,
        sender_id: String,
        recipient_id: String,
        _temp: TempDir,
    }

    fn fixture() -> Fixture {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("send-to-worker.db");
        let db = Database::new(&db_path).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
                 VALUES ('dm-sender', 'Sender DM', '2026-08-01T00:00:00.000000Z',
                         '2026-08-01T00:00:00.000000Z', 'hive');",
            )
            .unwrap();
        let workers = HiveWorkerStore::new(Database::new(&db_path).unwrap());
        let sender = workers
            .create(&NewHiveWorker {
                dm_session_id: Some("dm-sender".into()),
                ..NewHiveWorker::new("sender")
            })
            .unwrap();
        let recipient = workers.create(&NewHiveWorker::new("recipient")).unwrap();
        Fixture {
            db_path,
            sender_id: sender.id,
            recipient_id: recipient.id,
            _temp: temp,
        }
    }

    fn dm_ctx(fixture: &Fixture) -> ToolContext {
        ToolContext {
            db_path: Some(fixture.db_path.clone()),
            session_id: Some("dm-sender".into()),
            tool_use_id: Some("call-1".into()),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn sending_outside_a_worker_run_is_a_structured_error() {
        let result = SendToWorkerTool
            .execute(
                serde_json::json!({"recipient": "recipient", "message": "hi"}),
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(
            result.output.contains("not_a_worker_run"),
            "{}",
            result.output
        );
    }

    #[tokio::test]
    async fn queues_a_delivery_and_dedupes_the_same_call() {
        let fixture = fixture();
        let ctx = dm_ctx(&fixture);
        let first = SendToWorkerTool
            .execute(
                serde_json::json!({"recipient": "recipient", "message": "status?"}),
                &ctx,
            )
            .await;
        assert!(!first.is_error, "{}", first.output);
        let parsed: serde_json::Value = serde_json::from_str(&first.output).unwrap();
        assert_eq!(parsed["data"]["queued"], true);
        assert_eq!(parsed["data"]["deduplicated"], false);

        let replay = SendToWorkerTool
            .execute(
                serde_json::json!({"recipient": "recipient", "message": "status?"}),
                &ctx,
            )
            .await;
        assert!(!replay.is_error, "{}", replay.output);
        let replayed: serde_json::Value = serde_json::from_str(&replay.output).unwrap();
        assert_eq!(replayed["data"]["deduplicated"], true);
        assert_eq!(
            replayed["data"]["delivery_id"],
            parsed["data"]["delivery_id"]
        );

        let store = HiveDeliveryStore::new(Database::new(&fixture.db_path).unwrap());
        let rows = store
            .list_for_worker(&fixture.recipient_id, Some(HiveDeliveryStatus::Pending), 10)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].from_worker_id.as_deref(),
            Some(fixture.sender_id.as_str())
        );
    }

    #[tokio::test]
    async fn rejects_self_sends_and_unknown_or_archived_recipients() {
        let fixture = fixture();
        let ctx = dm_ctx(&fixture);
        let self_send = SendToWorkerTool
            .execute(
                serde_json::json!({"recipient": "sender", "message": "nope"}),
                &ctx,
            )
            .await;
        assert!(
            self_send.output.contains("self_send"),
            "{}",
            self_send.output
        );

        let missing = SendToWorkerTool
            .execute(
                serde_json::json!({"recipient": "ghost", "message": "hello"}),
                &ctx,
            )
            .await;
        assert!(
            missing.output.contains("recipient_unknown"),
            "{}",
            missing.output
        );

        HiveWorkerStore::new(Database::new(&fixture.db_path).unwrap())
            .set_status(&fixture.recipient_id, HiveWorkerStatus::Archived)
            .unwrap();
        let archived = SendToWorkerTool
            .execute(
                serde_json::json!({"recipient": "recipient", "message": "hello"}),
                &ctx,
            )
            .await;
        assert!(
            archived.output.contains("recipient_archived"),
            "{}",
            archived.output
        );
    }

    #[tokio::test]
    async fn group_run_sender_can_queue_a_high_priority_delivery() {
        let fixture = fixture();
        let ctx = ToolContext {
            db_path: Some(fixture.db_path.clone()),
            hive_group_run: Some(HiveGroupRunContext {
                group_id: "group-1".into(),
                group_turn_id: "turn-1".into(),
                run_id: "run-1".into(),
                worker_id: fixture.sender_id.clone(),
                max_member_messages_per_turn: 2,
                context_window_messages: 24,
            }),
            tool_use_id: Some("call-group".into()),
            ..ToolContext::default()
        };
        let result = SendToWorkerTool
            .execute(
                serde_json::json!({
                    "recipient": "recipient",
                    "message": "need you now",
                    "priority": "high"
                }),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.output);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["priority"], "high");
    }
}
