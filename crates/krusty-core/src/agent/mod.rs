//! Agent system for Krusty
//!
//! Provides centralized event handling, state tracking, and control
//! for the AI agent loop.
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
pub mod dual_mind;
pub mod event_bus;
pub mod events;
pub mod hooks;
pub mod pinch_context;
pub mod state;
pub mod subagent;
pub mod summarizer;
pub mod user_hooks;

pub use build_context::SharedBuildContext;
pub use cancellation::AgentCancellation;
pub use event_bus::AgentEventBus;
pub use events::{AgentEvent, InterruptReason};
pub use hooks::{LoggingHook, PlanModeHook, SafetyHook};
pub use pinch_context::PinchContext;
pub use state::{AgentConfig, AgentState};
pub use summarizer::{generate_summary, SummarizationResult};
pub use user_hooks::{
    UserHook, UserHookExecutor, UserHookManager, UserHookResult, UserHookType, UserPostToolHook,
    UserPreToolHook,
};

// Dual-mind system (Big Claw / Little Claw)
pub use dual_mind::{
    ClawRole, DialogueManager, DialogueResult, DialogueTurn, DualMind, DualMindBuilder,
    DualMindConfig, LittleClaw, Observation, Speaker,
};
