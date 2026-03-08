//! Request and response types for the API

use krusty_core::storage::{SessionInfo, WorkMode};
use krusty_core::tools::registry::PermissionMode;
use serde::{de, Deserialize, Deserializer, Serialize};

// ============================================================================
// Session Types
// ============================================================================

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub model: Option<String>,
    pub working_dir: Option<String>,
    pub target_branch: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    pub working_dir: Option<String>,
    pub mode: Option<WorkMode>,
    pub model: Option<String>,
    pub target_branch: Option<String>,
}

#[derive(Deserialize)]
pub struct PinchRequest {
    /// Optional hints about what to preserve
    pub preservation_hints: Option<String>,
    /// Optional direction for the new session
    pub direction: Option<String>,
}

#[derive(Serialize)]
pub struct PinchResponse {
    /// The new child session
    pub session: SessionResponse,
    /// Summary of what was preserved
    pub summary: String,
    /// Key decisions preserved
    pub key_decisions: Vec<String>,
    /// Pending tasks carried forward
    pub pending_tasks: Vec<String>,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    pub token_count: Option<usize>,
    pub parent_session_id: Option<String>,
    pub working_dir: Option<String>,
    pub mode: WorkMode,
    pub model: Option<String>,
    pub target_branch: Option<String>,
}

impl From<SessionInfo> for SessionResponse {
    fn from(s: SessionInfo) -> Self {
        Self {
            id: s.id,
            title: s.title,
            updated_at: s.updated_at.to_rfc3339(),
            token_count: s.token_count,
            parent_session_id: s.parent_session_id,
            working_dir: s.working_dir,
            mode: s.work_mode,
            model: s.model,
            target_branch: s.target_branch,
        }
    }
}

#[derive(Serialize)]
pub struct SessionWithMessagesResponse {
    pub session: SessionResponse,
    pub messages: Vec<MessageResponse>,
}

/// Agent execution state for a session
#[derive(Serialize)]
pub struct SessionStateResponse {
    /// Session ID
    pub id: String,
    /// Agent state: "idle", "streaming", "tool_executing", "awaiting_input", "error"
    pub agent_state: String,
    /// When the agent started (if not idle)
    pub started_at: Option<String>,
    /// Last event timestamp (for activity tracking)
    pub last_event_at: Option<String>,
    /// Current persisted work mode
    pub mode: WorkMode,
}

#[derive(Serialize)]
pub struct MessageResponse {
    pub role: String,
    pub content: serde_json::Value,
}

// ============================================================================
// Chat Types
// ============================================================================

/// Content block from PWA (text or image)
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// Extended thinking level (accepts legacy bool and newer string levels from clients).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
    XHigh,
}

impl ThinkingLevel {
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ThinkingLevelInput {
    Bool(bool),
    String(String),
}

fn deserialize_thinking_level<'de, D>(deserializer: D) -> Result<ThinkingLevel, D::Error>
where
    D: Deserializer<'de>,
{
    let input = Option::<ThinkingLevelInput>::deserialize(deserializer)?;
    match input {
        None => Ok(ThinkingLevel::Off),
        Some(ThinkingLevelInput::Bool(enabled)) => Ok(if enabled {
            ThinkingLevel::High
        } else {
            ThinkingLevel::Off
        }),
        Some(ThinkingLevelInput::String(raw)) => {
            let value = raw.trim().to_ascii_lowercase();
            match value.as_str() {
                "" | "off" | "false" | "disabled" | "none" => Ok(ThinkingLevel::Off),
                "on" | "true" | "enabled" => Ok(ThinkingLevel::High),
                "low" => Ok(ThinkingLevel::Low),
                "medium" => Ok(ThinkingLevel::Medium),
                "high" => Ok(ThinkingLevel::High),
                "xhigh" | "x-high" | "extra-high" => Ok(ThinkingLevel::XHigh),
                _ => Err(de::Error::custom(format!(
                    "invalid thinking_enabled value '{}'; expected bool or one of off/low/medium/high/xhigh",
                    raw
                ))),
            }
        }
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    /// Session ID (creates new session if not provided)
    pub session_id: Option<String>,
    /// User message content (text fallback)
    pub message: String,
    /// Multi-modal content blocks (text + images)
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// Model override
    pub model: Option<String>,
    /// Enable extended thinking
    #[serde(default, deserialize_with = "deserialize_thinking_level")]
    pub thinking_enabled: ThinkingLevel,
    /// Optional mode override for the session before starting this turn
    pub mode: Option<WorkMode>,
    /// Permission mode for tool execution
    #[serde(default)]
    pub permission_mode: PermissionMode,
}

#[cfg(test)]
mod tests {
    use super::{ChatRequest, ThinkingLevel};
    use serde_json::json;

    #[test]
    fn chat_request_accepts_legacy_bool_thinking() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "thinking_enabled": true
        }))
        .expect("request should deserialize");
        assert_eq!(req.thinking_enabled, ThinkingLevel::High);
    }

    #[test]
    fn chat_request_accepts_string_thinking_level() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "thinking_enabled": "medium"
        }))
        .expect("request should deserialize");
        assert_eq!(req.thinking_enabled, ThinkingLevel::Medium);
    }

    #[test]
    fn chat_request_defaults_thinking_to_off() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello"
        }))
        .expect("request should deserialize");
        assert_eq!(req.thinking_enabled, ThinkingLevel::Off);
    }

    #[test]
    fn chat_request_rejects_invalid_thinking_value() {
        let result = serde_json::from_value::<ChatRequest>(json!({
            "message": "hello",
            "thinking_enabled": "turbo"
        }));
        match result {
            Ok(_) => panic!("request should fail"),
            Err(err) => assert!(err.to_string().contains("invalid thinking_enabled value")),
        }
    }
}

