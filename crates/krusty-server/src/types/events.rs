use krusty_core::agent::subagent::AgentProgressStatus;
use krusty_core::agent::{
    DelegatedProgressEvent, DelegatedRunStage as CoreDelegatedRunStage,
    DelegatedToolKind as CoreDelegatedToolKind,
};
use krusty_core::ai::types::{Citation, WebFetchContent, WebSearchResult};
use krusty_core::storage::RuntimeTraceEvent;
use serde::Serialize;

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
        cache_creation_input_tokens: usize,
        cache_read_input_tokens: usize,
        total_tokens: usize,
    },
    /// Active work was handed off into a linked continuation session.
    SessionPinched {
        reason: String,
        source_session_id: String,
        new_session_id: String,
        estimated_tokens_before: usize,
    },
    /// In-place compaction started.
    ContextCompactionStarted { reason: String },
    /// Conversation was compacted in place.
    ContextCompacted {
        reason: String,
        estimated_tokens_before: usize,
        estimated_tokens_after: usize,
        replaced_messages: usize,
        checkpoint_id: String,
        compaction_count: u32,
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
            "session_pinched" => Some(Self::SessionPinched {
                reason: payload.get("reason")?.as_str()?.to_string(),
                source_session_id: payload.get("source_session_id")?.as_str()?.to_string(),
                new_session_id: payload.get("new_session_id")?.as_str()?.to_string(),
                estimated_tokens_before: payload.get("estimated_tokens_before")?.as_u64()? as usize,
            }),
            "context_compaction_started" => Some(Self::ContextCompactionStarted {
                reason: payload.get("reason")?.as_str()?.to_string(),
            }),
            "context_compacted" => Some(Self::ContextCompacted {
                reason: payload.get("reason")?.as_str()?.to_string(),
                estimated_tokens_before: payload.get("estimated_tokens_before")?.as_u64()? as usize,
                estimated_tokens_after: payload.get("estimated_tokens_after")?.as_u64()? as usize,
                replaced_messages: payload.get("replaced_messages")?.as_u64()? as usize,
                checkpoint_id: payload.get("checkpoint_id")?.as_str()?.to_string(),
                compaction_count: payload.get("compaction_count")?.as_u64()? as u32,
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
                cache_creation_input_tokens,
                cache_read_input_tokens,
                total_tokens,
            } => Self::Usage {
                prompt_tokens,
                completion_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                total_tokens,
            },
            LoopEvent::SessionPinched {
                reason,
                source_session_id,
                new_session_id,
                estimated_tokens_before,
            } => Self::SessionPinched {
                reason,
                source_session_id,
                new_session_id,
                estimated_tokens_before,
            },
            LoopEvent::ContextCompactionStarted { reason } => {
                Self::ContextCompactionStarted { reason }
            }
            LoopEvent::ContextCompacted {
                reason,
                estimated_tokens_before,
                estimated_tokens_after,
                replaced_messages,
                checkpoint_id,
                compaction_count,
            } => Self::ContextCompacted {
                reason,
                estimated_tokens_before,
                estimated_tokens_after,
                replaced_messages,
                checkpoint_id,
                compaction_count,
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
