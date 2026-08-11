//! Subset of the Codex app-server JSON-RPC protocol used by P0.
//!
//! Typed subset of the Codex app-server protocol used by the desktop.
//! Wire format: newline-delimited JSON over stdio (see crate README).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::environment::ModeKind;

// ---------------------------------------------------------------------------
// JSON-RPC envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
}

impl From<u64> for JsonRpcId {
    fn from(value: u64) -> Self {
        Self::Number(value as i64)
    }
}

impl From<i64> for JsonRpcId {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

/// Outbound client request (we always set `jsonrpc`).
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: JsonRpcId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: impl Into<JsonRpcId>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub id: JsonRpcId,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct JsonRpcErrorBody {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub id: Option<JsonRpcId>,
    pub error: JsonRpcErrorBody,
}

/// Server→client notification or unsolicited message without a matching client request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Notification {
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub emitted_at_ms: Option<u64>,
}

/// Decoded inbound line from app-server stdout.
#[derive(Debug, Clone)]
pub enum JsonRpcMessage {
    Response(JsonRpcResponse),
    Notification(Notification),
    /// Server-originated request (has id + method) — P0 surfaces as notification-like event.
    ServerRequest {
        id: JsonRpcId,
        method: String,
        params: Option<Value>,
    },
    Unknown(Value),
}

impl JsonRpcMessage {
    pub fn parse_line(line: &str) -> Result<Self, serde_json::Error> {
        let value: Value = serde_json::from_str(line)?;
        Ok(classify_message(value))
    }
}

fn classify_message(value: Value) -> JsonRpcMessage {
    let id = value.get("id");
    let method = value.get("method").and_then(|m| m.as_str());
    let has_result = value.get("result").is_some();
    let has_error = value.get("error").is_some();

    if id.is_some() && (has_result || has_error) {
        if let Ok(resp) = serde_json::from_value::<JsonRpcResponse>(value.clone()) {
            return JsonRpcMessage::Response(resp);
        }
    }

    if let Some(method) = method {
        if let Some(id_val) = id {
            // Server request awaiting client response
            if let Ok(rpc_id) = serde_json::from_value::<JsonRpcId>(id_val.clone()) {
                return JsonRpcMessage::ServerRequest {
                    id: rpc_id,
                    method: method.to_string(),
                    params: value.get("params").cloned(),
                };
            }
        }
        // Notification (no id)
        return JsonRpcMessage::Notification(Notification {
            method: method.to_string(),
            params: value.get("params").cloned(),
            emitted_at_ms: value
                .get("emittedAtMs")
                .and_then(|v| v.as_u64())
                .or_else(|| value.get("emitted_at_ms").and_then(|v| v.as_u64())),
        });
    }

    JsonRpcMessage::Unknown(value)
}

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_api: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server_openai_form_elicitation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opt_out_notification_methods: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_attestation: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub client_info: ClientInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<InitializeCapabilities>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub codex_home: String,
    pub platform_family: String,
    pub platform_os: String,
    pub user_agent: String,
}

// ---------------------------------------------------------------------------
// thread/list
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_term: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_state_db_only: Option<bool>,
}

/// Compact thread summary for UI / trait surface (subset of full Thread).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub ephemeral: Option<bool>,
    #[serde(default)]
    pub is_pinned: Option<bool>,
    /// When true, thread is archived (fixture / list filter; not always on wire Thread).
    #[serde(default)]
    pub archived: Option<bool>,
    /// Full raw object when deserialized from server (optional for fixtures).
    #[serde(default, skip_serializing)]
    pub raw: Option<Value>,
}

impl ThreadSummary {
    pub fn display_title(&self) -> String {
        if let Some(name) = &self.name {
            if !name.trim().is_empty() {
                return name.clone();
            }
        }
        if let Some(preview) = &self.preview {
            let trimmed = preview.trim();
            if !trimmed.is_empty() {
                let max = 64;
                if trimmed.chars().count() > max {
                    let short: String = trimmed.chars().take(max).collect();
                    return format!("{short}…");
                }
                return trimmed.to_string();
            }
        }
        format!("Thread {}", &self.id[..self.id.len().min(8)])
    }

