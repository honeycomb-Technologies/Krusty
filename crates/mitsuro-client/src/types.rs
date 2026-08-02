use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    #[serde(default)]
    pub features: std::collections::HashMap<String, bool>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub struct ModelsResponse {
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_model_key: Option<ModelKey>,
}

/// Provider-aware executable model identity returned by modern Mitsuro servers.
/// String-valued wire fields keep this client forward-compatible with new
/// providers and transport formats.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct ModelKey {
    pub provider: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_scope: Option<String>,
    pub api_format: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    #[serde(default)]
    pub key: Option<ModelKey>,
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub context_window: usize,
    #[serde(default)]
    pub max_output: usize,
    #[serde(default)]
    pub supports_thinking: bool,
    #[serde(default)]
    pub reasoning_control: Option<ReasoningControl>,
    #[serde(default)]
    pub supported_reasoning_levels: Vec<ReasoningEffort>,
    #[serde(default)]
    pub default_reasoning_level: Option<ReasoningEffort>,
    #[serde(default)]
    pub reasoning_is_mandatory: bool,
    #[serde(default)]
    pub supports_fast_mode: bool,
    #[serde(default)]
    pub fast_mode: Option<FastMode>,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_vision: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningControl {
    OpenAiEffort,
    AnthropicAdaptive,
    AnthropicBudget,
    Boolean,
    OutputOnly,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FastMode {
    Priority,
    AnthropicFast,
    #[serde(other)]
    Unknown,
}

impl ModelInfo {
    pub fn selectable_thinking_levels(&self) -> Vec<ThinkingLevel> {
        if self.reasoning_control == Some(ReasoningControl::OutputOnly) {
            return vec![ThinkingLevel::Off];
        }

        let mut levels = self
            .supported_reasoning_levels
            .iter()
            .filter(|effort| **effort != ReasoningEffort::Ultra)
            .filter_map(|effort| ThinkingLevel::from_reasoning_effort(*effort))
            .collect::<Vec<_>>();
        levels.dedup();

        // Older Mitsuro servers only advertised the coarse `supports_thinking`
        // flag. Keep those models usable while newer servers provide the exact
        // ordered effort list.
        if levels.is_empty() {
            if !self.supports_thinking {
                return vec![ThinkingLevel::Off];
            }

            let fallback = self
                .default_reasoning_level
                .and_then(ThinkingLevel::from_reasoning_effort)
                .filter(|level| !matches!(level, ThinkingLevel::Off | ThinkingLevel::Ultra))
                .unwrap_or(ThinkingLevel::Medium);
            return if self.reasoning_is_mandatory {
                vec![fallback]
            } else {
                vec![ThinkingLevel::Off, fallback]
            };
        }

        if self.reasoning_is_mandatory {
            levels.retain(|level| *level != ThinkingLevel::Off);
            if levels.is_empty() {
                levels.push(
                    self.default_reasoning_level
                        .and_then(ThinkingLevel::from_reasoning_effort)
                        .filter(|level| !matches!(level, ThinkingLevel::Off | ThinkingLevel::Ultra))
                        .unwrap_or(ThinkingLevel::Medium),
                );
            }
        } else if !levels.contains(&ThinkingLevel::Off) {
            levels.insert(0, ThinkingLevel::Off);
        }
        levels
    }

    pub fn normalize_thinking_level(&self, current: ThinkingLevel) -> ThinkingLevel {
        let levels = self.selectable_thinking_levels();
        if levels.is_empty() {
            return if self.supports_thinking {
                current
            } else {
                ThinkingLevel::Off
            };
        }
        if levels.contains(&current) {
            return current;
        }
        self.default_reasoning_level
            .and_then(ThinkingLevel::from_reasoning_effort)
            .filter(|level| levels.contains(level))
            .unwrap_or(levels[0])
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey,
    OAuthBrowser,
    OAuthDevice,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub configured: bool,
    pub has_oauth: bool,
    pub supports_oauth: bool,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SetCredentialRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OAuthStartRequest {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_type: Option<String>,
}

impl OAuthStartRequest {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            flow_type: None,
        }
    }

    pub fn with_flow_type(provider: impl Into<String>, flow_type: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            flow_type: Some(flow_type.into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthDeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthStartResponse {
    pub auth_url: String,
    pub provider: String,
    pub flow_type: String,
    pub paste_code: bool,
    #[serde(default)]
    pub device_code: Option<OAuthDeviceCodeInfo>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthStatusResponse {
    pub has_token: bool,
    pub flow_active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OAuthExchangeRequest {
    pub provider: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthExchangeResponse {
    pub success: bool,
}

impl ModelInfo {
    pub fn label(&self) -> &str {
        if self.display_name.is_empty() {
            &self.id
        } else {
            &self.display_name
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    Chat,
    #[default]
    Code,
    #[serde(alias = "mako")]
    Hive,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    Created,
    Selected,
    #[default]
    Neutral,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    #[default]
    Build,
    Plan,
}

impl WorkMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Build => Self::Plan,
            Self::Plan => Self::Build,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Supervised,
    #[default]
    Autonomous,
}

impl PermissionMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Supervised => Self::Autonomous,
            Self::Autonomous => Self::Supervised,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Supervised => "supervised",
            Self::Autonomous => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

impl ThinkingLevel {
    pub fn api_value(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Minimal => Some("minimal"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::XHigh => Some("xhigh"),
            Self::Max => Some("max"),
            Self::Ultra => Some("ultra"),
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Minimal,
            Self::Minimal => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Max,
            Self::Max | Self::Ultra => Self::Off,
        }
    }

    pub fn cycle_for_model(self, model: &ModelInfo) -> Self {
        let levels = model.selectable_thinking_levels();
        if levels.is_empty() {
            return model.normalize_thinking_level(self);
        }
        let index = levels.iter().position(|level| *level == self);
        levels[index.map_or(0, |index| (index + 1) % levels.len())]
    }

    fn from_reasoning_effort(effort: ReasoningEffort) -> Option<Self> {
        match effort {
            ReasoningEffort::None => Some(Self::Off),
            ReasoningEffort::Minimal => Some(Self::Minimal),
            ReasoningEffort::Low => Some(Self::Low),
            ReasoningEffort::Medium => Some(Self::Medium),
            ReasoningEffort::High => Some(Self::High),
            ReasoningEffort::XHigh => Some(Self::XHigh),
            ReasoningEffort::Max => Some(Self::Max),
            ReasoningEffort::Ultra => Some(Self::Ultra),
            ReasoningEffort::Unknown => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "min",
            Self::Low => "low",
            Self::Medium => "med",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub token_count: Option<usize>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub workspace_mode: WorkspaceMode,
    #[serde(default)]
    pub session_type: SessionType,
    #[serde(default)]
    pub mode: WorkMode,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_key: Option<ModelKey>,
    #[serde(default)]
    pub model_catalog_revision: Option<String>,
    #[serde(default)]
    pub target_branch: Option<String>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SessionWithMessages {
    pub session: SessionInfo,
    #[serde(default)]
    pub messages: Vec<MessageResponse>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MessageResponse {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct CreateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_key: Option<ModelKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_mode: Option<WorkspaceMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_type: Option<SessionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct UpdateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_key: Option<ModelKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_mode: Option<WorkspaceMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_branch: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<WorkMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SessionStateResponse {
    pub id: String,
    pub agent_state: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub last_event_at: Option<String>,
    #[serde(default)]
    pub mode: WorkMode,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub recovery: Option<SessionRecoveryState>,
    #[serde(default)]
    pub pending_interactions: Vec<PendingInteractionSnapshot>,
    #[serde(default)]
    pub live_partial_assistant: Option<PartialAssistantState>,
    #[serde(default)]
    pub delegated_tools: Vec<Value>,
    #[serde(default)]
    pub recent_delegated_runs: Vec<Value>,
    #[serde(default)]
    pub last_event_sequence: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SessionRecoveryState {
    pub schema_version: usize,
    pub status: String,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub partial_assistant: PartialAssistantState,
    #[serde(default)]
    pub pending_interactions: Vec<PendingInteractionSnapshot>,
    #[serde(default)]
    pub decision: Value,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct PartialAssistantState {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<RecoveryToolCall>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RecoveryToolArguments {
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub redacted_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RecoveryToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Option<RecoveryToolArguments>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingInteractionSnapshot {
    ToolApproval {
        tool_call: RecoveryToolCall,
    },
    AskUserQuestion {
        tool_call_id: String,
        #[serde(default)]
        questions: Vec<Value>,
    },
    PlanConfirm {
        tool_call_id: String,
        title: String,
        task_count: usize,
        #[serde(default)]
        tasks: Vec<Value>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServerAccessResponse {
    pub local_url: String,
    pub remote_access_enabled: bool,
    pub remote_access_token_available: bool,
    #[serde(default)]
    pub revealed_remote_access_token: Option<String>,
    #[serde(default)]
    pub remote_launch_url: Option<String>,
    pub tailscale: TailscaleAccessResponse,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct UpdateServerAccessRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate_token: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reveal_token: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TailscaleAccessResponse {
    pub status: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ServerStatusResponse {
    pub active_agent_streams: usize,
    #[serde(default)]
    pub active_sessions: Vec<ActiveSessionStatus>,
    #[serde(default)]
    pub memory: Value,
    pub tailscale: TailscaleAccessResponse,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ActiveSessionStatus {
    pub id: String,
    pub title: String,
    pub agent_state: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub last_event_at: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub workspace_mode: WorkspaceMode,
    pub active_viewers: usize,
    pub active_controllers: usize,
    pub stale_clients: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_mode: Option<WorkspaceMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_type: Option<SessionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_key: Option<ModelKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_enabled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<WorkMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Deprecated compatibility field. Chat research is automatic and this value is ignored.
    pub research_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolApprovalRequest {
    pub session_id: String,
    pub tool_call_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SimpleOkResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlanItem {
    pub content: String,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatStreamEvent {
    TextDelta {
        delta: String,
    },
    TextDeltaWithCitations {
        delta: String,
        citations: Value,
    },
    ThinkingDelta {
        thinking: String,
    },
    ThinkingComplete {
        thinking: String,
        signature: String,
    },
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallComplete {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolExecuting {
        id: String,
        name: String,
    },
    ToolOutputDelta {
        id: String,
        delta: String,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    ServerToolStart {
        id: String,
        name: String,
    },
    ServerToolComplete {
        id: String,
        name: String,
    },
    ServerToolError {
        tool_use_id: String,
        error_code: String,
    },
    WebSearchResults {
        tool_use_id: String,
        payload: Value,
    },
    WebFetchResult {
        tool_use_id: String,
        payload: Value,
    },
    DelegatedProgress {
        payload: Value,
    },
    AwaitingInput {
        tool_call_id: String,
        tool_name: String,
    },
    ModeChange {
        mode: String,
        reason: Option<String>,
    },
    PlanUpdate {
        items: Vec<PlanItem>,
    },
    PlanComplete {
        tool_call_id: String,
        title: String,
        task_count: usize,
    },
    AgentSleeping {
        duration_secs: u64,
        reason: String,
    },
    TurnComplete {
        turn: usize,
        has_more: bool,
    },
    TickInjected {
        tick_number: usize,
    },
    Usage {
        prompt_tokens: usize,
        input_tokens: usize,
        completion_tokens: usize,
        reasoning_tokens: usize,
        cache_creation_input_tokens: usize,
        cache_read_input_tokens: usize,
        total_tokens: usize,
    },
    SessionPinched {
        reason: String,
        source_session_id: String,
        new_session_id: String,
        estimated_tokens_before: usize,
    },
    ContextCompactionStarted {
        reason: String,
    },
    ContextCompacted {
        payload: Value,
    },
    Lagged {
        skipped: usize,
    },
    Finish {
        session_id: String,
        stop_reason: String,
    },
    TitleUpdate {
        title: String,
    },
    ToolApprovalRequired {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolApproved {
        id: String,
    },
    ToolDenied {
        id: String,
    },
    SteeringInjected {
        pending_id: Option<String>,
        message: String,
    },
    Error {
        error: String,
    },
    AgentBackgroundStarted {
        payload: Value,
    },
    AgentBackgroundCompleted {
        payload: Value,
    },
    UserMessage {
        title: Option<String>,
        message: String,
        level: String,
    },
    ClassifierDecision {
        tool_name: String,
        decision: String,
        reason: String,
        stage: u8,
    },
    TeammateSpawned {
        name: String,
        role: String,
    },
    TeammateTaskCompleted {
        name: String,
        task_id: String,
        result: String,
    },
    TeammateTaskFailed {
        name: String,
        task_id: String,
        error: String,
    },
    TeammateCancelled {
        name: String,
    },
    Other {
        event_type: String,
        payload: Value,
    },
}

impl ChatStreamEvent {
    pub fn from_json_value(value: Value) -> Self {
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match event_type.as_str() {
            "text_delta" => Self::TextDelta {
                delta: string_field(&value, "delta"),
            },
            "text_delta_with_citations" => Self::TextDeltaWithCitations {
                delta: string_field(&value, "delta"),
                citations: value.get("citations").cloned().unwrap_or(Value::Null),
            },
            "thinking_delta" => Self::ThinkingDelta {
                thinking: string_field(&value, "thinking"),
            },
            "thinking_complete" => Self::ThinkingComplete {
                thinking: string_field(&value, "thinking"),
                signature: string_field(&value, "signature"),
            },
            "tool_call_start" => Self::ToolCallStart {
                id: string_field(&value, "id"),
                name: string_field(&value, "name"),
            },
            "tool_call_complete" => Self::ToolCallComplete {
                id: string_field(&value, "id"),
                name: string_field(&value, "name"),
                arguments: value.get("arguments").cloned().unwrap_or(Value::Null),
            },
            "tool_executing" => Self::ToolExecuting {
                id: string_field(&value, "id"),
                name: string_field(&value, "name"),
            },
            "tool_output_delta" => Self::ToolOutputDelta {
                id: string_field(&value, "id"),
                delta: string_field(&value, "delta"),
            },
            "tool_result" => Self::ToolResult {
                id: string_field(&value, "id"),
                output: string_field(&value, "output"),
                is_error: bool_field(&value, "is_error"),
            },
            "server_tool_start" => Self::ServerToolStart {
                id: string_field(&value, "id"),
                name: string_field(&value, "name"),
            },
            "server_tool_complete" => Self::ServerToolComplete {
                id: string_field(&value, "id"),
                name: string_field(&value, "name"),
            },
            "server_tool_error" => Self::ServerToolError {
                tool_use_id: string_field(&value, "tool_use_id"),
                error_code: string_field(&value, "error_code"),
            },
            "web_search_results" => Self::WebSearchResults {
                tool_use_id: string_field(&value, "tool_use_id"),
                payload: value,
            },
            "web_fetch_result" => Self::WebFetchResult {
                tool_use_id: string_field(&value, "tool_use_id"),
                payload: value,
            },
            "delegated_progress" => Self::DelegatedProgress { payload: value },
            "awaiting_input" => Self::AwaitingInput {
                tool_call_id: string_field(&value, "tool_call_id"),
                tool_name: string_field(&value, "tool_name"),
            },
            "mode_change" => Self::ModeChange {
                mode: string_field(&value, "mode"),
                reason: optional_string_field(&value, "reason"),
            },
            "plan_update" => Self::PlanUpdate {
                items: value
                    .get("items")
                    .and_then(|items| serde_json::from_value(items.clone()).ok())
                    .unwrap_or_default(),
            },
            "plan_complete" => Self::PlanComplete {
                tool_call_id: string_field(&value, "tool_call_id"),
                title: string_field(&value, "title"),
                task_count: usize_field(&value, "task_count"),
            },
            "agent_sleeping" => Self::AgentSleeping {
                duration_secs: u64_field(&value, "duration_secs"),
                reason: string_field(&value, "reason"),
            },
            "turn_complete" => Self::TurnComplete {
                turn: usize_field(&value, "turn"),
                has_more: bool_field(&value, "has_more"),
            },
            "tick_injected" => Self::TickInjected {
                tick_number: usize_field(&value, "tick_number"),
            },
            "usage" => {
                let prompt_tokens = usize_field(&value, "prompt_tokens");
                let completion_tokens = usize_field(&value, "completion_tokens");
                let reasoning_tokens =
                    usize_field(&value, "reasoning_tokens").min(completion_tokens);
                let cache_creation_input_tokens =
                    usize_field(&value, "cache_creation_input_tokens");
                let cache_read_input_tokens = usize_field(&value, "cache_read_input_tokens");
                let input_tokens = value
                    .get("input_tokens")
                    .and_then(Value::as_u64)
                    .map(|tokens| tokens as usize)
                    .unwrap_or_else(|| {
                        prompt_tokens
                            .saturating_add(cache_creation_input_tokens)
                            .saturating_add(cache_read_input_tokens)
                    });
                Self::Usage {
                    prompt_tokens,
                    input_tokens,
                    completion_tokens,
                    reasoning_tokens,
                    cache_creation_input_tokens,
                    cache_read_input_tokens,
                    total_tokens: usize_field(&value, "total_tokens"),
                }
            }
            "session_pinched" => Self::SessionPinched {
                reason: string_field(&value, "reason"),
                source_session_id: string_field(&value, "source_session_id"),
                new_session_id: string_field(&value, "new_session_id"),
                estimated_tokens_before: usize_field(&value, "estimated_tokens_before"),
            },
            "context_compaction_started" => Self::ContextCompactionStarted {
                reason: string_field(&value, "reason"),
            },
            "context_compacted" => Self::ContextCompacted { payload: value },
            "lagged" => Self::Lagged {
                skipped: usize_field(&value, "skipped"),
            },
            "finish" => Self::Finish {
                session_id: string_field(&value, "session_id"),
                stop_reason: string_field(&value, "stop_reason"),
            },
            "title_update" => Self::TitleUpdate {
                title: string_field(&value, "title"),
            },
            "tool_approval_required" => Self::ToolApprovalRequired {
                id: string_field(&value, "id"),
                name: string_field(&value, "name"),
                arguments: value.get("arguments").cloned().unwrap_or(Value::Null),
            },
            "tool_approved" => Self::ToolApproved {
                id: string_field(&value, "id"),
            },
            "tool_denied" => Self::ToolDenied {
                id: string_field(&value, "id"),
            },
            "steering_injected" => Self::SteeringInjected {
                pending_id: optional_string_field(&value, "pending_id"),
                message: string_field(&value, "message"),
            },
            "error" => Self::Error {
                error: string_field(&value, "error"),
            },
            "agent_background_started" => Self::AgentBackgroundStarted { payload: value },
            "agent_background_completed" => Self::AgentBackgroundCompleted { payload: value },
            "user_message" => Self::UserMessage {
                title: optional_string_field(&value, "title"),
                message: string_field(&value, "message"),
                level: string_field(&value, "level"),
            },
            "classifier_decision" => Self::ClassifierDecision {
                tool_name: string_field(&value, "tool_name"),
                decision: string_field(&value, "decision"),
                reason: string_field(&value, "reason"),
                stage: usize_field(&value, "stage") as u8,
            },
            "teammate_spawned" => Self::TeammateSpawned {
                name: string_field(&value, "name"),
                role: string_field(&value, "role"),
            },
            "teammate_task_completed" => Self::TeammateTaskCompleted {
                name: string_field(&value, "name"),
                task_id: string_field(&value, "task_id"),
                result: string_field(&value, "result"),
            },
            "teammate_task_failed" => Self::TeammateTaskFailed {
                name: string_field(&value, "name"),
                task_id: string_field(&value, "task_id"),
                error: string_field(&value, "error"),
            },
            "teammate_cancelled" => Self::TeammateCancelled {
                name: string_field(&value, "name"),
            },
            _ => Self::Other {
                event_type,
                payload: value,
            },
        }
    }
}

fn string_field(value: &Value, field: &str) -> String {
    optional_string_field(value, field).unwrap_or_default()
}

fn optional_string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn bool_field(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn usize_field(value: &Value, field: &str) -> usize {
    value.get(field).and_then(Value::as_u64).unwrap_or(0) as usize
}

fn u64_field(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        ChatStreamEvent, FastMode, ModelInfo, ReasoningControl, ReasoningEffort, SessionType,
        ThinkingLevel,
    };
    use serde_json::json;

    #[test]
    fn session_type_accepts_deprecated_wire_value_and_emits_canonical_value() {
        assert_eq!(
            serde_json::from_str::<SessionType>(r#""mako""#).expect("deprecated session type"),
            SessionType::Hive
        );
        assert_eq!(
            serde_json::to_string(&SessionType::Hive).expect("canonical session type"),
            r#""hive""#
        );
    }

    #[test]
    fn usage_parses_explicit_logical_input_tokens() {
        let event = ChatStreamEvent::from_json_value(json!({
            "type": "usage",
            "prompt_tokens": 100,
            "input_tokens": 1_000,
            "completion_tokens": 50,
            "reasoning_tokens": 40,
            "cache_creation_input_tokens": 200,
            "cache_read_input_tokens": 700,
            "total_tokens": 1_050
        }));

        assert!(matches!(
            event,
            ChatStreamEvent::Usage {
                input_tokens: 1_000,
                reasoning_tokens: 40,
                ..
            }
        ));
    }

    #[test]
    fn usage_derives_input_tokens_for_older_servers() {
        let event = ChatStreamEvent::from_json_value(json!({
            "type": "usage",
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "cache_creation_input_tokens": 200,
            "cache_read_input_tokens": 700,
            "total_tokens": 1_050
        }));

        assert!(matches!(
            event,
            ChatStreamEvent::Usage {
                input_tokens: 1_000,
                reasoning_tokens: 0,
                ..
            }
        ));
    }

    #[test]
    fn model_capabilities_drive_reasoning_cycle_and_fast_support() {
        let model = ModelInfo {
            key: None,
            id: "gpt-test".to_string(),
            display_name: "GPT Test".to_string(),
            provider: "OpenAI".to_string(),
            context_window: 128_000,
            max_output: 16_384,
            supports_thinking: true,
            reasoning_control: Some(ReasoningControl::OpenAiEffort),
            supported_reasoning_levels: vec![
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
                ReasoningEffort::Max,
                ReasoningEffort::Ultra,
            ],
            default_reasoning_level: Some(ReasoningEffort::High),
            reasoning_is_mandatory: true,
            supports_fast_mode: true,
            fast_mode: Some(FastMode::Priority),
            supports_tools: true,
            supports_vision: true,
        };

        assert_eq!(
            model.selectable_thinking_levels(),
            vec![
                ThinkingLevel::Minimal,
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
                ThinkingLevel::XHigh,
                ThinkingLevel::Max
            ]
        );
        assert_eq!(
            model.normalize_thinking_level(ThinkingLevel::Off),
            ThinkingLevel::High
        );
        assert_eq!(
            ThinkingLevel::XHigh.cycle_for_model(&model),
            ThinkingLevel::Max
        );
        assert_eq!(
            ThinkingLevel::Max.cycle_for_model(&model),
            ThinkingLevel::Minimal
        );
        assert!(model.supports_fast_mode);
    }

    #[test]
    fn legacy_reasoning_metadata_keeps_a_safe_selectable_fallback() {
        let mut model = ModelInfo {
            key: None,
            id: "legacy-model".to_string(),
            display_name: "Legacy Model".to_string(),
            provider: "Legacy".to_string(),
            context_window: 0,
            max_output: 0,
            supports_thinking: true,
            reasoning_control: Some(ReasoningControl::AnthropicBudget),
            supported_reasoning_levels: Vec::new(),
            default_reasoning_level: None,
            reasoning_is_mandatory: false,
            supports_fast_mode: false,
            fast_mode: None,
            supports_tools: false,
            supports_vision: false,
        };

        assert_eq!(
            model.selectable_thinking_levels(),
            vec![ThinkingLevel::Off, ThinkingLevel::Medium]
        );

        model.reasoning_is_mandatory = true;
        model.default_reasoning_level = Some(ReasoningEffort::High);
        assert_eq!(
            model.selectable_thinking_levels(),
            vec![ThinkingLevel::High]
        );
        assert_eq!(
            model.normalize_thinking_level(ThinkingLevel::Off),
            ThinkingLevel::High
        );

        model.supported_reasoning_levels = vec![ReasoningEffort::None];
        model.default_reasoning_level = None;
        assert_eq!(
            model.selectable_thinking_levels(),
            vec![ThinkingLevel::Medium]
        );

        model.reasoning_control = Some(ReasoningControl::OutputOnly);
        assert_eq!(model.selectable_thinking_levels(), vec![ThinkingLevel::Off]);
        assert_eq!(
            model.normalize_thinking_level(ThinkingLevel::High),
            ThinkingLevel::Off
        );
    }
}
