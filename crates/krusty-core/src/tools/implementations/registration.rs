use std::sync::Arc;

use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::tools::registry::ToolRegistry;

use super::{
    AddSubtaskTool, AgentTool, ApplyPatchTool, AskUserQuestionTool, AutonomousTaskTool, BashTool,
    EditTool, EnterPlanModeTool, GlobTool, GrepTool, ListTool, MemoryTool, MultiEditTool,
    ProcessesTool, ReadTool, ReportTool, SendUserMessageTool, SetDependencyTool, SetWorkModeTool,
    SetWorkspaceContextTool, SkillTool, SleepTool, TaskCompleteTool, TaskStartTool, WriteTool,
};

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
    registry.register(Arc::new(SendUserMessageTool)).await;
    registry.register(Arc::new(SleepTool)).await;
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

/// Register Mako-specific tools (autonomous tasks and reports).
///
/// These are additive — call after `register_all_tools` so Mako sessions
/// get both the standard Code tools and the Mako extensions. Mako delegates
/// through the `agent` tool; the legacy mailbox `send_message` path is not
/// part of the autonomous contract.
pub async fn register_mako_tools(registry: &ToolRegistry) {
    registry.register(Arc::new(AutonomousTaskTool)).await;
    registry.register(Arc::new(ReportTool)).await;
}
