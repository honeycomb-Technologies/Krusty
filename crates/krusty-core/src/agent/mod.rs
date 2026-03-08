//! Agent system for Krusty
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
//! ## Builder Swarm (Octopod)
//! - `SharedBuildContext` - Coordination for builder agents
//! - Type registry, file locks, conventions

pub mod build_context;
pub mod cache;
pub mod cancellation;
pub mod constants;
pub mod context;
pub mod event_bus;
pub mod events;
pub mod executor;
pub mod failure;
pub mod hooks;
pub mod loop_events;
pub mod orchestrator;
pub mod pinch_context;
pub mod plan_handler;
pub mod state;
pub mod stream;
pub mod subagent;
pub mod summarizer;
pub mod user_hooks;

pub use build_context::SharedBuildContext;
pub use cancellation::AgentCancellation;
pub use context::{
    build_plan_context, build_project_context, build_skills_context, inject_context,
};
pub use event_bus::AgentEventBus;
pub use events::{AgentEvent, InterruptReason};
pub use hooks::{LoggingHook, PlanModeHook, SafetyHook};
pub use loop_events::{LoopEvent, LoopInput, PlanTaskInfo};
pub use orchestrator::{AgenticOrchestrator, OrchestratorConfig, OrchestratorServices};
pub use pinch_context::{PinchContext, PinchContextInput};
pub use state::{AgentConfig, AgentState};
pub use summarizer::{generate_summary, SummarizationResult};
pub use user_hooks::{
    UserHook, UserHookExecutor, UserHookManager, UserHookResult, UserHookType, UserPostToolHook,
    UserPreToolHook,
};
