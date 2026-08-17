//! Agent system for Mitsuro
//!
//! ## Orchestrator (the canonical agentic loop)
//! - `AgenticOrchestrator` - Unified loop: streaming, tools, plans, failure detection
//! - `LoopEvent` / `LoopInput` - Event protocol between orchestrator and consumers
//! - `OrchestratorConfig` / `OrchestratorServices` - Configuration and dependencies
//!
//! ## Core Components
//! - `AgentEventBus` - Central event dispatcher
//! - `AgentState` - Turn tracking and execution state
//! - `AgentCancellation` - Proper task cancellation
//!
//! ## Hooks
//! - `SafetyHook` - Blocks dangerous bash commands
//! - `LoggingHook` - Logs all tool executions
//! - `UserHookManager` - User-configurable hooks
//!
//! ## Pinch (Context Continuation)
//! - `PinchContext` - Structured context for session transitions
//! - `SummarizationResult` - Output from summarization agent
//!
//! ## Sub-agents
//! - `SubAgentPool` - Concurrent execution of lightweight agents
//! - `SubAgentTask` - Task configuration for sub-agents
//!
//! ## Autonomy (Hive)
//! - `TickEngine` - Autonomous wake/sleep driver for Hive sessions
//! - `coordinator_prompt` - Hive coordinator system prompt surface
//! - dynamic delegated agents - Background work through AgentSpec and lifecycle controls
//! - `AutoClassifierHook` - Autonomous tool-call guardrail hook
//!
//! ## Builder Swarm (Octopod)
//! - `subagent::build_context` - Shared coordination for builder agents
//! - Type registry, file locks, conventions

pub mod agent_types;
pub mod autonomy;
pub mod cancellation;
pub mod compaction;
pub mod constants;
pub mod context;
pub mod context_ledger;
pub mod delegation;
pub mod event_bus;
pub mod events;
pub mod executor;
pub mod failure;
pub mod history_policy;
pub mod hooks;
pub mod learning;
pub mod loop_events;
mod observability;
mod orchestrator;
pub mod pinch_context;
pub mod pinch_session;
pub mod plan_handler;
pub mod progress;
pub mod run_spec;
pub mod state;
pub mod stream;
pub mod subagent;
pub mod summarizer;
mod tool_control;
pub mod user_hooks;

use serde::{Deserialize, Serialize};

pub use autonomy::auto_classifier;
pub use autonomy::auto_classifier::AutoClassifierHook;
pub use autonomy::{coordinator_prompt, tick_engine};
pub use cancellation::AgentCancellation;
pub(crate) use compaction::estimate_with_usage as estimate_tokens_with_usage;
pub use compaction::{
    effective_context_window_for_runtime, estimate_rendered_request_tokens,
    run_compaction_pipeline, CompactionManager, CompactionRequest, CompactionRequestBudget,
    CompactionResult, CompactionTrigger, RenderedRequestTokenEstimate,
};
pub use context::{
    build_plan_context, build_project_context, build_skills_context, inject_context,
};
pub use delegation::{
    CoordinatedSynthesisOwnerFence, CoordinatedSynthesisPermit, CoordinatedTaskPermit,
    DelegationCoordinator, DelegationTaskOutcome,
};
pub use event_bus::AgentEventBus;
pub use events::{AgentEvent, InterruptReason};
pub use hooks::{LoggingHook, PlanModeHook, SafetyHook};
pub use loop_events::{LoopEvent, LoopInput, PlanTaskInfo, ProviderRequestSnapshot};
pub use observability::ProviderCallTraceContext;
pub use orchestrator::OrchestratorServices;
pub use pinch_context::{PinchContext, PinchContextInput};
pub use pinch_session::{
    create_pinched_session, CreatePinchedSessionRequest, CreatePinchedSessionResult,
};
pub use progress::{ActionClass, ProgressGuardAction, ProgressGuardTelemetry, ProgressLedger};
pub use run_spec::{RunKernel, RunProvenance, RunSpec, RunSpecBuilder, RunSpecError};
pub use state::{AgentConfig, AgentState, RunBudget, RunBudgetResolution, RunBudgetSource};
pub use subagent::build_context;
pub use subagent::build_context::SharedBuildContext;
pub use summarizer::{generate_summary, SummarizationResult};
pub use user_hooks::{
    PackageHookConfig, PackageHookLoadReport, UserHook, UserHookExecutor, UserHookManager,
    UserHookResult, UserHookSource, UserHookType, UserPostToolHook, UserPreToolHook,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedToolKind {
    Explore,
    Build,
    Plan,
    Verify,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedProgressEvent {
    pub delegated_run_id: String,
    pub parent_session_id: String,
    pub tool_call_id: String,
    pub kind: DelegatedToolKind,
    pub stage: DelegatedRunStage,
    pub progress: subagent::AgentProgress,
}