#[derive(Deserialize)]
pub struct ToolResultRequest {
    /// Session ID
    pub session_id: String,
    /// Tool use ID to respond to
    pub tool_call_id: String,
    /// Tool result content (JSON string)
    pub result: String,
}

#[derive(Deserialize)]
pub struct ToolApprovalRequest {
    pub session_id: String,
    pub tool_call_id: String,
    pub approved: bool,
}

// ============================================================================
// Model Types
// ============================================================================

#[derive(Serialize)]
pub struct ModelResponse {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub context_window: usize,
    pub max_output: usize,
    pub supports_thinking: bool,
    pub supports_tools: bool,
}

#[derive(Serialize)]
pub struct ModelsListResponse {
    pub models: Vec<ModelResponse>,
    pub default_model: String,
}

// ============================================================================
// Tool Types
// ============================================================================

#[derive(Deserialize)]
pub struct ToolExecuteRequest {
    pub tool_name: String,
    pub params: serde_json::Value,
    /// Optional working directory override
    pub working_dir: Option<String>,
    /// Optional mode override for one-off tool execution context
    pub mode: Option<WorkMode>,
}

#[derive(Serialize)]
pub struct ToolExecuteResponse {
    pub output: String,
    pub is_error: bool,
}

// ============================================================================
// Git Types
// ============================================================================

#[derive(Deserialize)]
pub struct GitQuery {
    /// Optional path to inspect. If omitted, defaults to current workspace path.
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct GitStatusResponse {
    pub in_repo: bool,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub branch_files: usize,
    pub branch_additions: usize,
    pub branch_deletions: usize,
    pub pr_number: Option<u64>,
    pub ahead: usize,
    pub behind: usize,
    pub staged: usize,
    pub modified: usize,
    pub untracked: usize,
    pub conflicted: usize,
    pub total_changes: usize,
}

#[derive(Serialize)]
pub struct GitBranchResponse {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub is_remote: bool,
}

#[derive(Serialize)]
pub struct GitBranchesResponse {
    pub repo_root: String,
    pub branches: Vec<GitBranchResponse>,
}

#[derive(Serialize)]
pub struct GitWorktreeResponse {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_current: bool,
}

#[derive(Serialize)]
pub struct GitWorktreesResponse {
    pub repo_root: String,
    pub worktrees: Vec<GitWorktreeResponse>,
}

#[derive(Deserialize)]
pub struct GitCheckoutRequest {
    /// Optional path within a repository.
    pub path: Option<String>,
    /// Branch to switch to.
    pub branch: String,
    /// If true, creates a new branch (`git checkout -b`).
    #[serde(default)]
    pub create: bool,
    /// Optional start point used when creating a new branch.
    pub start_point: Option<String>,
}

// ============================================================================
// File Types
// ============================================================================

#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}

#[derive(Deserialize)]
pub struct TreeQuery {
    pub root: Option<String>,
    #[serde(default = "default_depth", deserialize_with = "clamp_depth")]
    pub depth: usize,
}

fn default_depth() -> usize {
    3
}

/// Maximum tree depth to prevent DoS via deep recursion
const MAX_TREE_DEPTH: usize = 10;

fn clamp_depth<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: usize = serde::Deserialize::deserialize(deserializer)?;
    Ok(value.min(MAX_TREE_DEPTH))
}

#[derive(Serialize)]
pub struct FileResponse {
    pub path: String,
    pub content: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub struct FileWriteRequest {
    pub content: String,
}

#[derive(Serialize)]
pub struct FileWriteResponse {
    pub path: String,
    pub bytes_written: usize,
}

#[derive(Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeEntry>>,
}

#[derive(Serialize)]
pub struct TreeResponse {
    pub root: String,
    pub entries: Vec<TreeEntry>,
}

