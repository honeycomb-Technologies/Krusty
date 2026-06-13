use std::sync::Arc;

use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::tools::registry::ToolRegistry;

use super::{
    AddSubtaskTool, AgentTool, ApplyPatchTool, AskUserQuestionTool, AutonomousTaskTool, BashTool,
    EditTool, EnterPlanModeTool, GlobTool, GrepTool, ListTool, MemoryTool, MultiEditTool,
    ProcessesTool, ReadTool, ReportTool, SearchCompactionSegmentsTool, SendUserMessageTool,
    SetDependencyTool, SetWorkModeTool, SetWorkspaceContextTool, SkillTool, SleepTool,
    TaskCompleteTool, TaskStartTool, WebFetchTool, WebSearchTool, WriteTool,
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
    registry.register(Arc::new(WebSearchTool)).await;
    registry.register(Arc::new(WebFetchTool)).await;
    registry.register(Arc::new(ApplyPatchTool)).await;
    registry.register(Arc::new(ProcessesTool)).await;
    registry.register(Arc::new(SkillTool)).await;
    registry.register(Arc::new(MemoryTool)).await;
    registry
        .register(Arc::new(SearchCompactionSegmentsTool))
        .await;
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

/// Register tools for ACP.
///
/// ACP currently has no editor-backed user approval flow, so do not expose tools
/// that can execute arbitrary commands or manage long-lived host processes. File
/// tools remain available and are constrained to the session workspace by the ACP
/// processor's sandboxed [`ToolContext`]. Read-only web tools are also exposed
/// so ACP sessions can answer current-information questions without shell access.
pub async fn register_acp_tools(registry: &ToolRegistry) {
    registry.register(Arc::new(ReadTool)).await;
    registry.register(Arc::new(WriteTool)).await;
    registry.register(Arc::new(EditTool)).await;
    registry.register(Arc::new(MultiEditTool)).await;
    registry.register(Arc::new(GrepTool)).await;
    registry.register(Arc::new(GlobTool)).await;
    registry.register(Arc::new(ListTool)).await;
    registry.register(Arc::new(WebSearchTool)).await;
    registry.register(Arc::new(WebFetchTool)).await;
    registry.register(Arc::new(ApplyPatchTool)).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::ToolRegistry;

    #[tokio::test]
    async fn acp_tools_exclude_unsandboxable_host_execution_tools() {
        let registry = ToolRegistry::new();

        register_acp_tools(&registry).await;

        assert!(registry.get("read").await.is_some());
        assert!(registry.get("write").await.is_some());
        assert!(registry.get("edit").await.is_some());
        assert!(registry.get("grep").await.is_some());
        assert!(registry.get("glob").await.is_some());
        assert!(registry.get("web_search").await.is_some());
        assert!(registry.get("web_fetch").await.is_some());
        assert!(registry.get("bash").await.is_none());
        assert!(registry.get("processes").await.is_none());
    }
}