    /// Best-effort parse from a full server `Thread` object.
    pub fn from_value(value: &Value) -> Self {
        Self {
            id: value
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            preview: value
                .get("preview")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            cwd: value
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            created_at: value.get("createdAt").and_then(|v| v.as_i64()),
            updated_at: value.get("updatedAt").and_then(|v| v.as_i64()),
            model_provider: value
                .get("modelProvider")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            ephemeral: value.get("ephemeral").and_then(|v| v.as_bool()),
            is_pinned: value.get("isPinned").and_then(|v| v.as_bool()),
            archived: value.get("archived").and_then(|v| v.as_bool()),
            raw: Some(value.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListResponse {
    pub data: Vec<Value>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub backwards_cursor: Option<String>,
}

impl ThreadListResponse {
    pub fn threads(&self) -> Vec<ThreadSummary> {
        self.data.iter().map(ThreadSummary::from_value).collect()
    }
}

// ---------------------------------------------------------------------------
// thread/start
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    /// Model-advertised Codex service tier. `None` selects the standard tier.
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approvals_reviewer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_workspace_roots: Option<Vec<String>>,
    /// Transport-neutral product metadata consumed only by the Mitsuro adapter.
    #[serde(skip)]
    pub mitsuro_permission_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartResponse {
    pub thread: Value,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

impl ThreadStartResponse {
    pub fn summary(&self) -> ThreadSummary {
        ThreadSummary::from_value(&self.thread)
    }
}

// ---------------------------------------------------------------------------
// thread/read
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadParams {
    pub thread_id: String,
    /// When true, include turns and their items from rollout history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_turns: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadResponse {
    pub thread: Value,
}

impl ThreadReadResponse {
    pub fn summary(&self) -> ThreadSummary {
        ThreadSummary::from_value(&self.thread)
    }

    /// Best-effort extract of user/agent message text from `thread.turns[*].items`.
    pub fn transcript_messages(&self) -> Vec<TranscriptMessage> {
        extract_transcript_from_thread(&self.thread)
    }
}

/// Lightweight message pulled from a thread/read payload for UI transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptMessage {
    pub role: TranscriptRole,
    /// Flattened text fallback (user/assistant/reasoning/plan body, or a summary line).
    pub body: String,
    pub item_id: Option<String>,
    /// Populated when `role` is [`TranscriptRole::CommandExecution`].
    pub command: Option<CommandExecutionFields>,
    /// Populated when `role` is [`TranscriptRole::FileChange`].
    pub file_change: Option<FileChangeFields>,
    /// Populated for every non-chat activity item so reopening a thread keeps
    /// the same semantic block that was shown while the turn streamed.
    pub activity: Option<ActivityFields>,
    /// Image inputs attached to a persisted user message.
    pub images: Vec<TranscriptImage>,
    /// Audio inputs attached to a persisted user message.
    pub audio: Vec<TranscriptAudio>,
    /// Skill and file-mention references attached to a persisted user message.
    pub references: Vec<TranscriptReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptImage {
    pub source: TranscriptImageSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptImageSource {
    LocalPath(String),
    Url(String),
    Embedded { media_type: String, data: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptAudio {
    pub source: TranscriptAudioSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptAudioSource {
    LocalPath(String),
    Url(String),
    Embedded { media_type: String, data: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptReference {
    pub kind: TranscriptReferenceKind,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptReferenceKind {
    Skill,
    Mention,
}

impl TranscriptMessage {
    fn plain(role: TranscriptRole, body: String, item_id: Option<String>) -> Self {
        Self {
            role,
            body,
            item_id,
            command: None,
            file_change: None,
            activity: None,
            images: Vec::new(),
            audio: Vec::new(),
            references: Vec::new(),
        }
    }
}

/// Compact, backend-neutral presentation fields for an app-server activity item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityFields {
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub mcp_app: Option<crate::McpAppToolCall>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRole {
    User,
    Assistant,
    Reasoning,
    Plan,
    /// Generic system / unknown tool text (not a distinct UI block).
    System,
    /// Structured `commandExecution` item — map to CommandExecution UI block.
    CommandExecution,
    /// Structured `fileChange` item — map to FileChange UI block.
    FileChange,
}

pub fn extract_transcript_from_thread(thread: &Value) -> Vec<TranscriptMessage> {
    let mut out = Vec::new();
    let Some(turns) = thread.get("turns").and_then(|t| t.as_array()) else {
        return out;
    };
    for turn in turns {
        let Some(items) = turn.get("items").and_then(|i| i.as_array()) else {
            continue;
        };
        for item in items {
            if let Some(msg) = transcript_from_item(item) {
                out.push(msg);
            }
        }
    }
    out
}

/// Fast path for open-thread first paint: walk turns **newest-first**, keep only
/// user/assistant items, stop after `max_chat` messages. Avoids allocating huge
/// reasoning / fileChange bodies for 500+ item threads (UI hang source).
pub fn extract_chat_tail_from_thread(
    thread: &Value,
    max_chat: usize,
) -> (usize /* total_items_seen */, Vec<TranscriptMessage>) {
    if max_chat == 0 {
        return (0, Vec::new());
    }
    let mut seen = 0usize;
    let mut rev: Vec<TranscriptMessage> = Vec::with_capacity(max_chat);
    let Some(turns) = thread.get("turns").and_then(|t| t.as_array()) else {
        return (0, Vec::new());
    };
    for turn in turns.iter().rev() {
        let Some(items) = turn.get("items").and_then(|i| i.as_array()) else {
            continue;
        };
        for item in items.iter().rev() {
            seen += 1;
            let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if ty != "userMessage" && ty != "agentMessage" {
                continue;
            }
            if let Some(msg) = transcript_from_item(item) {
                rev.push(msg);
                if rev.len() >= max_chat {
                    rev.reverse();
                    return (seen, rev);
                }
            }
        }
    }
    rev.reverse();
    (seen, rev)
}

fn transcript_from_item(item: &Value) -> Option<TranscriptMessage> {
    let ty = item.get("type")?.as_str()?;
    let id = item.get("id").and_then(|v| v.as_str()).map(str::to_string);
    match ty {
        "userMessage" => {
            let content = item.get("content")?;
            let body = user_input_text(content);
            let images = user_input_images(content);
            let audio = user_input_audio(content);
            let references = user_input_references(content);
            if body.is_empty() && images.is_empty() && audio.is_empty() && references.is_empty() {
                return None;
            }
            let mut message = TranscriptMessage::plain(TranscriptRole::User, body, id);
            message.images = images;
            message.audio = audio;
            message.references = references;
            Some(message)
        }
        "agentMessage" => {
            let body = item.get("text")?.as_str()?.to_string();
            Some(TranscriptMessage::plain(
                TranscriptRole::Assistant,
                body,
                id,
            ))
        }
        "reasoning" => {
            let summary = item
                .get("summary")
                .and_then(|s| s.as_array())
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let content = item
                .get("content")
                .and_then(|s| s.as_array())
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let body = if !summary.is_empty() {
                summary
            } else {
                content
            };
            if body.is_empty() {
                return None;
            }
            Some(TranscriptMessage::plain(
                TranscriptRole::Reasoning,
                body,
                id,
            ))
        }
        "plan" => {
            let body = item.get("text")?.as_str()?.to_string();
            Some(TranscriptMessage::plain(TranscriptRole::Plan, body, id))
        }
        "commandExecution" => {
            // Same helpers as stream path so thread/read rebuilds CommandExecution blocks.
            let fields = command_execution_fields(item);
            let body = if !fields.output.is_empty() {
                format!(
                    "$ {} ({})\n{}",
                    fields.command, fields.status, fields.output
                )
            } else {
                format!("$ {} ({})", fields.command, fields.status)
            };
            Some(TranscriptMessage {
                role: TranscriptRole::CommandExecution,
                body,
                item_id: id,
                command: Some(fields),
                file_change: None,
                activity: None,
                images: Vec::new(),
                audio: Vec::new(),
                references: Vec::new(),
            })
        }
        "fileChange" => {
            // Same helpers as stream path so thread/read rebuilds FileChange blocks.
            let fields = file_change_fields(item);
            let body = if !fields.patch_preview.is_empty() {
                format!(
                    "file change · {} ({})\n{}",
                    fields.paths_summary, fields.status, fields.patch_preview
                )
            } else {
                format!("file change · {} ({})", fields.paths_summary, fields.status)
            };
            Some(TranscriptMessage {
                role: TranscriptRole::FileChange,
                body,
                item_id: id,
                command: None,
                file_change: Some(fields),
                activity: None,
                images: Vec::new(),
                audio: Vec::new(),
                references: Vec::new(),
            })
        }
        _ => {
            let activity = activity_item_fields(item);
            Some(TranscriptMessage {
                role: TranscriptRole::System,
                body: activity.summary.clone(),
                item_id: id,
                command: None,
                file_change: None,
                activity: Some(activity),
                images: Vec::new(),
                audio: Vec::new(),
                references: Vec::new(),
            })
        }
    }
}

/// Normalize every current or future non-chat ThreadItem into a bounded activity block.
/// Rich renderers can specialize by `kind`; this fallback prevents protocol evolution
/// from silently deleting durable thread history.
pub fn activity_item_fields(item: &Value) -> ActivityFields {
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("activity")
        .to_owned();
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let (title, summary) = match kind.as_str() {
        "mcpToolCall" => {
            let server = value_string(item, "server");
            let tool = value_string(item, "tool");
            let label = join_nonempty(&[server.as_str(), tool.as_str()], " · ");
            ("MCP tool".to_owned(), label)
        }
        "dynamicToolCall" => {
            let namespace = value_string(item, "namespace");
            let tool = value_string(item, "tool");
            let label = join_nonempty(&[namespace.as_str(), tool.as_str()], " · ");
            ("Tool call".to_owned(), label)
        }
        "webSearch" => ("Web search".to_owned(), value_string(item, "query")),
        "imageGeneration" => {
            let prompt = value_string(item, "revisedPrompt");
            let saved = value_string(item, "savedPath");
            (
                "Image generation".to_owned(),
                if prompt.is_empty() { saved } else { prompt },
            )
        }
        "imageView" => ("Viewed image".to_owned(), value_string(item, "path")),
        "collabAgentToolCall" => {
            let tool = value_string(item, "tool");
            let prompt = value_string(item, "prompt");
            (
                "Collaboration".to_owned(),
                join_nonempty(&[tool.as_str(), prompt.as_str()], " · "),
            )
        }
        "subAgentActivity" => {
            let path = value_string(item, "agentPath");
            let activity_kind = value_string(item, "kind");
            (
                "Subagent activity".to_owned(),
                join_nonempty(&[path.as_str(), activity_kind.as_str()], " · "),
            )
        }
        "contextCompaction" => (
            "Context compacted".to_owned(),
            "Conversation context was condensed for continuation.".to_owned(),
        ),
        "enteredReviewMode" => (
            "Review started".to_owned(),
            bounded_json_summary(item.get("review")),
        ),
        "exitedReviewMode" => (
            "Review completed".to_owned(),
            bounded_json_summary(item.get("review")),
        ),
        "hookPrompt" => (
            "Hook".to_owned(),
            collect_text_fragments(item.get("fragments")),
        ),
        "sleep" => {
            let duration = item
                .get("durationMs")
                .and_then(Value::as_u64)
                .map(format_duration_ms)
                .unwrap_or_default();
            ("Waiting".to_owned(), duration)
        }
        other => (humanize_item_kind(other), bounded_json_summary(Some(item))),
    };

    ActivityFields {
        kind,
        title,
        summary: bound_text(summary, 2_000),
        status,
        mcp_app: crate::McpAppToolCall::from_thread_item(item),
    }
}

fn value_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn join_nonempty(parts: &[&str], separator: &str) -> String {
    parts
        .iter()
        .copied()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

fn collect_text_fragments(value: Option<&Value>) -> String {
    let Some(parts) = value.and_then(Value::as_array) else {
        return bounded_json_summary(value);
    };
    let text = parts
        .iter()
        .filter_map(|part| {
            part.as_str()
                .map(str::to_owned)
                .or_else(|| part.get("text").and_then(Value::as_str).map(str::to_owned))
        })
        .collect::<Vec<_>>()
        .join("\n");
    bound_text(text, 2_000)
}

fn bounded_json_summary(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(text) = value.as_str() {
        return bound_text(text.to_owned(), 2_000);
    }
    bound_text(serde_json::to_string(value).unwrap_or_default(), 2_000)
}

fn bound_text(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut bounded = text.chars().take(max_chars).collect::<String>();
    bounded.push('…');
    bounded
}

fn humanize_item_kind(kind: &str) -> String {
    let mut out = String::new();
    for (index, ch) in kind.chars().enumerate() {
        if index > 0 && ch.is_ascii_uppercase() {
            out.push(' ');
        }
        if index == 0 {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "Activity".to_owned()
    } else {
        out
    }
}

fn format_duration_ms(duration_ms: u64) -> String {
    if duration_ms >= 1_000 {
        let seconds = duration_ms as f64 / 1_000.0;
        format!("Waiting for {seconds:.1} seconds")
    } else {
        format!("Waiting for {duration_ms} ms")
    }
}

fn user_input_text(content: &Value) -> String {
    let Some(arr) = content.as_array() else {
        return content.as_str().unwrap_or("").to_string();
    };
    arr.iter()
        .filter_map(|part| {
            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                part.get("text")
                    .and_then(|t| t.as_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn user_input_images(content: &Value) -> Vec<TranscriptImage> {
    let Some(parts) = content.as_array() else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
            Some("localImage") => {
                part.get("path")
                    .and_then(Value::as_str)
                    .map(|path| TranscriptImage {
                        source: TranscriptImageSource::LocalPath(path.to_owned()),
                    })
            }
            Some("image") => part
                .get("url")
                .and_then(Value::as_str)
                .map(|url| TranscriptImage {
                    source: data_image_parts(url)
                        .map(|(media_type, data)| TranscriptImageSource::Embedded {
                            media_type: media_type.to_owned(),
                            data: data.to_owned(),
                        })
                        .unwrap_or_else(|| TranscriptImageSource::Url(url.to_owned())),
                }),
            _ => None,
        })
        .collect()
}

fn data_image_parts(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (metadata, data) = rest.split_once(',')?;
    let media_type = metadata.strip_suffix(";base64")?;
    if !media_type.starts_with("image/") || data.is_empty() {
        return None;
    }
    Some((media_type, data))
}

fn user_input_audio(content: &Value) -> Vec<TranscriptAudio> {
    let Some(parts) = content.as_array() else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
            Some("localAudio") => {
                part.get("path")
                    .and_then(Value::as_str)
                    .map(|path| TranscriptAudio {
                        source: TranscriptAudioSource::LocalPath(path.to_owned()),
                    })
            }
            Some("audio") => part
                .get("url")
                .and_then(Value::as_str)
                .map(|url| TranscriptAudio {
                    source: data_audio_parts(url)
                        .map(|(media_type, data)| TranscriptAudioSource::Embedded {
                            media_type: media_type.to_owned(),
                            data: data.to_owned(),
                        })
                        .unwrap_or_else(|| TranscriptAudioSource::Url(url.to_owned())),
                }),
            _ => None,
        })
        .collect()
}

fn data_audio_parts(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (metadata, data) = rest.split_once(',')?;
    let media_type = metadata.strip_suffix(";base64")?;
    if !media_type.starts_with("audio/") || data.is_empty() {
        return None;
    }
    Some((media_type, data))
}

fn user_input_references(content: &Value) -> Vec<TranscriptReference> {
    let Some(parts) = content.as_array() else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| {
            let kind = match part.get("type").and_then(Value::as_str) {
                Some("skill") => TranscriptReferenceKind::Skill,
                Some("mention") => TranscriptReferenceKind::Mention,
                _ => return None,
            };
            Some(TranscriptReference {
                kind,
                name: part.get("name")?.as_str()?.to_owned(),
                path: part.get("path")?.as_str()?.to_owned(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// turn/start
// ---------------------------------------------------------------------------

/// Build a `UserInput` text part: `{ "type": "text", "text": "..." }`.
pub fn user_input_text_value(text: impl Into<String>) -> Value {
    serde_json::json!({
        "type": "text",
        "text": text.into(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams {
    pub thread_id: String,
    /// Array of UserInput objects (`text`, `image`, …).
    pub input: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Model-advertised Codex service tier. This intentionally serializes as
    /// `null` when standard is selected so a prior sticky fast tier is cleared.
    #[serde(default)]
    pub service_tier: Option<String>,
    /// Exact Codex collaboration preset for this turn and subsequent turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_mode: Option<CollaborationMode>,
    /// Reasoning effort advertised by the selected model. Codex app-server
    /// persists this override for this turn and subsequent turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Approval policy override accepted by Codex app-server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
    /// Approval reviewer override accepted by Codex app-server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approvals_reviewer: Option<String>,
    /// Typed sandbox policy override accepted by Codex app-server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<SandboxPolicy>,
    /// Named Codex permissions profile. Presets use `sandbox_policy`, not this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    /// Absolute workspace roots retained by Codex for this and subsequent turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_workspace_roots: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_user_message_id: Option<String>,
    /// Transport-neutral product metadata consumed only by the Mitsuro adapter.
    #[serde(skip)]
    pub mitsuro_fast_mode: Option<bool>,
    /// Transport-neutral product metadata consumed only by the Mitsuro adapter.
    #[serde(skip)]
    pub mitsuro_work_mode: Option<String>,
    /// Transport-neutral product metadata consumed only by the Mitsuro adapter.
    #[serde(skip)]
    pub mitsuro_permission_mode: Option<String>,
}

/// Schema-exact Codex app-server sandbox policy variants used by the access picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkAccess {
    Restricted,
    Enabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SandboxPolicy {
    #[serde(rename = "dangerFullAccess")]
    DangerFullAccess,
    #[serde(rename = "readOnly")]
    ReadOnly {
        #[serde(default, rename = "networkAccess")]
        network_access: bool,
    },
    #[serde(rename = "externalSandbox")]
    ExternalSandbox {
        #[serde(rename = "networkAccess")]
        network_access: NetworkAccess,
    },
    #[serde(rename = "workspaceWrite")]
    WorkspaceWrite {
        #[serde(default, rename = "writableRoots")]
        writable_roots: Vec<String>,
        #[serde(default, rename = "networkAccess")]
        network_access: bool,
        #[serde(default, rename = "excludeSlashTmp")]
        exclude_slash_tmp: bool,
        #[serde(default, rename = "excludeTmpdirEnvVar")]
        exclude_tmpdir_env_var: bool,
    },
}

impl TurnStartParams {
    pub fn text(thread_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            input: vec![user_input_text_value(text)],
            model: None,
            service_tier: None,
            collaboration_mode: None,
            effort: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            permissions: None,
            runtime_workspace_roots: None,
            cwd: None,
            client_user_message_id: None,
            mitsuro_fast_mode: None,
            mitsuro_work_mode: None,
            mitsuro_permission_mode: None,
        }
    }

    /// Text turn with optional model override (`TurnStartParams.model` → wire `model`).
    pub fn text_with_model(
        thread_id: impl Into<String>,
        text: impl Into<String>,
        model: Option<String>,
    ) -> Self {
        let mut p = Self::text(thread_id, text);
        p.model = model;
        p
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn push_local_image(&mut self, path: impl Into<String>) {
        self.input.push(serde_json::json!({
            "type": "localImage",
            "path": path.into(),
            "detail": null
        }));
    }

    pub fn push_image_url(&mut self, url: impl Into<String>) {
        self.input.push(serde_json::json!({
            "type": "image",
            "url": url.into(),
            "detail": null
        }));
    }

    pub fn push_local_audio(&mut self, path: impl Into<String>) {
        self.input.push(serde_json::json!({
            "type": "localAudio",
            "path": path.into()
        }));
    }

    pub fn push_audio_url(&mut self, url: impl Into<String>) {
        self.input.push(serde_json::json!({
            "type": "audio",
            "url": url.into()
        }));
    }

    pub fn push_skill(&mut self, name: impl Into<String>, path: impl Into<String>) {
        self.input.push(serde_json::json!({
            "type": "skill",
            "name": name.into(),
            "path": path.into()
        }));
    }

    pub fn push_mention(&mut self, name: impl Into<String>, path: impl Into<String>) {
        self.input.push(serde_json::json!({
            "type": "mention",
            "name": name.into(),
            "path": path.into()
        }));
    }
}

/// Schema-exact Codex collaboration mode object accepted by `turn/start`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationMode {
    pub mode: ModeKind,
    pub settings: CollaborationModeSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollaborationModeSettings {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartResponse {
    /// Full Turn object from the server.
    pub turn: Value,
}

impl TurnStartResponse {
    pub fn turn_id(&self) -> Option<&str> {
        self.turn.get("id").and_then(|v| v.as_str())
    }

    pub fn status(&self) -> Option<&str> {
        self.turn.get("status").and_then(|v| v.as_str())
    }
}

/// Inject input into the currently active turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSteerParams {
    pub thread_id: String,
    pub input: Vec<Value>,
    pub expected_turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_user_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<std::collections::BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsesapi_client_metadata: Option<std::collections::BTreeMap<String, String>>,
}

impl TurnSteerParams {
    pub fn text(
        thread_id: impl Into<String>,
        expected_turn_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            input: vec![user_input_text_value(text)],
            expected_turn_id: expected_turn_id.into(),
            client_user_message_id: None,
            additional_context: None,
            responsesapi_client_metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnSteerResponse {
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCompactStartParams {
    pub thread_id: String,
}

impl ThreadCompactStartParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadCompactStartResponse {}

/// Execute a user-authored command through the loaded thread's configured shell.
///
/// This is Codex's host-local shell escape hatch. It preserves shell syntax and
/// intentionally does not inherit the thread sandbox policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadShellCommandParams {
    pub thread_id: String,
    pub command: String,
}

impl ThreadShellCommandParams {
    pub fn new(thread_id: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            command: command.into(),
        }
    }
}

/// Empty acknowledgement returned after Codex accepts the shell command.
/// Command output and completion continue over thread lifecycle notifications.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadShellCommandResponse {}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewDelivery {
    Inline,
    Detached,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ReviewTarget {
    UncommittedChanges,
    BaseBranch {
        branch: String,
    },
    Commit {
        sha: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Custom {
        instructions: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStartParams {
    pub thread_id: String,
    pub target: ReviewTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ReviewDelivery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStartResponse {
    pub review_thread_id: String,
    pub turn: Value,
}

impl ReviewStartResponse {
    pub fn turn_id(&self) -> Option<&str> {
        self.turn.get("id").and_then(Value::as_str)
    }
}

// ---------------------------------------------------------------------------
// model/list
// ---------------------------------------------------------------------------

/// Params for `model/list` (Codex app-server v2).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListParams {
    /// Opaque pagination cursor returned by a previous call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// When true, include models that are hidden from the default picker list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_hidden: Option<bool>,
    /// Optional page size; defaults to a reasonable server-side value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One reasoning-effort option advertised by a model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningEffortOption {
    pub reasoning_effort: String,
    pub description: String,
}

/// One model-advertised service tier (for example Codex `priority` / Fast).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelServiceTier {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Model catalog entry from `model/list` (UI-facing subset of protocol `Model`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub is_default: bool,
    /// Catalog default reasoning effort (string tag, e.g. `"medium"`).
    #[serde(default)]
    pub default_reasoning_effort: String,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<ReasoningEffortOption>,
    /// Optional accelerated service tiers advertised by this exact model.
    #[serde(default)]
    pub service_tiers: Vec<ModelServiceTier>,
    /// Catalog default tier. `None` means the standard service tier.
    #[serde(default)]
    pub default_service_tier: Option<String>,
    /// Canonical input modality tags advertised by the model.
    #[serde(default = "default_model_input_modalities")]
    pub input_modalities: Vec<String>,
    /// Optional upgrade target model id/slug.
    #[serde(default)]
    pub upgrade: Option<String>,
}

impl ModelInfo {
    /// Chip / picker label (display name, falling back to model slug).
    pub fn label(&self) -> &str {
        if !self.display_name.trim().is_empty() {
            &self.display_name
        } else if !self.model.trim().is_empty() {
            &self.model
        } else {
            &self.id
        }
    }

    /// Best-effort parse from a full server `Model` object.
    pub fn from_value(value: &Value) -> Self {
        let efforts = value
            .get("supportedReasoningEfforts")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| {
                        Some(ReasoningEffortOption {
                            reasoning_effort: e
                                .get("reasoningEffort")
                                .and_then(|v| v.as_str())?
                                .to_string(),
                            description: e
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let input_modalities = value
            .get("inputModalities")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_else(default_model_input_modalities);
        let service_tiers = value
            .get("serviceTiers")
            .and_then(Value::as_array)
            .map(|tiers| {
                tiers
                    .iter()
                    .filter_map(|tier| {
                        Some(ModelServiceTier {
                            id: tier.get("id")?.as_str()?.to_owned(),
                            name: tier.get("name")?.as_str()?.to_owned(),
                            description: tier
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id: value
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            model: value
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            display_name: value
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            description: value
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            hidden: value
                .get("hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_default: value
                .get("isDefault")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            default_reasoning_effort: value
                .get("defaultReasoningEffort")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            supported_reasoning_efforts: efforts,
            service_tiers,
            default_service_tier: value
                .get("defaultServiceTier")
                .and_then(Value::as_str)
                .map(str::to_owned),
            input_modalities,
            upgrade: value
                .get("upgrade")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }
}

fn default_model_input_modalities() -> Vec<String> {
    vec!["text".to_owned()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListResponse {
    pub data: Vec<ModelInfo>,
    /// Opaque cursor for the next page; `None` when exhausted.
    #[serde(default)]
    pub next_cursor: Option<String>,
}

impl ModelListResponse {
    pub fn models(&self) -> &[ModelInfo] {
        &self.data
    }

    /// Default model, or first non-hidden entry, or first entry.
    pub fn default_model(&self) -> Option<&ModelInfo> {
        self.data
            .iter()
            .find(|m| m.is_default && !m.hidden)
            .or_else(|| self.data.iter().find(|m| !m.hidden))
            .or_else(|| self.data.first())
    }
}

/// Offline/demo catalog used by the fixture backend (no paid API).
pub fn fixture_demo_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "gpt-5-demo".into(),
            model: "gpt-5".into(),
            // Bar-style chip label (fixture stub — not a live paid model name).
            display_name: "5.6 Sol Ultra".into(),
            description: "Offline demo model · fixture mode (no paid API)".into(),
            hidden: false,
            is_default: true,
            default_reasoning_effort: "medium".into(),
            supported_reasoning_efforts: vec![
                ReasoningEffortOption {
                    reasoning_effort: "low".into(),
                    description: "Faster, lighter reasoning".into(),
                },
                ReasoningEffortOption {
                    reasoning_effort: "medium".into(),
                    description: "Balanced (default)".into(),
                },
                ReasoningEffortOption {
                    reasoning_effort: "high".into(),
                    description: "Deeper reasoning".into(),
                },
            ],
            service_tiers: vec![ModelServiceTier {
                id: "priority".into(),
                name: "Fast".into(),
                description: "Faster fixture streaming".into(),
            }],
            default_service_tier: None,
            input_modalities: vec!["text".into(), "image".into()],
            upgrade: None,
        },
        ModelInfo {
            id: "o3-demo".into(),
            model: "o3".into(),
            display_name: "o3 (demo)".into(),
            description: "Offline demo reasoning model · fixture mode".into(),
            hidden: false,
            is_default: false,
            default_reasoning_effort: "high".into(),
            supported_reasoning_efforts: vec![ReasoningEffortOption {
                reasoning_effort: "high".into(),
                description: "Deep reasoning".into(),
            }],
            service_tiers: Vec::new(),
            default_service_tier: None,
            input_modalities: vec!["text".into(), "image".into()],
            upgrade: None,
        },
        ModelInfo {
            id: "local-codex-demo".into(),
            model: "local-codex".into(),
            display_name: "local codex (demo)".into(),
            description: "Local / offline codex-shaped demo entry".into(),
            hidden: false,
            is_default: false,
            default_reasoning_effort: "medium".into(),
            supported_reasoning_efforts: vec![ReasoningEffortOption {
                reasoning_effort: "medium".into(),
                description: "Default".into(),
            }],
            service_tiers: Vec::new(),
            default_service_tier: None,
            input_modalities: vec!["text".into()],
            upgrade: None,
        },
    ]
}

// ---------------------------------------------------------------------------
// config/read
// ---------------------------------------------------------------------------

/// Params for `config/read` (effective config layers).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigReadParams {
    /// Optional cwd used to resolve project config layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// When true, include layer stack in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_layers: Option<bool>,
}

/// Response for `config/read`. Full `Config` schema is large — keep as JSON Value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigReadResponse {
    /// Effective merged config object.
    pub config: Value,
    /// Optional ordered config layers when `includeLayers` was set.
    #[serde(default)]
    pub layers: Option<Vec<Value>>,
    /// Map of config key path → origin layer metadata.
    #[serde(default)]
    pub origins: Value,
}

impl ConfigReadResponse {
    /// Best-effort model slug from `config.model`.
    pub fn model(&self) -> Option<&str> {
        self.config.get("model").and_then(|v| v.as_str())
    }

    /// Best-effort approval policy string from `config.approval_policy`.
    pub fn approval_policy(&self) -> Option<&str> {
        self.config.get("approval_policy").and_then(|v| v.as_str())
    }

    /// Short multi-line snippet for Settings UI (model, sandbox, provider).
    pub fn settings_snippet(&self) -> String {
        let mut lines = Vec::new();
        if let Some(m) = self.model() {
            lines.push(format!("model: {m}"));
        }
        if let Some(p) = self.config.get("model_provider").and_then(|v| v.as_str()) {
            lines.push(format!("model_provider: {p}"));
        }
        if let Some(a) = self.approval_policy() {
            lines.push(format!("approval_policy: {a}"));
        }
        if let Some(s) = self.config.get("sandbox_mode").and_then(|v| v.as_str()) {
            lines.push(format!("sandbox_mode: {s}"));
        }
        if let Some(w) = self
            .config
            .get("model_reasoning_effort")
            .and_then(|v| v.as_str())
        {
            lines.push(format!("model_reasoning_effort: {w}"));
        }
        if lines.is_empty() {
            // Fall back to compact pretty JSON (capped).
            let raw = serde_json::to_string_pretty(&self.config).unwrap_or_else(|_| "{}".into());
            let capped: String = raw.chars().take(280).collect();
            if raw.chars().count() > 280 {
                format!("{capped}…")
            } else {
                capped
            }
        } else {
            lines.join("\n")
        }
    }
}

/// Offline demo config used by the fixture backend.
pub fn fixture_demo_config() -> ConfigReadResponse {
    ConfigReadResponse {
        config: serde_json::json!({
            "model": "gpt-5",
            "model_provider": "fixture",
            "approval_policy": "on-request",
            "sandbox_mode": "workspace-write",
            "model_reasoning_effort": "medium",
            "model_reasoning_summary": "auto",
        }),
        layers: Some(vec![serde_json::json!({
            "name": { "type": "user", "file": "/tmp/mitsuro-fixture-home/config.toml" },
            "version": "fixture-1",
            "config": { "model": "gpt-5" },
        })]),
        origins: serde_json::json!({
            "model": {
                "name": { "type": "user", "file": "/tmp/mitsuro-fixture-home/config.toml" },
                "version": "fixture-1"
            }
        }),
    }
}

// ---------------------------------------------------------------------------
// thread/search
// ---------------------------------------------------------------------------

/// Params for `thread/search` (full-text / substring thread search).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchParams {
    /// Required substring / full-text query.
    pub search_term: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// `"asc"` | `"desc"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_direction: Option<String>,
    /// `"created_at"` | `"updated_at"` | `"recency_at"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kinds: Option<Vec<String>>,
}

impl ThreadSearchParams {
    pub fn new(search_term: impl Into<String>) -> Self {
        Self {
            search_term: search_term.into(),
            archived: None,
            cursor: None,
            limit: None,
            sort_direction: None,
            sort_key: None,
            source_kinds: None,
        }
    }
}

/// One hit from `thread/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchResult {
    pub snippet: String,
    pub thread: Value,
}

impl ThreadSearchResult {
    pub fn summary(&self) -> ThreadSummary {
        ThreadSummary::from_value(&self.thread)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchResponse {
    pub data: Vec<ThreadSearchResult>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub backwards_cursor: Option<String>,
}

impl ThreadSearchResponse {
    pub fn threads(&self) -> Vec<ThreadSummary> {
        self.data.iter().map(|r| r.summary()).collect()
    }
}

// ---------------------------------------------------------------------------
// thread/name/set
// ---------------------------------------------------------------------------

/// Params for `thread/name/set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSetNameParams {
    pub thread_id: String,
    pub name: String,
}

impl ThreadSetNameParams {
    pub fn new(thread_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            name: name.into(),
        }
    }
}

/// Empty object response for `thread/name/set`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSetNameResponse {}

// ---------------------------------------------------------------------------
// thread/archive
// ---------------------------------------------------------------------------

/// Params for `thread/archive`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadArchiveParams {
    pub thread_id: String,
}

impl ThreadArchiveParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }
}

/// Empty object response for `thread/archive`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadArchiveResponse {}

// ---------------------------------------------------------------------------
// thread/unarchive
// ---------------------------------------------------------------------------

/// Params for `thread/unarchive`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadUnarchiveParams {
    pub thread_id: String,
}

impl ThreadUnarchiveParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }
}

/// Response for `thread/unarchive` — returns the restored thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadUnarchiveResponse {
    pub thread: Value,
}

impl ThreadUnarchiveResponse {
    pub fn summary(&self) -> ThreadSummary {
        ThreadSummary::from_value(&self.thread)
    }
}

// ---------------------------------------------------------------------------
// thread/delete
// ---------------------------------------------------------------------------

/// Params for `thread/delete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDeleteParams {
    pub thread_id: String,
}

impl ThreadDeleteParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }
}

/// Empty object response for `thread/delete`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDeleteResponse {}

// ---------------------------------------------------------------------------
// thread/fork
// ---------------------------------------------------------------------------

/// Params for `thread/fork`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadForkParams {
    pub thread_id: String,
    /// Optional last turn id to fork through, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_id: Option<String>,
    /// Optional turn id to fork before (exclusive of that turn and later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    /// Double-wrapped so callers can distinguish omission from clearing a
    /// sticky service tier with JSON `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_workspace_roots: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<crate::AskForApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approvals_reviewer: Option<crate::ApprovalsReviewer>,
    /// Unlike turn/start's structured policy, thread/fork accepts a named
    /// sandbox mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<crate::SandboxMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<std::collections::BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_source: Option<String>,
    /// When true, omit `thread.turns` in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_turns: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defer_goal_continuation: Option<bool>,
}

impl ThreadForkParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            last_turn_id: None,
            before_turn_id: None,
            path: None,
            model: None,
            model_provider: None,
            service_tier: None,
            cwd: None,
            runtime_workspace_roots: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox: None,
            permissions: None,
            config: None,
            base_instructions: None,
            developer_instructions: None,
            ephemeral: None,
            thread_source: None,
            exclude_turns: None,
            defer_goal_continuation: None,
        }
    }
}

/// Response for `thread/fork` (lenient subset of full schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadForkResponse {
    pub thread: Value,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

impl ThreadForkResponse {
    pub fn summary(&self) -> ThreadSummary {
        ThreadSummary::from_value(&self.thread)
    }
}

/// Raw Responses API items appended to a thread's model-visible history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInjectItemsParams {
    pub thread_id: String,
    pub items: Vec<Value>,
}

impl ThreadInjectItemsParams {
    pub fn new(thread_id: impl Into<String>, items: Vec<Value>) -> Self {
        Self {
            thread_id: thread_id.into(),
            items,
        }
    }

    /// Construct the hidden user message used to separate inherited fork
    /// history from instructions in a side conversation.
    pub fn input_text_boundary(thread_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(
            thread_id,
            vec![serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": text.into()}]
            })],
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadInjectItemsResponse {}

// ---------------------------------------------------------------------------
// thread/resume
// ---------------------------------------------------------------------------

/// Params for `thread/resume` (subset; prefer `threadId`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeParams {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// When true, omit `thread.turns` in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_turns: Option<bool>,
    /// Optional first `thread/turns/list` page returned atomically with resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_turns_page: Option<ThreadResumeInitialTurnsPageParams>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeInitialTurnsPageParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_direction: Option<crate::ThreadTurnsSortDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_view: Option<crate::ThreadTurnItemsView>,
}

impl ThreadResumeParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            path: None,
            model: None,
            model_provider: None,
            cwd: None,
            exclude_turns: None,
            initial_turns_page: None,
        }
    }
}

/// Response for `thread/resume` (lenient subset of full schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeResponse {
    pub thread: Value,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub active_permission_profile: Option<crate::ActivePermissionProfile>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// Page requested through `initialTurnsPage`, if supported by app-server.
    #[serde(default)]
    pub initial_turns_page: Option<crate::ThreadTurnsListResponse>,
    /// Opaque cursor for reversing from the newest hydrated turn.
    #[serde(default)]
    pub turns_backwards_cursor: Option<String>,
    /// Opaque cursor for reversing from the newest hydrated item.
    #[serde(default)]
    pub items_backwards_cursor: Option<String>,
}

impl ThreadResumeResponse {
    pub fn summary(&self) -> ThreadSummary {
        ThreadSummary::from_value(&self.thread)
    }

    pub fn transcript_messages(&self) -> Vec<TranscriptMessage> {
        extract_transcript_from_thread(&self.thread)
    }
}

// ---------------------------------------------------------------------------
// thread/unsubscribe
// ---------------------------------------------------------------------------

/// Params for releasing an app-server thread subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadUnsubscribeParams {
    pub thread_id: String,
}

impl ThreadUnsubscribeParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }
}

/// Exact status returned by `thread/unsubscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadUnsubscribeStatus {
    NotLoaded,
    NotSubscribed,
    Unsubscribed,
}

/// Response for `thread/unsubscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadUnsubscribeResponse {
    pub status: ThreadUnsubscribeStatus,
}

// ---------------------------------------------------------------------------
// thread/goal/* (get · set · clear)
// ---------------------------------------------------------------------------

/// Wire status for a thread-attached long-running goal (`ThreadGoalStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadGoalStatus {
    #[default]
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
}

impl ThreadGoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::UsageLimited => "usageLimited",
            Self::BudgetLimited => "budgetLimited",
            Self::Complete => "complete",
        }
    }
}

/// Long-running goal attached to a thread (`ThreadGoal` wire type).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoal {
    pub thread_id: String,
    pub objective: String,
    pub status: ThreadGoalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
    pub tokens_used: i64,
    pub time_used_seconds: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ThreadGoal {
    /// New active goal for `thread_id` with the given objective (fixture timestamps).
    pub fn new_active(thread_id: impl Into<String>, objective: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            thread_id: thread_id.into(),
            objective: objective.into(),
            status: ThreadGoalStatus::Active,
            token_budget: None,
            tokens_used: 0,
            time_used_seconds: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Params for `thread/goal/get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalGetParams {
    pub thread_id: String,
}

impl ThreadGoalGetParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }
}

/// Response for `thread/goal/get` — `goal` may be null when unset.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalGetResponse {
    #[serde(default)]
    pub goal: Option<ThreadGoal>,
}

/// Params for `thread/goal/set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalSetParams {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ThreadGoalStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
}

impl ThreadGoalSetParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            objective: None,
            status: None,
            token_budget: None,
        }
    }

    pub fn with_objective(mut self, objective: impl Into<String>) -> Self {
        self.objective = Some(objective.into());
        self
    }

    pub fn with_status(mut self, status: ThreadGoalStatus) -> Self {
        self.status = Some(status);
        self
    }
}

/// Response for `thread/goal/set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalSetResponse {
    pub goal: ThreadGoal,
}

/// Params for `thread/goal/clear`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalClearParams {
    pub thread_id: String,
}

