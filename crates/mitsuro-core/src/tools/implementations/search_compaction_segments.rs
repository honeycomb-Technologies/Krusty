//! Search persisted compaction segments for recovery after in-place compaction.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{CompactionStore, Database};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
const MAX_SEGMENT_PREVIEW_CHARS: usize = 4_000;

pub struct SearchCompactionSegmentsTool;

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for SearchCompactionSegmentsTool {
    fn name(&self) -> &str {
        "search_compaction_segments"
    }

    fn description(&self) -> &str {
        "Search persisted compaction segments for the current session. Use to recover details dropped during in-place compaction."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Search archived conversation segments saved during compaction checkpoints.

Use when:
- You need details from before a compaction boundary
- A user references work that was summarized away
- You want to verify what was compacted in a prior checkpoint

Provide an optional query to filter segment markdown by keyword. Results are ordered newest-first."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional keyword filter applied to segment markdown"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum segments to return (default 5, max 20)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let Some(session_id) = ctx.session_id.as_deref() else {
            return ToolResult::error("search_compaction_segments requires an active session");
        };

        let Some(db_path) = ctx.db_path.as_deref() else {
            return ToolResult::error("search_compaction_segments requires database access");
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(err) => return ToolResult::error(format!("failed to open database: {err}")),
        };

        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let store = CompactionStore::new(&db);
        let segments = match store.search_segments(session_id, params.query.as_deref(), limit) {
            Ok(segments) => segments,
            Err(err) => return ToolResult::error(format!("segment search failed: {err}")),
        };

        if segments.is_empty() {
            return ToolResult::success_data(json!({
                "segments": [],
                "message": "No compaction segments found for this session."
            }));
        }

        let payload: Vec<Value> = segments
            .into_iter()
            .map(|segment| {
                let (preview, truncated) = preview_segment(&segment.segment_markdown);
                json!({
                    "id": segment.id,
                    "checkpoint_id": segment.checkpoint_id,
                    "message_id_start": segment.message_id_start,
                    "message_id_end": segment.message_id_end,
                    "token_estimate": segment.token_estimate,
                    "created_at": segment.created_at,
                    "segment_markdown_preview": preview,
                    "segment_markdown_chars": segment.segment_markdown.chars().count(),
                    "truncated": truncated,
                })
            })
            .collect();

        ToolResult::success_data(json!({
            "segments": payload,
            "count": payload.len(),
        }))
    }
}

fn preview_segment(segment_markdown: &str) -> (String, bool) {
    if segment_markdown.chars().count() <= MAX_SEGMENT_PREVIEW_CHARS {
        return (segment_markdown.to_string(), false);
    }

    let mut preview: String = segment_markdown
        .chars()
        .take(MAX_SEGMENT_PREVIEW_CHARS)
        .collect();
    preview.push_str("\n… [truncated; refine query for narrower recovery]");
    (preview, true)
}
