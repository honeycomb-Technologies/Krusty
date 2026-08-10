//! Typed thread-history contracts shared by the Codex and Mitsuro desktop adapters.
//!
//! Codex owns the wire protocol. Mitsuro maps its real persisted transcript into the
//! same read/search presentation contract so the GPUI can remain backend-neutral.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters for `thread/searchOccurrences`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchOccurrencesParams {
    pub thread_id: String,
    pub search_term: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl ThreadSearchOccurrencesParams {
    pub fn new(thread_id: impl Into<String>, search_term: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            search_term: search_term.into(),
            cursor: None,
            limit: None,
        }
    }
}

/// UTF-16 code-unit range, matching the Codex app-server wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchTextRange {
    pub start: u32,
    pub end: u32,
}

/// One visible message occurrence returned by `thread/searchOccurrences`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchOccurrence {
    pub turn_id: String,
    pub turn_cursor: String,
    pub item_id: String,
    pub snippet: String,
    pub snippet_match_range: ThreadSearchTextRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchOccurrencesResponse {
    pub data: Vec<ThreadSearchOccurrence>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Deprecated Codex rollback request, retained because the reference desktop still
/// uses it to edit the latest user turn and to recover retryable turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRollbackParams {
    pub thread_id: String,
    pub num_turns: u32,
}

