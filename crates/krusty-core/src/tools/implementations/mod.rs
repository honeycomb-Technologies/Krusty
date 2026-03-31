//! Tool implementations
//!
//! Core tools:
//! - read: Read files
//! - write: Create/overwrite files
//! - edit: Edit specific lines with fuzzy matching
//! - multiedit: Multiple edits to one file
//! - bash: Execute shell commands
//! - grep: Search with ripgrep
//! - glob: Find files by pattern
//! - list: List directory contents
//! - apply_patch: Multi-file patch application
//! - processes: Manage background processes
//! - agent: Unified sub-agent dispatch (explore, plan, verify, build)
//! - skill: Invoke skills for specialized instructions
//! - ask_user: Interactive user prompts (handled by UI)
//! - task_complete: Mark plan tasks as complete with result (handled by UI)
//! - task_start: Mark task as in-progress (handled by UI)
//! - add_subtask: Create subtasks for task breakdown (handled by UI)
//! - set_dependency: Create task dependencies (handled by UI)
//! - set_work_mode: Switch between build and plan modes (handled by UI/server)
//! - enter_plan_mode: Switch to plan mode (handled by UI)

pub mod add_subtask;
pub mod agent;
pub mod apply_patch;
pub mod ask_user;
pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod list;
pub mod memory;
pub mod multiedit;
pub mod plan_mode;
pub mod processes;
pub mod read;
pub mod set_dependency;
pub mod set_work_mode;
pub mod set_workspace_context;
pub mod skill;
pub mod task_complete;
pub mod task_start;
pub mod write;

pub use add_subtask::AddSubtaskTool;
pub use agent::AgentTool;
pub use apply_patch::ApplyPatchTool;
pub use ask_user::AskUserQuestionTool;
pub use bash::BashTool;
pub use edit::EditTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use list::ListTool;
pub use memory::MemoryTool;
pub use multiedit::MultiEditTool;
pub use plan_mode::EnterPlanModeTool;
pub use processes::ProcessesTool;
pub use read::ReadTool;
pub use set_dependency::SetDependencyTool;
pub use set_work_mode::SetWorkModeTool;
pub use set_workspace_context::SetWorkspaceContextTool;
pub use skill::SkillTool;
pub use task_complete::TaskCompleteTool;
pub use task_start::TaskStartTool;
pub use write::WriteTool;

use std::sync::Arc;

use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::tools::registry::ToolRegistry;

/// Register all built-in tools (except agent which needs client)
pub async fn register_all_tools(registry: &ToolRegistry) {
    registry.register(Arc::new(ReadTool)).await;
    registry.register(Arc::new(WriteTool)).await;
    registry.register(Arc::new(EditTool)).await;
    registry.register(Arc::new(MultiEditTool)).await;
    registry.register(Arc::new(BashTool)).await;
    registry.register(Arc::new(GrepTool)).await;
    registry.register(Arc::new(GlobTool)).await;
    registry.register(Arc::new(ListTool)).await;
    registry.register(Arc::new(ApplyPatchTool)).await;
    registry.register(Arc::new(ProcessesTool)).await;
    registry.register(Arc::new(SkillTool)).await;
    registry.register(Arc::new(MemoryTool)).await;
    registry.register(Arc::new(AskUserQuestionTool)).await;
    registry.register(Arc::new(TaskCompleteTool)).await;
    registry.register(Arc::new(TaskStartTool)).await;
    registry.register(Arc::new(AddSubtaskTool)).await;
    registry.register(Arc::new(SetDependencyTool)).await;
    registry.register(Arc::new(SetWorkspaceContextTool)).await;
    registry.register(Arc::new(SetWorkModeTool)).await;
    registry.register(Arc::new(EnterPlanModeTool)).await;
}

/// Register tools for ACP (excludes TUI-only tools)
///
/// Excludes:
/// - AskUserQuestionTool (requires TUI interaction)
/// - TaskCompleteTool (requires TUI plan mode)
/// - EnterPlanModeTool (requires TUI plan mode)
/// - SkillTool (requires skills manager setup)
pub async fn register_acp_tools(registry: &ToolRegistry) {
    registry.register(Arc::new(ReadTool)).await;
    registry.register(Arc::new(WriteTool)).await;
    registry.register(Arc::new(EditTool)).await;
    registry.register(Arc::new(MultiEditTool)).await;
    registry.register(Arc::new(BashTool)).await;
    registry.register(Arc::new(GrepTool)).await;
    registry.register(Arc::new(GlobTool)).await;
    registry.register(Arc::new(ListTool)).await;
    registry.register(Arc::new(ApplyPatchTool)).await;
    registry.register(Arc::new(ProcessesTool)).await;
}

/// Register the unified agent tool (explore, plan, verify, build)
///
/// Call this after authentication when the AI client is available.
pub async fn register_agent_tool(
    registry: &ToolRegistry,
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
) {
    registry
        .register(Arc::new(AgentTool::new(client, cancellation)))
        .await;
}
