//! Request and response types for the API

use krusty_core::agent::subagent::AgentProgressStatus;
use krusty_core::agent::{
    DelegatedProgressEvent, DelegatedRunStage as CoreDelegatedRunStage,
    DelegatedToolKind as CoreDelegatedToolKind,
};
use krusty_core::ai::types::{Citation, WebFetchContent, WebSearchResult};
use krusty_core::storage::{
    DelegatedRunRecord, DelegatedRunScope, PartialAssistantState, RuntimeTraceEvent,
    RuntimeTraceSummary, SessionInfo, SessionRecoveryState, SessionType, WorkMode, WorkspaceMode,
};
use krusty_core::tools::registry::PermissionMode;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;

// ============================================================================
// Session Types
// ============================================================================

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub model: Option<String>,
    pub project_dir: Option<String>,
    pub working_dir: Option<String>,
    pub workspace_mode: Option<WorkspaceMode>,
    pub target_branch: Option<String>,
    pub session_type: Option<SessionType>,
}

#[derive(Deserialize)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    pub project_dir: Option<String>,
    pub working_dir: Option<String>,
    pub workspace_mode: Option<WorkspaceMode>,
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
    pub project_dir: Option<String>,
    pub workspace_mode: WorkspaceMode,
    pub session_type: SessionType,
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
            project_dir: s.project_dir,
            workspace_mode: s.workspace_mode,
            session_type: s.session_type,
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
    /// Interrupted-turn recovery state, if any.
    pub recovery: Option<SessionRecoveryState>,
    /// Authoritative in-flight partial assistant state for active sessions.
    pub live_partial_assistant: Option<PartialAssistantState>,
    /// Active delegated tool snapshots for this session, keyed by top-level tool call.
    pub delegated_tools: Vec<DelegatedToolStateResponse>,
    /// Recent persisted delegated runs for this session.
    pub recent_delegated_runs: Vec<DelegatedRunResponse>,
    /// Latest persisted runtime trace sequence observed for this session.
    pub last_event_sequence: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DelegatedAgentStateResponse {
    pub task_id: String,
    pub agent_name: String,
    pub status: DelegatedProgressStatus,
    pub tool_count: usize,
    pub tokens: usize,
    pub current_action: Option<String>,
    pub completion_summary: Option<String>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub completed_plan_task: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DelegatedToolStateResponse {
    pub delegated_run_id: String,
    pub tool_call_id: String,
    pub kind: DelegatedToolKind,
    pub stage: DelegatedRunStage,
    pub parent_session_id: Option<String>,
    pub agents: Vec<DelegatedAgentStateResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DelegatedRunScopeResponse {
    pub label: String,
    pub path: String,
    pub kind: String,
}

impl From<DelegatedRunScope> for DelegatedRunScopeResponse {
    fn from(value: DelegatedRunScope) -> Self {
        Self {
            label: value.label,
            path: value.path,
            kind: value.kind,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DelegatedRunResponse {
    pub delegated_run_id: String,
    pub parent_tool_call_id: Option<String>,
    pub kind: DelegatedToolKind,
    pub stage: DelegatedRunStage,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub resumable: bool,
    pub resumed_from_run_id: Option<String>,
    pub target_scope: Vec<DelegatedRunScopeResponse>,
    pub human_review: Option<String>,
    pub artifact: Option<Value>,
    pub updated_at: String,
}

impl From<DelegatedRunRecord> for DelegatedRunResponse {
    fn from(value: DelegatedRunRecord) -> Self {
        Self {
            delegated_run_id: value.delegated_run_id,
            parent_tool_call_id: value.parent_tool_call_id,
            kind: match value.role {
                krusty_core::storage::DelegatedRunRole::Explore => DelegatedToolKind::Explore,
                krusty_core::storage::DelegatedRunRole::Planner => DelegatedToolKind::Plan,
                krusty_core::storage::DelegatedRunRole::Verifier => DelegatedToolKind::Verify,
                krusty_core::storage::DelegatedRunRole::Build => DelegatedToolKind::Build,
            },
            stage: match value.stage {
                krusty_core::agent::DelegatedRunStage::Created => DelegatedRunStage::Created,
                krusty_core::agent::DelegatedRunStage::Running => DelegatedRunStage::Running,
                krusty_core::agent::DelegatedRunStage::Synthesizing => {
                    DelegatedRunStage::Synthesizing
                }
                krusty_core::agent::DelegatedRunStage::Complete => DelegatedRunStage::Complete,
                krusty_core::agent::DelegatedRunStage::Degraded => DelegatedRunStage::Degraded,
                krusty_core::agent::DelegatedRunStage::Failed => DelegatedRunStage::Failed,
                krusty_core::agent::DelegatedRunStage::Cancelled => DelegatedRunStage::Cancelled,
            },
            provider: value.provider,
            model: value.model,
            resumable: value.resumable,
            resumed_from_run_id: value.resumed_from_run_id,
            target_scope: value.target_scope.into_iter().map(Into::into).collect(),
            human_review: value.human_review,
            artifact: value.artifact,
            updated_at: value.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct SessionTraceResponse {
    pub id: String,
    pub summary: RuntimeTraceSummary,
    pub events: Vec<RuntimeTraceEvent>,
    pub latest_sequence: Option<i64>,
}

#[derive(Deserialize)]
pub struct SessionPresenceHeartbeatRequest {
    pub client_id: String,
    pub surface: String,
    pub capability: crate::presence::PresenceCapability,
    pub last_event_sequence: Option<i64>,
}

#[derive(Serialize)]
pub struct SessionPresenceClientResponse {
    pub client_id: String,
    pub surface: String,
    pub capability: crate::presence::PresenceCapability,
    pub user_id: Option<String>,
    pub last_seen_at: String,
    pub last_event_sequence: Option<i64>,
    pub stale: bool,
}

#[derive(Serialize)]
pub struct SessionPresenceResponse {
    pub session_id: String,
    pub active_viewers: usize,
    pub active_controllers: usize,
    pub stale_clients: usize,
    pub clients: Vec<SessionPresenceClientResponse>,
}

#[derive(Serialize)]
pub struct ServerAccessResponse {
    pub local_url: String,
    pub remote_access_enabled: bool,
    pub remote_access_token: String,
    pub remote_launch_url: Option<String>,
    pub tailscale: TailscaleAccessResponse,
}

#[derive(Deserialize)]
pub struct UpdateServerAccessRequest {
    pub enabled: Option<bool>,
    pub rotate_token: Option<bool>,
}

#[derive(Serialize)]
pub struct TailscaleAccessResponse {
    pub status: String,
    pub url: Option<String>,
    pub detail: Option<String>,
}

#[derive(Serialize)]
pub struct ActiveSessionStatusResponse {
    pub id: String,
    pub title: String,
    pub agent_state: String,
    pub started_at: Option<String>,
    pub last_event_at: Option<String>,
    pub working_dir: Option<String>,
    pub project_dir: Option<String>,
    pub workspace_mode: WorkspaceMode,
    pub active_viewers: usize,
    pub active_controllers: usize,
    pub stale_clients: usize,
}

#[derive(Serialize)]
pub struct ServerStatusResponse {
    pub active_agent_streams: usize,
    pub active_sessions: Vec<ActiveSessionStatusResponse>,
    pub memory: ServerMemoryStatusResponse,
    pub tailscale: TailscaleAccessResponse,
}

#[derive(Serialize)]
pub struct ServerMemoryStatusResponse {
    pub rss_bytes: Option<u64>,
    pub virtual_bytes: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub peak_virtual_bytes: Option<u64>,
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
    /// Explicit project directory for newly created sessions.
    pub project_dir: Option<String>,
    /// Explicit workspace directory for newly created sessions. `None` means no project selected.
    /// Legacy alias for `project_dir`.
    pub working_dir: Option<String>,
    /// Explicit semantic workspace mode for newly created sessions.
    pub workspace_mode: Option<WorkspaceMode>,
    /// High-level session surface for newly created sessions.
    pub session_type: Option<SessionType>,
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
    /// Enable research tools (agent, reports) in Chat sessions
    #[serde(default)]
    pub research_enabled: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::{AgenticEvent, ChatRequest, ThinkingLevel};
    use krusty_core::agent::LoopEvent;
    use krusty_core::storage::{RuntimeTraceEvent, WorkspaceMode};
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
    fn chat_request_accepts_explicit_no_project_working_dir() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "working_dir": null
        }))
        .expect("request should deserialize");
        assert_eq!(req.working_dir, None);
        assert_eq!(req.project_dir, None);
        assert_eq!(req.workspace_mode, None);
    }

    #[test]
    fn chat_request_accepts_explicit_workspace_contract() {
        let req: ChatRequest = serde_json::from_value(json!({
            "message": "hello",
            "project_dir": "/tmp/demo",
            "workspace_mode": "created"
        }))
        .expect("request should deserialize");
        assert_eq!(req.project_dir.as_deref(), Some("/tmp/demo"));
        assert_eq!(req.workspace_mode, Some(WorkspaceMode::Created));
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

    #[test]
    fn agentic_event_preserves_thinking_complete_shape() {
        let mapped = AgenticEvent::from(LoopEvent::ThinkingComplete {
            thinking: "analysis".to_string(),
            signature: "sig-1".to_string(),
        });

        match mapped {
            AgenticEvent::ThinkingComplete {
                thinking,
                signature,
            } => {
                assert_eq!(thinking, "analysis");
                assert_eq!(signature, "sig-1");
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn agentic_event_preserves_web_search_results_shape() {
        let mapped = AgenticEvent::from(LoopEvent::WebSearchResults {
            tool_use_id: "tool-1".to_string(),
            results: Vec::new(),
        });

        match mapped {
            AgenticEvent::WebSearchResults {
                tool_use_id,
                results,
            } => {
                assert_eq!(tool_use_id, "tool-1");
                assert!(results.is_empty());
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn agentic_event_preserves_server_tool_error_shape() {
        let mapped = AgenticEvent::from(LoopEvent::ServerToolError {
            tool_use_id: "tool-7".to_string(),
            error_code: "timeout".to_string(),
        });

        match mapped {
            AgenticEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => {
                assert_eq!(tool_use_id, "tool-7");
                assert_eq!(error_code, "timeout");
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }
    #[test]
    fn agentic_event_replays_user_message_trace() {
        let event = RuntimeTraceEvent {
            run_id: "run-1".to_string(),
            sequence: 7,
            turn: 1,
            event_type: "user_message".to_string(),
            payload: json!({
                "title": "Milestone",
                "message": "Indexing finished",
                "level": "success"
            }),
            failure_category: None,
            stop_reason: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        match AgenticEvent::from_runtime_trace(event) {
            Some(AgenticEvent::UserMessage {
                title,
                message,
                level,
            }) => {
                assert_eq!(title.as_deref(), Some("Milestone"));
                assert_eq!(message, "Indexing finished");
                assert_eq!(level, "success");
            }
            other => panic!("unexpected replay mapping: {other:?}"),
        }
    }

    #[test]
    fn agentic_event_skips_unreplayable_trace_shapes() {
        let event = RuntimeTraceEvent {
            run_id: "run-1".to_string(),
            sequence: 8,
            turn: 1,
            event_type: "plan_update".to_string(),
            payload: json!({ "task_count": 3 }),
            failure_category: None,
            stop_reason: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        assert!(AgenticEvent::from_runtime_trace(event).is_none());
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

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedToolKind {
    Explore,
    Plan,
    Verify,
    Build,
}

impl From<CoreDelegatedToolKind> for DelegatedToolKind {
    fn from(value: CoreDelegatedToolKind) -> Self {
        match value {
            CoreDelegatedToolKind::Explore => Self::Explore,
            CoreDelegatedToolKind::Plan => Self::Plan,
            CoreDelegatedToolKind::Verify => Self::Verify,
            CoreDelegatedToolKind::Build => Self::Build,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedProgressStatus {
    Running,
    Complete,
    Failed,
}

impl From<&AgentProgressStatus> for DelegatedProgressStatus {
    fn from(value: &AgentProgressStatus) -> Self {
        match value {
            AgentProgressStatus::Running => Self::Running,
            AgentProgressStatus::Complete => Self::Complete,
            AgentProgressStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedRunStage {
    Created,
    Running,
    Synthesizing,
    Complete,
    Degraded,
    Failed,
    Cancelled,
}

impl From<CoreDelegatedRunStage> for DelegatedRunStage {
    fn from(value: CoreDelegatedRunStage) -> Self {
        match value {
            CoreDelegatedRunStage::Created => Self::Created,
            CoreDelegatedRunStage::Running => Self::Running,
            CoreDelegatedRunStage::Synthesizing => Self::Synthesizing,
            CoreDelegatedRunStage::Complete => Self::Complete,
            CoreDelegatedRunStage::Degraded => Self::Degraded,
            CoreDelegatedRunStage::Failed => Self::Failed,
            CoreDelegatedRunStage::Cancelled => Self::Cancelled,
        }
    }
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
    /// Text content delta with citations
    TextDeltaWithCitations {
        delta: String,
        citations: Vec<Citation>,
    },
    /// Extended thinking delta
    ThinkingDelta { thinking: String },
    /// Extended thinking block completed
    ThinkingComplete { thinking: String, signature: String },
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
    /// Live delegated progress from explore/build sub-agents.
    DelegatedProgress {
        delegated_run_id: String,
        tool_call_id: String,
        kind: DelegatedToolKind,
        stage: DelegatedRunStage,
        parent_session_id: String,
        task_id: String,
        agent_name: String,
        status: DelegatedProgressStatus,
        tool_count: usize,
        tokens: usize,
        current_action: Option<String>,
        completion_summary: Option<String>,
        lines_added: usize,
        lines_removed: usize,
        completed_plan_task: Option<String>,
    },
    /// Tool execution result
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    /// Server-side tool started
    ServerToolStart { id: String, name: String },
    /// Server-side tool completed
    ServerToolComplete { id: String, name: String },
    /// Server-side web search results
    WebSearchResults {
        tool_use_id: String,
        results: Vec<WebSearchResult>,
    },
    /// Server-side web fetch result
    WebFetchResult {
        tool_use_id: String,
        content: WebFetchContent,
    },
    /// Server-side tool error
    ServerToolError {
        tool_use_id: String,
        error_code: String,
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
    /// The autonomous agent is sleeping between ticks.
    AgentSleeping { duration_secs: u64, reason: String },
    /// An agentic turn completed
    TurnComplete { turn: usize, has_more: bool },
    /// The server injected a synthetic tick to continue autonomous work.
    TickInjected { tick_number: usize },
    /// Token usage information
    Usage {
        prompt_tokens: usize,
        completion_tokens: usize,
    },
    /// Live conversation compaction telemetry
    ContextCompacted {
        reason: String,
        estimated_tokens_before: usize,
        estimated_tokens_after: usize,
        replaced_messages: usize,
    },
    /// Some non-terminal stream events were dropped because the client fell behind.
    Lagged { skipped: usize },
    /// Agentic loop finished
    Finish {
        session_id: String,
        stop_reason: String,
    },
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
    /// A background agent was started
    AgentBackgroundStarted {
        delegated_run_id: String,
        agent_type: String,
        description: String,
    },
    /// A background agent completed
    AgentBackgroundCompleted {
        delegated_run_id: String,
        agent_type: String,
        success: bool,
        summary: String,
    },
    // ── Mako autonomous agent events ─────────────────────────────────
    /// A user-visible message emitted by the SendUserMessage tool
    UserMessage {
        title: Option<String>,
        message: String,
        level: String,
    },
    /// Auto-classifier evaluated a tool call
    ClassifierDecision {
        tool_name: String,
        decision: String,
        reason: String,
        stage: u8,
    },
    /// A teammate was spawned
    TeammateSpawned { name: String, role: String },
    /// A teammate completed a task
    TeammateTaskCompleted {
        name: String,
        task_id: String,
        result: String,
    },
    /// A teammate failed a task
    TeammateTaskFailed {
        name: String,
        task_id: String,
        error: String,
    },
    /// A teammate was cancelled
    TeammateCancelled { name: String },
}

impl AgenticEvent {
    pub fn delegated_progress(event: DelegatedProgressEvent) -> Self {
        let progress = event.progress;
        Self::DelegatedProgress {
            delegated_run_id: event.delegated_run_id,
            tool_call_id: event.tool_call_id,
            kind: DelegatedToolKind::from(event.kind),
            stage: DelegatedRunStage::from(event.stage),
            parent_session_id: event.parent_session_id,
            task_id: progress.task_id,
            agent_name: progress.name,
            status: DelegatedProgressStatus::from(&progress.status),
            tool_count: progress.tool_count,
            tokens: progress.tokens,
            current_action: progress.current_action,
            completion_summary: progress.completion_summary,
            lines_added: progress.lines_added,
            lines_removed: progress.lines_removed,
            completed_plan_task: progress.completed_plan_task,
        }
    }

    pub fn from_runtime_trace(event: RuntimeTraceEvent) -> Option<Self> {
        let payload = event.payload;
        match event.event_type.as_str() {
            "agent_sleeping" => Some(Self::AgentSleeping {
                duration_secs: payload.get("duration_secs")?.as_u64()?,
                reason: payload.get("reason")?.as_str()?.to_string(),
            }),
            "finished" => Some(Self::Finish {
                session_id: payload.get("session_id")?.as_str()?.to_string(),
                stop_reason: payload.get("stop_reason")?.as_str()?.to_string(),
            }),
            "error" => Some(Self::Error {
                error: payload.get("error")?.as_str()?.to_string(),
            }),
            "user_message" => Some(Self::UserMessage {
                title: payload
                    .get("title")
                    .and_then(|value| value.as_str().map(ToString::to_string)),
                message: payload.get("message")?.as_str()?.to_string(),
                level: payload
                    .get("level")
                    .and_then(|value| value.as_str())
                    .unwrap_or("info")
                    .to_string(),
            }),
            "classifier_decision" => Some(Self::ClassifierDecision {
                tool_name: payload.get("tool_name")?.as_str()?.to_string(),
                decision: payload.get("decision")?.as_str()?.to_string(),
                reason: payload.get("reason")?.as_str()?.to_string(),
                stage: payload.get("stage")?.as_u64()? as u8,
            }),
            "title_generated" => Some(Self::TitleUpdate {
                title: payload.get("title")?.as_str()?.to_string(),
            }),
            "agent_background_started" => Some(Self::AgentBackgroundStarted {
                delegated_run_id: payload.get("delegated_run_id")?.as_str()?.to_string(),
                agent_type: payload.get("agent_type")?.as_str()?.to_string(),
                description: payload.get("description")?.as_str()?.to_string(),
            }),
            "agent_background_completed" => Some(Self::AgentBackgroundCompleted {
                delegated_run_id: payload.get("delegated_run_id")?.as_str()?.to_string(),
                agent_type: payload.get("agent_type")?.as_str()?.to_string(),
                success: payload.get("success")?.as_bool()?,
                summary: payload.get("summary")?.as_str()?.to_string(),
            }),
            _ => None,
        }
    }
}

impl From<krusty_core::agent::LoopEvent> for AgenticEvent {
    fn from(event: krusty_core::agent::LoopEvent) -> Self {
        use krusty_core::agent::LoopEvent;
        match event {
            LoopEvent::TextDelta { delta } => Self::TextDelta { delta },
            LoopEvent::TextDeltaWithCitations { delta, citations } => {
                Self::TextDeltaWithCitations { delta, citations }
            }
            LoopEvent::ThinkingDelta { thinking } => Self::ThinkingDelta { thinking },
            LoopEvent::ThinkingComplete {
                thinking,
                signature,
            } => Self::ThinkingComplete {
                thinking,
                signature,
            },
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
            LoopEvent::ServerToolStart { id, name } => Self::ServerToolStart { id, name },
            LoopEvent::ServerToolComplete { id, name } => Self::ServerToolComplete { id, name },
            LoopEvent::WebSearchResults {
                tool_use_id,
                results,
            } => Self::WebSearchResults {
                tool_use_id,
                results,
            },
            LoopEvent::WebFetchResult {
                tool_use_id,
                content,
            } => Self::WebFetchResult {
                tool_use_id,
                content,
            },
            LoopEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => Self::ServerToolError {
                tool_use_id,
                error_code,
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
            LoopEvent::AgentSleeping {
                duration_secs,
                reason,
            } => Self::AgentSleeping {
                duration_secs,
                reason,
            },
            LoopEvent::TurnComplete { turn, has_more } => Self::TurnComplete { turn, has_more },
            LoopEvent::TickInjected { tick_number } => Self::TickInjected { tick_number },
            LoopEvent::Usage {
                prompt_tokens,
                completion_tokens,
            } => Self::Usage {
                prompt_tokens,
                completion_tokens,
            },
            LoopEvent::ContextCompacted {
                reason,
                estimated_tokens_before,
                estimated_tokens_after,
                replaced_messages,
            } => Self::ContextCompacted {
                reason,
                estimated_tokens_before,
                estimated_tokens_after,
                replaced_messages,
            },
            LoopEvent::TitleGenerated { title } => Self::TitleUpdate { title },
            LoopEvent::Finished {
                session_id,
                stop_reason,
            } => Self::Finish {
                session_id,
                stop_reason: serde_json::to_value(stop_reason)
                    .ok()
                    .and_then(|v| v.as_str().map(ToString::to_string))
                    .unwrap_or_else(|| "completed".to_string()),
            },
            LoopEvent::Error { error } => Self::Error { error },
            LoopEvent::AgentBackgroundStarted {
                delegated_run_id,
                agent_type,
                description,
            } => Self::AgentBackgroundStarted {
                delegated_run_id,
                agent_type,
                description,
            },
            LoopEvent::AgentBackgroundCompleted {
                delegated_run_id,
                agent_type,
                success,
                summary,
            } => Self::AgentBackgroundCompleted {
                delegated_run_id,
                agent_type,
                success,
                summary,
            },
            LoopEvent::UserMessage {
                title,
                message,
                level,
            } => Self::UserMessage {
                title,
                message,
                level,
            },
            LoopEvent::ClassifierDecision {
                tool_name,
                decision,
                reason,
                stage,
            } => Self::ClassifierDecision {
                tool_name,
                decision,
                reason,
                stage,
            },
            LoopEvent::TeammateSpawned { name, role } => Self::TeammateSpawned { name, role },
            LoopEvent::TeammateTaskCompleted {
                name,
                task_id,
                result,
            } => Self::TeammateTaskCompleted {
                name,
                task_id,
                result,
            },
            LoopEvent::TeammateTaskFailed {
                name,
                task_id,
                error,
            } => Self::TeammateTaskFailed {
                name,
                task_id,
                error,
            },
            LoopEvent::TeammateCancelled { name } => Self::TeammateCancelled { name },
        }
    }
}