impl ThreadRollbackParams {
    pub fn one(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            num_turns: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRollbackResponse {
    pub thread: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadTurnItemsView {
    NotLoaded,
    Summary,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadTurnsSortDirection {
    Asc,
    Desc,
}

/// Parameters for `thread/turns/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTurnsListParams {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_direction: Option<ThreadTurnsSortDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_view: Option<ThreadTurnItemsView>,
}

impl ThreadTurnsListParams {
    pub fn newest(thread_id: impl Into<String>, limit: u32) -> Self {
        Self {
            thread_id: thread_id.into(),
            cursor: None,
            limit: Some(limit),
            sort_direction: Some(ThreadTurnsSortDirection::Desc),
            items_view: Some(ThreadTurnItemsView::Full),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTurnsListResponse {
    pub data: Vec<Value>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub backwards_cursor: Option<String>,
}

/// Map a real thread/read payload into the occurrence contract. This is used by
/// the Mitsuro adapter and by offline contract tests; Codex calls its native method.
pub fn search_occurrences_in_thread(
    thread: &Value,
    params: &ThreadSearchOccurrencesParams,
) -> ThreadSearchOccurrencesResponse {
    let needle = params.search_term.trim();
    if needle.is_empty() {
        return ThreadSearchOccurrencesResponse {
            data: Vec::new(),
            next_cursor: None,
        };
    }

    let offset = parse_local_cursor(params.cursor.as_deref()).unwrap_or(0);
    let limit = params.limit.unwrap_or(20).clamp(1, 100) as usize;
    let mut all = Vec::new();

    if let Some(turns) = thread.get("turns").and_then(Value::as_array) {
        for (turn_index, turn) in turns.iter().enumerate() {
            let turn_id = turn
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("mitsuro-turn-{turn_index}"));
            let turn_cursor = format!("mitsuro-turn:{turn_index}");
            let Some(items) = turn.get("items").and_then(Value::as_array) else {
                continue;
            };
            for (item_index, item) in items.iter().enumerate() {
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                let text = match item_type {
                    "userMessage" => visible_user_text(item),
                    "agentMessage"
                        if item.get("phase").and_then(Value::as_str) != Some("commentary") =>
                    {
                        item.get("text").and_then(Value::as_str).map(str::to_owned)
                    }
                    _ => None,
                };
                let Some(text) = text else { continue };
                let item_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("mitsuro-item-{turn_index}-{item_index}"));
                for byte_range in case_insensitive_ranges(&text, needle) {
                    let (snippet, snippet_start, snippet_prefix_utf16) =
                        bounded_snippet(&text, byte_range.start);
                    let match_start = text[..byte_range.start].encode_utf16().count()
                        - text[..snippet_start].encode_utf16().count()
                        + snippet_prefix_utf16;
                    let match_end = match_start + text[byte_range.clone()].encode_utf16().count();
                    all.push(ThreadSearchOccurrence {
                        turn_id: turn_id.clone(),
                        turn_cursor: turn_cursor.clone(),
                        item_id: item_id.clone(),
                        snippet,
                        snippet_match_range: ThreadSearchTextRange {
                            start: match_start as u32,
                            end: match_end as u32,
                        },
                    });
                }
            }
        }
    }

    let end = offset.saturating_add(limit).min(all.len());
    let data = all.get(offset..end).unwrap_or_default().to_vec();
    let next_cursor = (end < all.len()).then(|| format!("mitsuro-offset:{end}"));
    ThreadSearchOccurrencesResponse { data, next_cursor }
}

/// Map real turns from a thread/read payload into a bounded page.
pub fn list_turns_in_thread(
    thread: &Value,
    params: &ThreadTurnsListParams,
) -> ThreadTurnsListResponse {
    let mut turns = thread
        .get("turns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let direction = params
        .sort_direction
        .unwrap_or(ThreadTurnsSortDirection::Desc);
    if direction == ThreadTurnsSortDirection::Desc {
        turns.reverse();
    }
    let offset = resolve_turn_cursor(params.cursor.as_deref(), turns.len(), direction).unwrap_or(0);
    let limit = params.limit.unwrap_or(20).clamp(1, 100) as usize;
    let end = offset.saturating_add(limit).min(turns.len());
    let mut data = turns.get(offset..end).unwrap_or_default().to_vec();
    if params.items_view == Some(ThreadTurnItemsView::NotLoaded) {
        for turn in &mut data {
            if let Some(object) = turn.as_object_mut() {
                object.insert("items".to_owned(), Value::Array(Vec::new()));
                object.insert(
                    "itemsView".to_owned(),
                    Value::String("notLoaded".to_owned()),
                );
            }
        }
    }
    ThreadTurnsListResponse {
        data,
        next_cursor: (end < turns.len()).then(|| format!("mitsuro-offset:{end}")),
        backwards_cursor: (!turns.is_empty()).then(|| format!("mitsuro-offset:{offset}")),
    }
}

fn visible_user_text(item: &Value) -> Option<String> {
    let text = item
        .get("content")?
        .as_array()?
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn parse_local_cursor(cursor: Option<&str>) -> Option<usize> {
    cursor?
        .strip_prefix("mitsuro-offset:")
        .and_then(|value| value.parse().ok())
}

fn resolve_turn_cursor(
    cursor: Option<&str>,
    turn_count: usize,
    direction: ThreadTurnsSortDirection,
) -> Option<usize> {
    if let Some(offset) = parse_local_cursor(cursor) {
        return Some(offset);
    }
    let original_index = cursor?
        .strip_prefix("mitsuro-turn:")?
        .parse::<usize>()
        .ok()?;
    if original_index >= turn_count {
        return None;
    }
    Some(match direction {
        ThreadTurnsSortDirection::Asc => original_index,
        ThreadTurnsSortDirection::Desc => turn_count - original_index - 1,
    })
}

fn case_insensitive_ranges(text: &str, needle: &str) -> Vec<std::ops::Range<usize>> {
    let folded_needle = needle.to_lowercase();
    if folded_needle.is_empty() {
        return Vec::new();
    }
    let starts = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    let mut ranges = Vec::new();
    let mut start_index = 0;
    while start_index + 1 < starts.len() {
        let start = starts[start_index];
        let mut folded = String::new();
        let mut matched = None;
        for end_index in start_index + 1..starts.len() {
            folded.push_str(&text[starts[end_index - 1]..starts[end_index]].to_lowercase());
            if folded == folded_needle {
                matched = Some(starts[end_index]);
                break;
            }
            if !folded_needle.starts_with(&folded) {
                break;
            }
        }
        if let Some(end) = matched {
            ranges.push(start..end);
            while start_index + 1 < starts.len() && starts[start_index] < end {
                start_index += 1;
            }
        } else {
            start_index += 1;
        }
    }
    ranges
}

fn bounded_snippet(text: &str, match_start: usize) -> (String, usize, usize) {
    const CONTEXT_CHARS: usize = 80;
    let char_starts = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    let match_char = char_starts
        .partition_point(|index| *index < match_start)
        .min(char_starts.len().saturating_sub(1));
    let start_char = match_char.saturating_sub(CONTEXT_CHARS);
    let end_char = match_char
        .saturating_add(CONTEXT_CHARS * 2)
        .min(char_starts.len().saturating_sub(1));
    let start = char_starts[start_char];
    let end = char_starts[end_char];
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.push_str(&text[start..end]);
    if end < text.len() {
        snippet.push('…');
    }
    (snippet, start, usize::from(start > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_wire_shapes_match_generated_codex_schema() {
        let params = ThreadSearchOccurrencesParams {
            thread_id: "thread-1".to_owned(),
            search_term: "Layout".to_owned(),
            cursor: Some("cursor-1".to_owned()),
            limit: Some(10),
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({
                "threadId": "thread-1",
                "searchTerm": "Layout",
                "cursor": "cursor-1",
                "limit": 10
            })
        );
        let turns = ThreadTurnsListParams::newest("thread-1", 5);
        assert_eq!(
            serde_json::to_value(turns).unwrap(),
            serde_json::json!({
                "threadId": "thread-1",
                "limit": 5,
                "sortDirection": "desc",
                "itemsView": "full"
            })
        );
        assert_eq!(
            serde_json::to_value(ThreadRollbackParams::one("thread-1")).unwrap(),
            serde_json::json!({"threadId": "thread-1", "numTurns": 1})
        );
    }

    #[test]
    fn local_search_uses_real_visible_messages_and_utf16_ranges() {
        let thread = serde_json::json!({
            "turns": [{
                "id": "turn-1",
                "items": [
                    {"id":"user-1","type":"userMessage","content":[{"type":"text","text":"A 😀 Layout plan"}]},
                    {"id":"comment-1","type":"agentMessage","phase":"commentary","text":"layout hidden"},
                    {"id":"agent-1","type":"agentMessage","phase":"final_answer","text":"LAYOUT done"}
                ]
            }]
        });
        let response = search_occurrences_in_thread(
            &thread,
            &ThreadSearchOccurrencesParams::new("thread-1", "layout"),
        );
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[0].item_id, "user-1");
        assert_eq!(response.data[0].snippet_match_range.start, 5);
        assert_eq!(response.data[0].snippet_match_range.end, 11);
        assert_eq!(response.data[1].item_id, "agent-1");
    }

    #[test]
    fn local_turn_pages_are_real_bounded_history() {
        let thread = serde_json::json!({"turns":[
            {"id":"one","items":[{"id":"a"}]},
            {"id":"two","items":[{"id":"b"}]},
            {"id":"three","items":[{"id":"c"}]}
        ]});
        let first = list_turns_in_thread(&thread, &ThreadTurnsListParams::newest("thread-1", 2));
        assert_eq!(first.data[0]["id"], "three");
        assert_eq!(first.data[1]["id"], "two");
        assert_eq!(first.next_cursor.as_deref(), Some("mitsuro-offset:2"));
        let second = list_turns_in_thread(
            &thread,
            &ThreadTurnsListParams {
                cursor: first.next_cursor,
                ..ThreadTurnsListParams::newest("thread-1", 2)
            },
        );
        assert_eq!(second.data[0]["id"], "one");
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn occurrence_turn_cursor_hydrates_in_both_directions() {
        let thread = serde_json::json!({"turns":[
            {"id":"one","items":[]},
            {"id":"two","items":[]},
            {"id":"three","items":[]},
            {"id":"four","items":[]}
        ]});
        let page = |sort_direction| {
            list_turns_in_thread(
                &thread,
                &ThreadTurnsListParams {
                    thread_id: "thread-1".to_owned(),
                    cursor: Some("mitsuro-turn:2".to_owned()),
                    limit: Some(2),
                    sort_direction: Some(sort_direction),
                    items_view: Some(ThreadTurnItemsView::Full),
                },
            )
        };
        let older = page(ThreadTurnsSortDirection::Desc);
        assert_eq!(older.data[0]["id"], "three");
        assert_eq!(older.data[1]["id"], "two");
        let newer = page(ThreadTurnsSortDirection::Asc);
        assert_eq!(newer.data[0]["id"], "three");
        assert_eq!(newer.data[1]["id"], "four");
    }
}