#[derive(Deserialize)]
pub struct BrowseQuery {
    /// Directory to list (defaults to home directory)
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct BrowseEntry {
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct BrowseResponse {
    pub current: String,
    pub parent: Option<String>,
    pub directories: Vec<BrowseEntry>,
}

// ============================================================================
// Plan Types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct PlanItem {
    pub content: String,
    pub completed: bool,
}

// ============================================================================
// Agentic SSE Events
// ============================================================================

/// Events sent to the client during agentic chat loop
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgenticEvent {
    /// Text content delta from AI
    TextDelta { delta: String },
    /// Extended thinking delta
    ThinkingDelta { thinking: String },
    /// AI is starting a tool call
    ToolCallStart { id: String, name: String },
    /// Tool call complete with arguments
    ToolCallComplete {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Server is executing a tool
    ToolExecuting { id: String, name: String },
    /// Streaming output delta from a tool (e.g., bash)
    ToolOutputDelta { id: String, delta: String },
    /// Tool execution result
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    /// Waiting for user input (AskUserQuestion)
    AwaitingInput {
        tool_call_id: String,
        tool_name: String,
    },
    /// Mode change (set_work_mode / enter_plan_mode tools)
    ModeChange {
        mode: String,
        reason: Option<String>,
    },
    /// Plan tasks update - sent when plan is detected
    PlanUpdate { items: Vec<PlanItem> },
    /// Plan detected in AI response - awaiting confirmation
    PlanComplete {
        tool_call_id: String,
        title: String,
        task_count: usize,
    },
    /// An agentic turn completed
    TurnComplete { turn: usize, has_more: bool },
    /// Token usage information
    Usage {
        prompt_tokens: usize,
        completion_tokens: usize,
    },
    /// Agentic loop finished
    Finish { session_id: String },
    /// Session title updated (from Haiku)
    TitleUpdate { title: String },
    /// Tool requires user approval (supervised mode)
    ToolApprovalRequired {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Tool was approved by user
    ToolApproved { id: String },
    /// Tool was denied by user
    ToolDenied { id: String },
    /// Error occurred
    Error { error: String },
}

impl From<krusty_core::agent::LoopEvent> for AgenticEvent {
    fn from(event: krusty_core::agent::LoopEvent) -> Self {
        use krusty_core::agent::LoopEvent;
        match event {
            LoopEvent::TextDelta { delta } => Self::TextDelta { delta },
            LoopEvent::TextDeltaWithCitations { delta, .. } => Self::TextDelta { delta },
            LoopEvent::ThinkingDelta { thinking } => Self::ThinkingDelta { thinking },
            LoopEvent::ThinkingComplete { .. } => {
                // Server doesn't need thinking lifecycle events
                Self::ThinkingDelta {
                    thinking: String::new(),
                }
            }
            LoopEvent::ToolCallStart { id, name } => Self::ToolCallStart { id, name },
            LoopEvent::ToolCallComplete {
                id,
                name,
                arguments,
            } => Self::ToolCallComplete {
                id,
                name,
                arguments,
            },
            LoopEvent::ToolExecuting { id, name } => Self::ToolExecuting { id, name },
            LoopEvent::ToolOutputDelta { id, delta } => Self::ToolOutputDelta { id, delta },
            LoopEvent::ToolResult {
                id,
                output,
                is_error,
            } => Self::ToolResult {
                id,
                output,
                is_error,
            },
            LoopEvent::AwaitingInput {
                tool_call_id,
                tool_name,
            } => Self::AwaitingInput {
                tool_call_id,
                tool_name,
            },
            LoopEvent::ToolApprovalRequired {
                id,
                name,
                arguments,
            } => Self::ToolApprovalRequired {
                id,
                name,
                arguments,
            },
            LoopEvent::ToolApproved { id } => Self::ToolApproved { id },
            LoopEvent::ToolDenied { id } => Self::ToolDenied { id },
            // Server-side tool events — pass through as tool execution events
            LoopEvent::ServerToolStart { id, name } => Self::ToolExecuting { id, name },
            LoopEvent::ServerToolComplete { id, name } => Self::ToolResult {
                id,
                output: format!("{} completed", name),
                is_error: false,
            },
            LoopEvent::WebSearchResults { .. } => {
                // Web search results are consumed by the AI, not forwarded to SSE
                Self::TextDelta {
                    delta: String::new(),
                }
            }
            LoopEvent::WebFetchResult { .. } => Self::TextDelta {
                delta: String::new(),
            },
            LoopEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => Self::ToolResult {
                id: tool_use_id,
                output: format!("Server tool error: {}", error_code),
                is_error: true,
            },
            LoopEvent::ModeChange { mode, reason } => Self::ModeChange { mode, reason },
            LoopEvent::PlanUpdate { tasks } => Self::PlanUpdate {
                items: tasks
                    .into_iter()
                    .map(|t| PlanItem {
                        content: t.description,
                        completed: t.completed,
                    })
                    .collect(),
            },
            LoopEvent::PlanComplete {
                tool_call_id,
                title,
                task_count,
            } => Self::PlanComplete {
                tool_call_id,
                title,
                task_count,
            },
            LoopEvent::TurnComplete { turn, has_more } => Self::TurnComplete { turn, has_more },
            LoopEvent::Usage {
                prompt_tokens,
                completion_tokens,
            } => Self::Usage {
                prompt_tokens,
                completion_tokens,
            },
            LoopEvent::TitleGenerated { title } => Self::TitleUpdate { title },
            LoopEvent::Finished { session_id } => Self::Finish { session_id },
            LoopEvent::Error { error } => Self::Error { error },
        }
    }
}