impl ThreadGoalClearParams {
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }
}

/// Response for `thread/goal/clear`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoalClearResponse {
    pub cleared: bool,
}

// ---------------------------------------------------------------------------
// turn/interrupt
// ---------------------------------------------------------------------------

/// Params for `turn/interrupt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptParams {
    pub thread_id: String,
    pub turn_id: String,
}

impl TurnInterruptParams {
    pub fn new(thread_id: impl Into<String>, turn_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
        }
    }
}

/// Empty object response for `turn/interrupt`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptResponse {}

// ---------------------------------------------------------------------------
// skills/list
// ---------------------------------------------------------------------------

/// Params for `skills/list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListParams {
    /// Working directories to scan; empty defaults to session cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwds: Option<Vec<String>>,
    /// When true, bypass skills cache and re-scan from disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_reload: Option<bool>,
}

/// One skill entry from `skills/list` (UI-facing subset of `SkillMetadata`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub path: String,
    /// `"user"` | `"repo"` | `"system"` | `"admin"`.
    pub scope: String,
    #[serde(default)]
    pub short_description: Option<String>,
}

impl SkillMetadata {
    pub fn from_value(value: &Value) -> Self {
        Self {
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            description: value
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            enabled: value
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            path: value
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            scope: value
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string(),
            short_description: value
                .get("shortDescription")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }

    pub fn label(&self) -> &str {
        if let Some(s) = &self.short_description {
            if !s.trim().is_empty() {
                return s.as_str();
            }
        }
        if !self.description.trim().is_empty() {
            &self.description
        } else {
            &self.name
        }
    }
}

/// Per-cwd skills payload from `skills/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListEntry {
    pub cwd: String,
    #[serde(default)]
    pub skills: Vec<SkillMetadata>,
    #[serde(default)]
    pub errors: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListResponse {
    pub data: Vec<SkillsListEntry>,
}

impl SkillsListResponse {
    /// Flatten all skills across cwd entries.
    pub fn all_skills(&self) -> Vec<&SkillMetadata> {
        self.data.iter().flat_map(|e| e.skills.iter()).collect()
    }

    pub fn skill_count(&self) -> usize {
        self.data.iter().map(|e| e.skills.len()).sum()
    }

    pub fn enabled_count(&self) -> usize {
        self.data
            .iter()
            .flat_map(|e| e.skills.iter())
            .filter(|s| s.enabled)
            .count()
    }
}

/// Offline demo skills for the fixture backend.
pub fn fixture_demo_skills() -> SkillsListResponse {
    SkillsListResponse {
        data: vec![SkillsListEntry {
            cwd: "/tmp/mitsuro-fixture".into(),
            skills: vec![
                SkillMetadata {
                    name: "fixture-review".into(),
                    description: "Offline demo skill · code review checklist".into(),
                    enabled: true,
                    path: "/tmp/mitsuro-fixture-home/skills/fixture-review/SKILL.md".into(),
                    scope: "user".into(),
                    short_description: Some("Review checklist".into()),
                },
                SkillMetadata {
                    name: "fixture-docs".into(),
                    description: "Offline demo skill · docs helper".into(),
                    enabled: true,
                    path: "/tmp/mitsuro-fixture-home/skills/fixture-docs/SKILL.md".into(),
                    scope: "user".into(),
                    short_description: Some("Docs helper".into()),
                },
                SkillMetadata {
                    name: "repo-fixture-skill".into(),
                    description: "Repo-scoped demo skill (disabled)".into(),
                    enabled: false,
                    path: "/tmp/mitsuro-fixture/.codex/skills/repo-fixture/SKILL.md".into(),
                    scope: "repo".into(),
                    short_description: Some("Repo skill".into()),
                },
            ],
            errors: vec![],
        }],
    }
}

// ---------------------------------------------------------------------------
// Notification → typed turn stream events
// ---------------------------------------------------------------------------

use crate::types::{ItemKind, TurnStreamEvent};

/// Map a server-originated request into a typed stream event (approvals, etc.).
pub fn map_server_request_to_event(
    id: JsonRpcId,
    method: &str,
    params: Option<&Value>,
) -> TurnStreamEvent {
    if let Some(pending) = crate::approvals::parse_approval_request(id.clone(), method, params) {
        return TurnStreamEvent::ApprovalRequested(pending);
    }
    if let Some(pending) =
        crate::server_requests::parse_user_input_request(id.clone(), method, params)
    {
        return TurnStreamEvent::UserInputRequested(pending);
    }
    if let Some(pending) =
        crate::server_requests::parse_mcp_elicitation_request(id.clone(), method, params)
    {
        return TurnStreamEvent::McpElicitationRequested(pending);
    }
    // Non-approval server requests still surface for forward-compat.
    TurnStreamEvent::Other {
        method: method.to_string(),
        params: Some(serde_json::json!({
            "id": id,
            "params": params.cloned().unwrap_or(Value::Null),
        })),
    }
}

/// Map a raw app-server notification into a typed [`TurnStreamEvent`].
pub fn map_notification_to_event(method: &str, params: Option<&Value>) -> TurnStreamEvent {
    // Live backend rewrites server requests as pseudo-notifications
    // `serverRequest/<method>` with `{ id, params }`.
    if let Some(pending) =
        crate::approvals::pending_from_server_request_notification(method, params)
    {
        return TurnStreamEvent::ApprovalRequested(pending);
    }

    if let Some(real_method) = method.strip_prefix("serverRequest/") {
        if let Some(wrapper) = params {
            if let Some(id) = wrapper
                .get("id")
                .cloned()
                .and_then(|id| serde_json::from_value(id).ok())
            {
                return map_server_request_to_event(id, real_method, wrapper.get("params"));
            }
        }
    }

    let p = params.cloned().unwrap_or(Value::Null);
    match method {
        "turn/started" => {
            let thread_id = str_field(&p, "threadId").unwrap_or_default();
            let turn = p.get("turn").cloned();
            let turn_id = turn
                .as_ref()
                .and_then(|t| t.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            TurnStreamEvent::TurnStarted {
                thread_id,
                turn_id,
                turn,
            }
        }
        "turn/completed" => {
            let thread_id = str_field(&p, "threadId").unwrap_or_default();
            let turn = p.get("turn").cloned();
            let turn_id = turn
                .as_ref()
                .and_then(|t| t.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = turn
                .as_ref()
                .and_then(|t| t.get("status"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            TurnStreamEvent::TurnCompleted {
                thread_id,
                turn_id,
                status,
                turn,
            }
        }
        "item/started" => {
            let thread_id = str_field(&p, "threadId").unwrap_or_default();
            let turn_id = str_field(&p, "turnId").unwrap_or_default();
            let item = p.get("item").cloned();
            let (item_id, kind) = item_id_and_kind(item.as_ref());
            TurnStreamEvent::ItemStarted {
                thread_id,
                turn_id,
                item_id,
                kind,
                item,
            }
        }
        "item/completed" => {
            let thread_id = str_field(&p, "threadId").unwrap_or_default();
            let turn_id = str_field(&p, "turnId").unwrap_or_default();
            let item = p.get("item").cloned();
            let (item_id, kind) = item_id_and_kind(item.as_ref());
            let text = item.as_ref().and_then(item_text);
            TurnStreamEvent::ItemCompleted {
                thread_id,
                turn_id,
                item_id,
                kind,
                text,
                item,
            }
        }
        "item/agentMessage/delta" => TurnStreamEvent::AgentMessageDelta {
            thread_id: str_field(&p, "threadId").unwrap_or_default(),
            turn_id: str_field(&p, "turnId").unwrap_or_default(),
            item_id: str_field(&p, "itemId").unwrap_or_default(),
            delta: str_field(&p, "delta").unwrap_or_default(),
        },
        "item/reasoning/textDelta" => TurnStreamEvent::ReasoningTextDelta {
            thread_id: str_field(&p, "threadId").unwrap_or_default(),
            turn_id: str_field(&p, "turnId").unwrap_or_default(),
            item_id: str_field(&p, "itemId").unwrap_or_default(),
            content_index: p.get("contentIndex").and_then(|v| v.as_i64()),
            delta: str_field(&p, "delta").unwrap_or_default(),
        },
        "item/reasoning/summaryTextDelta" => TurnStreamEvent::ReasoningSummaryDelta {
            thread_id: str_field(&p, "threadId").unwrap_or_default(),
            turn_id: str_field(&p, "turnId").unwrap_or_default(),
            item_id: str_field(&p, "itemId").unwrap_or_default(),
            summary_index: p.get("summaryIndex").and_then(|v| v.as_i64()),
            delta: str_field(&p, "delta").unwrap_or_default(),
        },
        "item/plan/delta" => TurnStreamEvent::PlanDelta {
            thread_id: str_field(&p, "threadId").unwrap_or_default(),
            turn_id: str_field(&p, "turnId").unwrap_or_default(),
            item_id: str_field(&p, "itemId").unwrap_or_default(),
            delta: str_field(&p, "delta").unwrap_or_default(),
        },
        "item/commandExecution/outputDelta" => TurnStreamEvent::CommandExecutionOutputDelta {
            thread_id: str_field(&p, "threadId").unwrap_or_default(),
            turn_id: str_field(&p, "turnId").unwrap_or_default(),
            item_id: str_field(&p, "itemId").unwrap_or_default(),
            delta: str_field(&p, "delta").unwrap_or_default(),
        },
        "item/fileChange/outputDelta" => TurnStreamEvent::FileChangeOutputDelta {
            thread_id: str_field(&p, "threadId").unwrap_or_default(),
            turn_id: str_field(&p, "turnId").unwrap_or_default(),
            item_id: str_field(&p, "itemId").unwrap_or_default(),
            delta: str_field(&p, "delta").unwrap_or_default(),
        },
        "item/fileChange/patchUpdated" => TurnStreamEvent::FileChangePatchUpdated {
            thread_id: str_field(&p, "threadId").unwrap_or_default(),
            turn_id: str_field(&p, "turnId").unwrap_or_default(),
            item_id: str_field(&p, "itemId").unwrap_or_default(),
            changes: p
                .get("changes")
                .cloned()
                .unwrap_or_else(|| Value::Array(vec![])),
        },
        "process/outputDelta" => {
            let (process_handle, stream, delta_base64, delta, cap_reached) =
                crate::process::parse_process_output_delta(Some(&p));
            TurnStreamEvent::ProcessOutputDelta {
                process_handle,
                stream,
                delta_base64,
                delta,
                cap_reached,
            }
        }
        "process/exited" => {
            let (process_handle, exit_code, stdout, stdout_cap_reached, stderr, stderr_cap_reached) =
                crate::process::parse_process_exited(Some(&p));
            TurnStreamEvent::ProcessExited {
                process_handle,
                exit_code,
                stdout,
                stdout_cap_reached,
                stderr,
                stderr_cap_reached,
            }
        }
        other => crate::notifications::known_notification_event(
            other,
            if p.is_null() { None } else { Some(&p) },
        )
        .unwrap_or_else(|| TurnStreamEvent::Other {
            method: other.to_string(),
            params: if p.is_null() { None } else { Some(p) },
        }),
    }
}

/// Parse a JSONL notification **or server-request** line into a typed stream event.
///
/// Fixture streams may interleave normal notifications with approval server requests
/// (`id` + `method` + `params`).
pub fn parse_notification_line(line: &str) -> Result<TurnStreamEvent, serde_json::Error> {
    let msg = JsonRpcMessage::parse_line(line)?;
    match msg {
        JsonRpcMessage::Notification(n) => {
            Ok(map_notification_to_event(&n.method, n.params.as_ref()))
        }
        JsonRpcMessage::ServerRequest { id, method, params } => {
            Ok(map_server_request_to_event(id, &method, params.as_ref()))
        }
        _ => {
            // Also accept bare params objects wrapped as notifications without method —
            // treat unknown as Other.
            let value: Value = serde_json::from_str(line)?;
            if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
                Ok(map_notification_to_event(method, value.get("params")))
            } else {
                Ok(TurnStreamEvent::Other {
                    method: "unknown".into(),
                    params: Some(value),
                })
            }
        }
    }
}

/// Parse a multi-line JSONL fixture into typed events (skips blank / comment lines).
pub fn parse_fixture_jsonl(content: &str) -> std::result::Result<Vec<TurnStreamEvent>, String> {
    let mut events = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_notification_line(line) {
            Ok(ev) => events.push(ev),
            Err(e) => {
                return Err(format!("fixture line {}: {e}", idx + 1));
            }
        }
    }
    Ok(events)
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

fn item_id_and_kind(item: Option<&Value>) -> (String, ItemKind) {
    let Some(item) = item else {
        return (String::new(), ItemKind::Other("unknown".into()));
    };
    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let kind = item
        .get("type")
        .and_then(|v| v.as_str())
        .map(ItemKind::from_type_str)
        .unwrap_or(ItemKind::Other("unknown".into()));
    (id, kind)
}

fn item_text(item: &Value) -> Option<String> {
    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
        return Some(t.to_string());
    }
    if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
        let joined = summary
            .iter()
            .filter_map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    // commandExecution: prefer aggregated output for completed display fallback
    if item.get("type").and_then(|t| t.as_str()) == Some("commandExecution") {
        if let Some(out) = item.get("aggregatedOutput").and_then(|v| v.as_str()) {
            if !out.is_empty() {
                return Some(out.to_string());
            }
        }
    }
    // fileChange: join unified diffs
    if item.get("type").and_then(|t| t.as_str()) == Some("fileChange") {
        let (_, patch) = summarize_file_changes(item.get("changes"));
        if !patch.is_empty() {
            return Some(patch);
        }
    }
    None
}

/// Extract command / cwd / status / aggregatedOutput from a `commandExecution` item.
pub fn command_execution_fields(item: &Value) -> CommandExecutionFields {
    CommandExecutionFields {
        command: item
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        cwd: item
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        status: item
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("inProgress")
            .to_string(),
        output: item
            .get("aggregatedOutput")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// UI-facing subset of a `commandExecution` ThreadItem.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandExecutionFields {
    pub command: String,
    pub cwd: String,
    pub status: String,
    pub output: String,
}

/// Extract paths summary / patch preview / status from a `fileChange` item.
pub fn file_change_fields(item: &Value) -> FileChangeFields {
    let status = item
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("inProgress")
        .to_string();
    let (paths_summary, patch_preview) = summarize_file_changes(item.get("changes"));
    FileChangeFields {
        paths_summary,
        patch_preview,
        status,
    }
}

/// UI-facing subset of a `fileChange` ThreadItem.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileChangeFields {
    pub paths_summary: String,
    pub patch_preview: String,
    pub status: String,
}

/// Summarize a `FileUpdateChange[]` array into path list + joined unified diffs.
pub fn summarize_file_changes(changes: Option<&Value>) -> (String, String) {
    let Some(arr) = changes.and_then(|c| c.as_array()) else {
        return (String::new(), String::new());
    };
    let paths: Vec<String> = arr
        .iter()
        .filter_map(|c| c.get("path").and_then(|p| p.as_str()).map(str::to_string))
        .collect();
    let patch = arr
        .iter()
        .filter_map(|c| c.get("diff").and_then(|d| d.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    let paths_summary = match paths.len() {
        0 => String::new(),
        1 => paths[0].clone(),
        n => format!("{n} files · {}", paths.join(", ")),
    };
    (paths_summary, patch)
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use crate::types::TurnStreamEvent;

    #[test]
    fn maps_agent_message_delta() {
        let line = r#"{"method":"item/agentMessage/delta","params":{"threadId":"t1","turnId":"u1","itemId":"i1","delta":"Hi"},"emittedAtMs":1}"#;
        let ev = parse_notification_line(line).unwrap();
        match ev {
            TurnStreamEvent::AgentMessageDelta {
                thread_id,
                item_id,
                delta,
                ..
            } => {
                assert_eq!(thread_id, "t1");
                assert_eq!(item_id, "i1");
                assert_eq!(delta, "Hi");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn maps_turn_started_and_completed() {
        let start = r#"{"method":"turn/started","params":{"threadId":"t1","turn":{"id":"turn-1","status":"inProgress","items":[],"itemsView":"full","error":null,"startedAt":1,"completedAt":null,"durationMs":null}}}"#;
        let ev = parse_notification_line(start).unwrap();
        assert!(matches!(
            ev,
            TurnStreamEvent::TurnStarted {
                turn_id,
                ..
            } if turn_id == "turn-1"
        ));

        let done = r#"{"method":"turn/completed","params":{"threadId":"t1","turn":{"id":"turn-1","status":"completed","items":[],"itemsView":"full","error":null,"startedAt":1,"completedAt":2,"durationMs":1000}}}"#;
        let ev = parse_notification_line(done).unwrap();
        assert!(matches!(
            ev,
            TurnStreamEvent::TurnCompleted {
                status: Some(ref s),
                ..
            } if s == "completed"
        ));
    }

    #[test]
    fn maps_exec_command_approval_server_request() {
        let line = r#"{"id":3,"method":"execCommandApproval","params":{"conversationId":"c1","callId":"call-1","approvalId":null,"command":["uname","-a"],"cwd":"/tmp","reason":null,"parsedCmd":[{"type":"unknown","cmd":"uname -a"}]}}"#;
        let ev = parse_notification_line(line).unwrap();
        match ev {
            TurnStreamEvent::ApprovalRequested(p) => {
                assert_eq!(p.kind, crate::approvals::ApprovalKind::ExecCommand);
                assert_eq!(p.summary, "uname -a");
                assert_eq!(p.request_id, JsonRpcId::Number(3));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn maps_server_request_pseudo_notification() {
        let line = r#"{"method":"serverRequest/item/commandExecution/requestApproval","params":{"id":11,"params":{"itemId":"i1","threadId":"t1","turnId":"u1","startedAtMs":9,"command":"echo ok","cwd":"/tmp"}}}"#;
        let ev = parse_notification_line(line).unwrap();
        match ev {
            TurnStreamEvent::ApprovalRequested(p) => {
                assert_eq!(p.kind, crate::approvals::ApprovalKind::CommandExecution);
                assert_eq!(p.summary, "echo ok");
                assert_eq!(p.request_id, JsonRpcId::Number(11));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn maps_command_execution_item_started_completed_and_output_delta() {
        let started = r#"{"method":"item/started","params":{"threadId":"t1","turnId":"u1","item":{"type":"commandExecution","id":"cmd-1","command":"echo hi","cwd":"/tmp","status":"inProgress","aggregatedOutput":null}}}"#;
        let ev = parse_notification_line(started).unwrap();
        match ev {
            TurnStreamEvent::ItemStarted {
                item_id,
                kind,
                item,
                ..
            } => {
                assert_eq!(item_id, "cmd-1");
                assert_eq!(kind, crate::types::ItemKind::CommandExecution);
                let fields = command_execution_fields(item.as_ref().unwrap());
                assert_eq!(fields.command, "echo hi");
                assert_eq!(fields.cwd, "/tmp");
                assert_eq!(fields.status, "inProgress");
            }
            other => panic!("unexpected {other:?}"),
        }

        let delta = r#"{"method":"item/commandExecution/outputDelta","params":{"threadId":"t1","turnId":"u1","itemId":"cmd-1","delta":"hi\n"}}"#;
        let ev = parse_notification_line(delta).unwrap();
        match ev {
            TurnStreamEvent::CommandExecutionOutputDelta { item_id, delta, .. } => {
                assert_eq!(item_id, "cmd-1");
                assert_eq!(delta, "hi\n");
            }
            other => panic!("unexpected {other:?}"),
        }

        let done = r#"{"method":"item/completed","params":{"threadId":"t1","turnId":"u1","item":{"type":"commandExecution","id":"cmd-1","command":"echo hi","cwd":"/tmp","status":"completed","aggregatedOutput":"hi\n","exitCode":0}}}"#;
        let ev = parse_notification_line(done).unwrap();
        match ev {
            TurnStreamEvent::ItemCompleted {
                item_id,
                kind,
                text,
                item,
                ..
            } => {
                assert_eq!(item_id, "cmd-1");
                assert_eq!(kind, crate::types::ItemKind::CommandExecution);
                assert_eq!(text.as_deref(), Some("hi\n"));
                let fields = command_execution_fields(item.as_ref().unwrap());
                assert_eq!(fields.status, "completed");
                assert_eq!(fields.output, "hi\n");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn maps_file_change_item_patch_updated_and_output_delta() {
        let started = r#"{"method":"item/started","params":{"threadId":"t1","turnId":"u1","item":{"type":"fileChange","id":"fc-1","changes":[],"status":"inProgress"}}}"#;
        let ev = parse_notification_line(started).unwrap();
        assert!(matches!(
            ev,
            TurnStreamEvent::ItemStarted {
                kind: crate::types::ItemKind::FileChange,
                item_id,
                ..
            } if item_id == "fc-1"
        ));

        let patch = r#"{"method":"item/fileChange/patchUpdated","params":{"threadId":"t1","turnId":"u1","itemId":"fc-1","changes":[{"path":"src/hello.rs","kind":{"type":"add"},"diff":"@@ -0,0 +1,3 @@\n+fn main() {\n+    println!(\"hi\");\n+}\n"}]}}"#;
        let ev = parse_notification_line(patch).unwrap();
        match ev {
            TurnStreamEvent::FileChangePatchUpdated {
                item_id, changes, ..
            } => {
                assert_eq!(item_id, "fc-1");
                let (paths, diff) = summarize_file_changes(Some(&changes));
                assert_eq!(paths, "src/hello.rs");
                assert!(diff.contains("println!"));
            }
            other => panic!("unexpected {other:?}"),
        }

        let out_delta = r#"{"method":"item/fileChange/outputDelta","params":{"threadId":"t1","turnId":"u1","itemId":"fc-1","delta":"+fn main()\n"}}"#;
        let ev = parse_notification_line(out_delta).unwrap();
        match ev {
            TurnStreamEvent::FileChangeOutputDelta { item_id, delta, .. } => {
                assert_eq!(item_id, "fc-1");
                assert!(delta.starts_with("+fn"));
            }
            other => panic!("unexpected {other:?}"),
        }

        let done = r#"{"method":"item/completed","params":{"threadId":"t1","turnId":"u1","item":{"type":"fileChange","id":"fc-1","status":"completed","changes":[{"path":"src/hello.rs","kind":{"type":"add"},"diff":"@@ -0,0 +1,3 @@\n+fn main() {\n+    println!(\"hi\");\n+}\n"}]}}}"#;
        let ev = parse_notification_line(done).unwrap();
        match ev {
            TurnStreamEvent::ItemCompleted {
                kind, text, item, ..
            } => {
                assert_eq!(kind, crate::types::ItemKind::FileChange);
                assert!(text.as_ref().is_some_and(|t| t.contains("println!")));
                let fields = file_change_fields(item.as_ref().unwrap());
                assert_eq!(fields.status, "completed");
                assert_eq!(fields.paths_summary, "src/hello.rs");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn maps_process_output_delta_and_exited() {
        // "hello\n" base64
        let delta_line = r#"{"method":"process/outputDelta","params":{"processHandle":"ph-1","stream":"stdout","deltaBase64":"aGVsbG8K","capReached":false},"emittedAtMs":1}"#;
        let ev = parse_notification_line(delta_line).unwrap();
        assert_eq!(ev.method_name(), "process/outputDelta");
        match &ev {
            TurnStreamEvent::ProcessOutputDelta {
                process_handle,
                stream,
                delta,
                delta_base64,
                cap_reached,
            } => {
                assert_eq!(process_handle, "ph-1");
                assert_eq!(*stream, crate::process::ProcessOutputStream::Stdout);
                assert_eq!(delta, "hello\n");
                assert_eq!(delta_base64, "aGVsbG8K");
                assert!(!cap_reached);
            }
            other => panic!("unexpected {other:?}"),
        }

        let exit_line = r#"{"method":"process/exited","params":{"processHandle":"ph-1","exitCode":0,"stdout":"","stdoutCapReached":false,"stderr":"","stderrCapReached":false}}"#;
        let ev = parse_notification_line(exit_line).unwrap();
        assert_eq!(ev.method_name(), "process/exited");
        match &ev {
            TurnStreamEvent::ProcessExited {
                process_handle,
                exit_code,
                stdout,
                stderr,
                ..
            } => {
                assert_eq!(process_handle, "ph-1");
                assert_eq!(*exit_code, 0);
                assert!(stdout.is_empty());
                assert!(stderr.is_empty());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn extract_transcript_maps_command_execution_and_file_change() {
        let thread = serde_json::json!({
            "id": "thr-1",
            "turns": [{
                "id": "turn-1",
                "items": [
                    {
                        "type": "userMessage",
                        "id": "u1",
                        "content": [{"type": "text", "text": "hello"}]
                    },
                    {
                        "type": "agentMessage",
                        "id": "a1",
                        "text": "running tools"
                    },
                    {
                        "type": "commandExecution",
                        "id": "cmd-1",
                        "command": "ls -la",
                        "cwd": "/tmp",
                        "status": "completed",
                        "aggregatedOutput": "file.txt\n"
                    },
                    {
                        "type": "fileChange",
                        "id": "fc-1",
                        "status": "completed",
                        "changes": [{
                            "path": "src/hello.rs",
                            "kind": {"type": "add"},
                            "diff": "@@ -0,0 +1 @@\n+fn main() {}\n"
                        }]
                    },
                    {
                        "type": "plan",
                        "id": "p1",
                        "text": "1. done"
                    }
                ]
            }]
        });
        let msgs = extract_transcript_from_thread(&thread);
        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[0].role, TranscriptRole::User);
        assert_eq!(msgs[1].role, TranscriptRole::Assistant);
        assert_eq!(msgs[2].role, TranscriptRole::CommandExecution);
        let cmd = msgs[2].command.as_ref().expect("command fields");
        assert_eq!(cmd.command, "ls -la");
        assert_eq!(cmd.cwd, "/tmp");
        assert_eq!(cmd.status, "completed");
        assert_eq!(cmd.output, "file.txt\n");
        assert_eq!(msgs[2].item_id.as_deref(), Some("cmd-1"));
        assert_eq!(msgs[3].role, TranscriptRole::FileChange);
        let fc = msgs[3].file_change.as_ref().expect("file change fields");
        assert_eq!(fc.paths_summary, "src/hello.rs");
        assert!(fc.patch_preview.contains("fn main"));
        assert_eq!(fc.status, "completed");
        assert_eq!(msgs[4].role, TranscriptRole::Plan);
    }

    #[test]
    fn extract_transcript_preserves_local_remote_and_embedded_user_images() {
        let thread = serde_json::json!({
            "turns": [{
                "items": [{
                    "id": "user-images",
                    "type": "userMessage",
                    "content": [
                        {"type": "text", "text": "compare these"},
                        {"type": "localImage", "path": "/tmp/local.png", "detail": null},
                        {"type": "image", "url": "https://example.com/remote.webp", "detail": null},
                        {"type": "image", "url": "data:image/png;base64,cG5n", "detail": null}
                    ]
                }]
            }]
        });
        let messages = extract_transcript_from_thread(&thread);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "compare these");
        assert_eq!(
            messages[0].images,
            vec![
                TranscriptImage {
                    source: TranscriptImageSource::LocalPath("/tmp/local.png".to_owned())
                },
                TranscriptImage {
                    source: TranscriptImageSource::Url(
                        "https://example.com/remote.webp".to_owned()
                    )
                },
                TranscriptImage {
                    source: TranscriptImageSource::Embedded {
                        media_type: "image/png".to_owned(),
                        data: "cG5n".to_owned()
                    }
                }
            ]
        );
    }

    #[test]
    fn extract_transcript_preserves_local_remote_and_embedded_user_audio() {
        let thread = serde_json::json!({
            "turns": [{
                "items": [{
                    "id": "user-audio",
                    "type": "userMessage",
                    "content": [
                        {"type": "localAudio", "path": "/tmp/local.wav"},
                        {"type": "audio", "url": "https://example.com/remote.mp3"},
                        {"type": "audio", "url": "data:audio/ogg;base64,b2dn"}
                    ]
                }]
            }]
        });
        let messages = extract_transcript_from_thread(&thread);
        assert_eq!(messages.len(), 1, "audio-only user messages remain visible");
        assert!(messages[0].body.is_empty());
        assert_eq!(
            messages[0].audio,
            vec![
                TranscriptAudio {
                    source: TranscriptAudioSource::LocalPath("/tmp/local.wav".to_owned())
                },
                TranscriptAudio {
                    source: TranscriptAudioSource::Url("https://example.com/remote.mp3".to_owned())
                },
                TranscriptAudio {
                    source: TranscriptAudioSource::Embedded {
                        media_type: "audio/ogg".to_owned(),
                        data: "b2dn".to_owned()
                    }
                }
            ]
        );
    }

    #[test]
    fn extract_transcript_preserves_skill_and_mention_user_inputs() {
        let thread = serde_json::json!({
            "turns": [{
                "items": [{
                    "id": "user-references",
                    "type": "userMessage",
                    "content": [
                        {"type": "skill", "name": "release", "path": "/skills/release/SKILL.md"},
                        {"type": "mention", "name": "Cargo.toml", "path": "/workspace/Cargo.toml"}
                    ]
                }]
            }]
        });
        let messages = extract_transcript_from_thread(&thread);
        assert_eq!(
            messages.len(),
            1,
            "reference-only user messages remain visible"
        );
        assert!(messages[0].body.is_empty());
        assert_eq!(
            messages[0].references,
            vec![
                TranscriptReference {
                    kind: TranscriptReferenceKind::Skill,
                    name: "release".to_owned(),
                    path: "/skills/release/SKILL.md".to_owned(),
                },
                TranscriptReference {
                    kind: TranscriptReferenceKind::Mention,
                    name: "Cargo.toml".to_owned(),
                    path: "/workspace/Cargo.toml".to_owned(),
                }
            ]
        );
    }

    #[test]
    fn every_current_thread_item_kind_is_typed_and_round_trips() {
        let inventory = include_str!("../fixtures/thread-item-types.txt");
        let kinds = inventory.lines().filter(|line| !line.is_empty());
        for wire in kinds {
            let kind = crate::types::ItemKind::from_type_str(wire);
            assert!(
                !matches!(kind, crate::types::ItemKind::Other(_)),
                "current item kind must be typed: {wire}"
            );
            assert_eq!(kind.as_str(), wire);
        }
    }

    #[test]
    fn hydrated_transcript_preserves_non_chat_activity_items() {
        let thread = serde_json::json!({
            "turns": [{
                "items": [
                    {
                        "type": "mcpToolCall",
                        "id": "mcp-1",
                        "server": "github",
                        "tool": "search_issues",
                        "status": "completed",
                        "arguments": {"query": "is:open label:bug"},
                        "appContext": {
                            "resourceUri": "ui://github/issues",
                            "appName": "GitHub issues"
                        },
                        "result": {
                            "structuredContent": {"total": 2}
                        }
                    },
                    {
                        "type": "webSearch",
                        "id": "search-1",
                        "query": "Codex app-server protocol"
                    },
                    {
                        "type": "subAgentActivity",
                        "id": "agent-1",
                        "agentPath": "/root/reviewer",
                        "agentThreadId": "thread-2",
                        "kind": "completed"
                    },
                    {
                        "type": "futureItem",
                        "id": "future-1",
                        "status": "inProgress",
                        "value": "kept"
                    }
                ]
            }]
        });

        let messages = extract_transcript_from_thread(&thread);
        assert_eq!(messages.len(), 4);
        assert!(messages
            .iter()
            .all(|message| message.role == TranscriptRole::System));
        assert_eq!(messages[0].activity.as_ref().unwrap().kind, "mcpToolCall");
        assert_eq!(messages[0].activity.as_ref().unwrap().title, "MCP tool");
        assert!(messages[0].body.contains("search_issues"));
        let mcp_app = messages[0]
            .activity
            .as_ref()
            .unwrap()
            .mcp_app
            .as_ref()
            .expect("interactive MCP app metadata survives hydration");
        assert_eq!(mcp_app.resource_uri, "ui://github/issues");
        assert_eq!(
            mcp_app.arguments,
            serde_json::json!({"query": "is:open label:bug"})
        );
        assert_eq!(
            mcp_app.result.as_ref().unwrap()["structuredContent"]["total"],
            2
        );
        assert_eq!(messages[1].activity.as_ref().unwrap().title, "Web search");
        assert_eq!(
            messages[2].activity.as_ref().unwrap().title,
            "Subagent activity"
        );
        assert_eq!(messages[3].activity.as_ref().unwrap().kind, "futureItem");
        assert!(messages[3].body.contains("kept"));
    }
}

#[cfg(test)]
mod model_list_tests {
    use super::*;

    #[test]
    fn deserializes_model_list_response_shape() {
        let raw = serde_json::json!({
            "data": [{
                "id": "gpt-5",
                "model": "gpt-5",
                "displayName": "GPT-5",
                "description": "Flagship",
                "hidden": false,
                "isDefault": true,
                "defaultReasoningEffort": "medium",
                "supportedReasoningEfforts": [
                    { "reasoningEffort": "medium", "description": "Balanced" }
                ],
                "upgrade": null,
                "inputModalities": ["text", "image"],
                "supportsPersonality": false,
                "serviceTiers": [{
                    "id": "priority",
                    "name": "Fast",
                    "description": "1.5x speed, increased usage"
                }],
                "defaultServiceTier": null
            }],
            "nextCursor": null
        });
        let resp: ModelListResponse = serde_json::from_value(raw).expect("model list");
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].display_name, "GPT-5");
        assert!(resp.data[0].is_default);
        assert_eq!(resp.data[0].input_modalities, ["text", "image"]);
        assert_eq!(resp.data[0].service_tiers[0].id, "priority");
        assert_eq!(resp.data[0].service_tiers[0].name, "Fast");
        assert_eq!(resp.default_model().unwrap().id, "gpt-5");
    }

    #[test]
    fn model_info_from_value_and_fixture_demo() {
        let v = serde_json::json!({
            "id": "x",
            "model": "x-model",
            "displayName": "X",
            "description": "desc",
            "hidden": false,
            "isDefault": false,
            "defaultReasoningEffort": "low",
            "supportedReasoningEfforts": []
        });
        let m = ModelInfo::from_value(&v);
        assert_eq!(m.label(), "X");
        assert_eq!(m.input_modalities, ["text"]);
        let demo = fixture_demo_models();
        assert!(demo
            .iter()
            .any(|m| m.id == "gpt-5-demo" || m.model.contains("gpt-5")));
    }
}

/// Param / response wire shapes for P9 protocol methods (camelCase per bar json-schema).
#[cfg(test)]
mod p9_protocol_shape_tests {
    use super::*;

    #[test]
    fn turn_start_params_serializes_model_camel_case() {
        let mut p = TurnStartParams::text_with_model("thread-1", "hello", Some("gpt-5".into()));
        p.effort = Some("high".into());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "thread-1");
        assert_eq!(v["model"], "gpt-5");
        assert_eq!(v["effort"], "high");
        assert!(
            v.get("thread_id").is_none(),
            "must not emit snake_case thread_id"
        );
        assert!(v.get("clientUserMessageId").is_none() || v["clientUserMessageId"].is_null());
        // Optional None fields skipped
        assert!(v.get("cwd").is_none());
        let input = v["input"].as_array().expect("input array");
        assert_eq!(input[0]["type"], "text");
        assert_eq!(input[0]["text"], "hello");
    }

    #[test]
    fn turn_start_params_omits_model_when_none() {
        let p = TurnStartParams::text("t", "x");
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("model").is_none());
        assert_eq!(v["threadId"], "t");
    }

    #[test]
    fn turn_start_local_image_matches_generated_user_input_contract() {
        let mut params = TurnStartParams::text("thread-1", "inspect this");
        params.push_local_image("/tmp/screenshot.png");
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["input"][1]["type"], "localImage");
        assert_eq!(value["input"][1]["path"], "/tmp/screenshot.png");
        assert!(value["input"][1]["detail"].is_null());
    }

    #[test]
    fn turn_start_local_audio_matches_generated_user_input_contract() {
        let mut params = TurnStartParams::text("thread-1", "transcribe this");
        params.push_local_audio("/tmp/recording.wav");
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["input"][1]["type"], "localAudio");
        assert_eq!(value["input"][1]["path"], "/tmp/recording.wav");
    }

    #[test]
    fn turn_start_skill_and_mention_match_generated_user_input_contract() {
        let mut params = TurnStartParams::text("thread-1", "use these");
        params.push_skill("release", "/skills/release/SKILL.md");
        params.push_mention("Cargo.toml", "/workspace/Cargo.toml");
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(
            value["input"][1],
            serde_json::json!({
                "type": "skill",
                "name": "release",
                "path": "/skills/release/SKILL.md"
            })
        );
        assert_eq!(
            value["input"][2],
            serde_json::json!({
                "type": "mention",
                "name": "Cargo.toml",
                "path": "/workspace/Cargo.toml"
            })
        );
    }

    #[test]
    fn turn_steer_matches_generated_camel_case_contract() {
        let p = TurnSteerParams::text("thread-1", "turn-9", "change direction");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "thread-1");
        assert_eq!(v["expectedTurnId"], "turn-9");
        assert_eq!(v["input"][0]["type"], "text");
        assert_eq!(v["input"][0]["text"], "change direction");
        assert!(v.get("thread_id").is_none());
        assert!(v.get("additionalContext").is_none());

        let response: TurnSteerResponse =
            serde_json::from_value(serde_json::json!({"turnId": "turn-9"})).unwrap();
        assert_eq!(response.turn_id, "turn-9");
    }

    #[test]
    fn thread_compact_start_matches_generated_contract() {
        let value = serde_json::to_value(ThreadCompactStartParams::new("thread-1")).unwrap();
        assert_eq!(value, serde_json::json!({"threadId": "thread-1"}));
        let _: ThreadCompactStartResponse = serde_json::from_value(serde_json::json!({})).unwrap();
    }

    #[test]
    fn thread_shell_command_matches_generated_contract() {
        let value = serde_json::to_value(ThreadShellCommandParams::new(
            "thread-1",
            "printf 'one\\ntwo\\n' | tail -n 1",
        ))
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "threadId": "thread-1",
                "command": "printf 'one\\ntwo\\n' | tail -n 1"
            })
        );
        assert!(value.get("thread_id").is_none());
        let _: ThreadShellCommandResponse = serde_json::from_value(serde_json::json!({})).unwrap();
    }

    #[test]
    fn review_start_matches_generated_tagged_contract() {
        let params = ReviewStartParams {
            thread_id: "thread-1".to_owned(),
            target: ReviewTarget::UncommittedChanges,
            delivery: Some(ReviewDelivery::Inline),
        };
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(
            value["target"],
            serde_json::json!({"type": "uncommittedChanges"})
        );
        assert_eq!(value["delivery"], "inline");

        let response: ReviewStartResponse = serde_json::from_value(serde_json::json!({
            "reviewThreadId": "thread-1",
            "turn": {"id": "turn-review", "status": "inProgress"}
        }))
        .unwrap();
        assert_eq!(response.turn_id(), Some("turn-review"));
    }

    #[test]
    fn config_read_params_and_response_camel_case() {
        let params = ConfigReadParams {
            cwd: Some("/proj".into()),
            include_layers: Some(true),
        };
        let v = serde_json::to_value(&params).unwrap();
        assert_eq!(v["cwd"], "/proj");
        assert_eq!(v["includeLayers"], true);
        assert!(v.get("include_layers").is_none());

        let raw = serde_json::json!({
            "config": {
                "model": "gpt-5",
                "approval_policy": "on-request",
                "sandbox_mode": "workspace-write",
                "model_provider": "openai"
            },
            "layers": null,
            "origins": {
                "model": {
                    "name": { "type": "user", "file": "/home/u/.codex/config.toml" },
                    "version": "1"
                }
            }
        });
        let resp: ConfigReadResponse = serde_json::from_value(raw).expect("config/read");
        assert_eq!(resp.model(), Some("gpt-5"));
        assert_eq!(resp.approval_policy(), Some("on-request"));
        let snip = resp.settings_snippet();
        assert!(snip.contains("model: gpt-5"));
        assert!(snip.contains("sandbox_mode: workspace-write"));
    }

    #[test]
    fn thread_search_params_and_response_camel_case() {
        let params = ThreadSearchParams {
            search_term: "layout".into(),
            archived: Some(false),
            cursor: None,
            limit: Some(20),
            sort_direction: Some("desc".into()),
            sort_key: Some("updated_at".into()),
            source_kinds: None,
        };
        let v = serde_json::to_value(&params).unwrap();
        assert_eq!(v["searchTerm"], "layout");
        assert_eq!(v["sortDirection"], "desc");
        assert_eq!(v["sortKey"], "updated_at");
        assert!(v.get("search_term").is_none());

        let raw = serde_json::json!({
            "data": [{
                "snippet": "…layout plan…",
                "thread": {
                    "id": "th-1",
                    "name": "Layout",
                    "preview": "layout plan",
                    "cwd": "/tmp",
                    "createdAt": 1,
                    "updatedAt": 2,
                    "modelProvider": "openai",
                    "ephemeral": false,
                    "isPinned": false
                }
            }],
            "nextCursor": null,
            "backwardsCursor": null
        });
        let resp: ThreadSearchResponse = serde_json::from_value(raw).expect("thread/search");
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].snippet, "…layout plan…");
        assert_eq!(resp.threads()[0].id, "th-1");
        assert_eq!(resp.threads()[0].name.as_deref(), Some("Layout"));
    }

    #[test]
    fn thread_set_name_params_camel_case() {
        let p = ThreadSetNameParams::new("thread-xyz", "My title");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "thread-xyz");
        assert_eq!(v["name"], "My title");
        assert!(v.get("thread_id").is_none());

        let resp: ThreadSetNameResponse =
            serde_json::from_value(serde_json::json!({})).expect("empty object");
        let _ = resp;
    }

    #[test]
    fn skills_list_params_and_response_camel_case() {
        let params = SkillsListParams {
            cwds: Some(vec!["/tmp".into()]),
            force_reload: Some(true),
        };
        let v = serde_json::to_value(&params).unwrap();
        assert_eq!(v["forceReload"], true);
        assert_eq!(v["cwds"][0], "/tmp");
        assert!(v.get("force_reload").is_none());

        let raw = serde_json::json!({
            "data": [{
                "cwd": "/tmp",
                "errors": [],
                "skills": [{
                    "name": "review",
                    "description": "Review skill",
                    "enabled": true,
                    "path": "/tmp/skills/review/SKILL.md",
                    "scope": "user",
                    "shortDescription": "Review"
                }]
            }]
        });
        let resp: SkillsListResponse = serde_json::from_value(raw).expect("skills/list");
        assert_eq!(resp.skill_count(), 1);
        assert_eq!(resp.enabled_count(), 1);
        assert_eq!(resp.all_skills()[0].name, "review");
        assert_eq!(
            resp.all_skills()[0].short_description.as_deref(),
            Some("Review")
        );

        let demo = fixture_demo_skills();
        assert!(demo.skill_count() >= 2);
    }

    #[test]
    fn fixture_demo_config_snippet_has_model() {
        let cfg = fixture_demo_config();
        assert_eq!(cfg.model(), Some("gpt-5"));
        assert!(cfg.settings_snippet().contains("model: gpt-5"));
    }
}

/// Param / response wire shapes for P11 thread lifecycle + turn/interrupt.
#[cfg(test)]
mod p11_protocol_shape_tests {
    use super::*;

    #[test]
    fn thread_archive_params_camel_case() {
        let p = ThreadArchiveParams::new("th-1");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "th-1");
        assert!(v.get("thread_id").is_none());
        let _: ThreadArchiveResponse =
            serde_json::from_value(serde_json::json!({})).expect("empty");
    }

    #[test]
    fn thread_unarchive_params_and_response_camel_case() {
        let p = ThreadUnarchiveParams::new("th-2");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "th-2");
        assert!(v.get("thread_id").is_none());

        let raw = serde_json::json!({
            "thread": {
                "id": "th-2",
                "name": "Restored",
                "archived": false,
                "cwd": "/tmp",
                "createdAt": 1,
                "updatedAt": 2,
                "modelProvider": "fixture",
                "ephemeral": true,
                "isPinned": false,
                "turns": []
            }
        });
        let resp: ThreadUnarchiveResponse = serde_json::from_value(raw).expect("thread/unarchive");
        assert_eq!(resp.summary().id, "th-2");
        assert_eq!(resp.summary().name.as_deref(), Some("Restored"));
    }

    #[test]
    fn thread_delete_params_camel_case() {
        let p = ThreadDeleteParams::new("th-del");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "th-del");
        assert!(v.get("thread_id").is_none());
        let _: ThreadDeleteResponse = serde_json::from_value(serde_json::json!({})).expect("empty");
    }

    #[test]
    fn thread_fork_params_camel_case() {
        let mut p = ThreadForkParams::new("th-src");
        p.last_turn_id = Some("turn-3".into());
        p.exclude_turns = Some(true);
        p.model = Some("gpt-5".into());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "th-src");
        assert_eq!(v["lastTurnId"], "turn-3");
        assert_eq!(v["excludeTurns"], true);
        assert_eq!(v["model"], "gpt-5");
        assert!(v.get("thread_id").is_none());
        assert!(v.get("last_turn_id").is_none());
        // Omitted optionals not present
        assert!(v.get("beforeTurnId").is_none());
        assert!(v.get("path").is_none());
    }

    #[test]
    fn side_thread_fork_and_injection_match_generated_contracts() {
        let mut fork = ThreadForkParams::new("th-parent");
        fork.cwd = Some("/workspace".into());
        fork.model = Some("gpt-5.6-sol".into());
        fork.service_tier = Some(None);
        fork.config = Some(std::collections::BTreeMap::from([(
            "model_reasoning_effort".to_owned(),
            serde_json::json!("high"),
        )]));
        fork.ephemeral = Some(true);
        fork.thread_source = Some("user".into());
        fork.exclude_turns = Some(true);
        let value = serde_json::to_value(fork).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "threadId": "th-parent",
                "model": "gpt-5.6-sol",
                "serviceTier": null,
                "cwd": "/workspace",
                "config": {"model_reasoning_effort": "high"},
                "ephemeral": true,
                "threadSource": "user",
                "excludeTurns": true
            })
        );

        let inject =
            ThreadInjectItemsParams::input_text_boundary("th-side", "Side conversation boundary.");
        assert_eq!(
            serde_json::to_value(inject).unwrap(),
            serde_json::json!({
                "threadId": "th-side",
                "items": [{
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": "Side conversation boundary."
                    }]
                }]
            })
        );
        let _: ThreadInjectItemsResponse = serde_json::from_value(serde_json::json!({})).unwrap();
    }

    #[test]
    fn thread_resume_params_camel_case() {
        let mut p = ThreadResumeParams::new("th-resume");
        p.cwd = Some("/work".into());
        p.exclude_turns = Some(true);
        p.initial_turns_page = Some(ThreadResumeInitialTurnsPageParams {
            limit: Some(16),
            sort_direction: Some(crate::ThreadTurnsSortDirection::Desc),
            items_view: Some(crate::ThreadTurnItemsView::NotLoaded),
        });
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "th-resume");
        assert_eq!(v["cwd"], "/work");
        assert_eq!(v["excludeTurns"], true);
        assert_eq!(
            v["initialTurnsPage"],
            serde_json::json!({
                "limit": 16,
                "sortDirection": "desc",
                "itemsView": "notLoaded"
            })
        );
        assert!(v.get("thread_id").is_none());
    }

    #[test]
    fn thread_unsubscribe_shape_matches_generated_contract() {
        let params = ThreadUnsubscribeParams::new("th-unsubscribe");
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value, serde_json::json!({ "threadId": "th-unsubscribe" }));

        for (wire, expected) in [
            ("notLoaded", ThreadUnsubscribeStatus::NotLoaded),
            ("notSubscribed", ThreadUnsubscribeStatus::NotSubscribed),
            ("unsubscribed", ThreadUnsubscribeStatus::Unsubscribed),
        ] {
            let response: ThreadUnsubscribeResponse =
                serde_json::from_value(serde_json::json!({ "status": wire })).unwrap();
            assert_eq!(response.status, expected);
        }
    }

    #[test]
    fn turn_interrupt_params_camel_case() {
        let p = TurnInterruptParams::new("th-1", "turn-9");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["threadId"], "th-1");
        assert_eq!(v["turnId"], "turn-9");
        assert!(v.get("thread_id").is_none());
        assert!(v.get("turn_id").is_none());
        let _: TurnInterruptResponse =
            serde_json::from_value(serde_json::json!({})).expect("empty");
    }

    #[test]
    fn thread_goal_set_get_clear_camel_case() {
        let set = ThreadGoalSetParams::new("th-goal")
            .with_objective("Ship Work mode")
            .with_status(ThreadGoalStatus::Active);
        let v = serde_json::to_value(&set).unwrap();
        assert_eq!(v["threadId"], "th-goal");
        assert_eq!(v["objective"], "Ship Work mode");
        assert_eq!(v["status"], "active");
        assert!(v.get("thread_id").is_none());

        let get = ThreadGoalGetParams::new("th-goal");
        let gv = serde_json::to_value(&get).unwrap();
        assert_eq!(gv["threadId"], "th-goal");

        let clear = ThreadGoalClearParams::new("th-goal");
        let cv = serde_json::to_value(&clear).unwrap();
        assert_eq!(cv["threadId"], "th-goal");

        let goal = ThreadGoal::new_active("th-goal", "Ship Work mode");
        let set_resp = ThreadGoalSetResponse { goal: goal.clone() };
        let sv = serde_json::to_value(&set_resp).unwrap();
        assert_eq!(sv["goal"]["threadId"], "th-goal");
        assert_eq!(sv["goal"]["objective"], "Ship Work mode");
        assert_eq!(sv["goal"]["status"], "active");
        assert_eq!(sv["goal"]["tokensUsed"], 0);

        let get_resp: ThreadGoalGetResponse = serde_json::from_value(serde_json::json!({
            "goal": null
        }))
        .expect("null goal");
        assert!(get_resp.goal.is_none());

        let get_resp2: ThreadGoalGetResponse = serde_json::from_value(
            serde_json::to_value(&ThreadGoalGetResponse { goal: Some(goal) }).unwrap(),
        )
        .expect("goal present");
        assert_eq!(get_resp2.goal.as_ref().unwrap().thread_id, "th-goal");

        let cleared: ThreadGoalClearResponse =
            serde_json::from_value(serde_json::json!({ "cleared": true })).expect("clear");
        assert!(cleared.cleared);
    }
}
