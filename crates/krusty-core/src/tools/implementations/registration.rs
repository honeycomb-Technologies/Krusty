use std::sync::Arc;

use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::tools::registry::ToolRegistry;

use super::{
    AddSubtaskTool, AgentTool, ApplyPatchTool, AskUserQuestionTool, AutonomousTaskTool, BashTool,
    EditTool, EnterPlanModeTool, GlobTool, GrepTool, ListTool, MemoryTool, MultiEditTool,
    ProcessesTool, ReadTool, ReportTool, SearchCompactionSegmentsTool, SendUserMessageTool,
    SetDependencyTool, SetWorkModeTool, SetWorkspaceContextTool, SkillTool, SleepTool,
    TaskCompleteTool, TaskStartTool, ToolSearchTool, WebFetchTool, WebSearchTool, WriteTool,
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
    registry.register(Arc::new(ToolSearchTool)).await;
    registry.register(Arc::new(SendUserMessageTool)).await;
    registry.register(Arc::new(SleepTool)).await;
}

/// Register tools for ACP.
///
/// ACP relays supervised approvals through the editor, but an approval dialog is
/// not an operating-system sandbox. Do not expose tools that can execute arbitrary
/// host commands or manage long-lived host processes until ACP can give them an
/// isolated execution boundary. File tools remain path-scoped to the canonical
/// session workspace. Read-only web tools are also exposed so ACP sessions can
/// answer current-information questions without shell access.
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
    registry.register(Arc::new(ToolSearchTool)).await;
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
    use std::sync::Arc;

    use super::*;
    use crate::agent::AgentCancellation;
    use crate::ai::client::{AiClient, AiClientConfig, KRUSTY_SYSTEM_PROMPT};
    use crate::ai::format::get_format_handler;
    use crate::ai::models::ApiFormat;
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
        assert!(registry.get("tool_search").await.is_some());
        assert!(registry.get("bash").await.is_none());
        assert!(registry.get("processes").await.is_none());
    }

    #[tokio::test]
    async fn default_wire_surface_is_bounded_but_catalog_remains_reachable() {
        let registry = ToolRegistry::new();
        register_all_tools(&registry).await;
        register_agent_tool(
            &registry,
            Arc::new(AiClient::new(AiClientConfig::default(), String::new())),
            AgentCancellation::new(),
        )
        .await;

        let wire_tools = registry.get_ai_tools().await;
        let catalog = registry.get_ai_tools_all().await;

        assert!(wire_tools.len() <= crate::tools::registry::DEFAULT_CODE_TOOL_LIMIT);
        assert!(wire_tools.iter().any(|tool| tool.name == "tool_search"));
        assert!(catalog.len() > wire_tools.len());
        assert!(catalog.iter().any(|tool| tool.name == "memory"));
        assert!(catalog.iter().all(|tool| tool.prompt.is_none()));

        let provider_tools =
            get_format_handler(ApiFormat::OpenAIResponses).convert_tools(&wire_tools);
        for (tool, provider_tool) in wire_tools.iter().zip(provider_tools.iter()) {
            println!(
                "tool_schema name={} bytes={}",
                tool.name,
                serde_json::to_vec(provider_tool).unwrap().len()
            );
        }
        let tool_bytes = serde_json::to_vec(&provider_tools).unwrap().len();
        let base_prompt_bytes = KRUSTY_SYSTEM_PROMPT.len();
        let fixed_bytes = base_prompt_bytes + tool_bytes;
        let estimated_tokens = fixed_bytes.div_ceil(4);
        println!(
            "default_tool_count={} base_prompt_bytes={} tool_bytes={} fixed_bytes={} estimated_tokens={}",
            wire_tools.len(),
            base_prompt_bytes,
            tool_bytes,
            fixed_bytes,
            estimated_tokens
        );
        assert!(
            estimated_tokens <= 2_000,
            "fixed no-project budget exceeded: tool_count={} base_prompt_bytes={} tool_bytes={} fixed_bytes={} estimated_tokens={} ceiling=2000",
            wire_tools.len(),
            base_prompt_bytes,
            tool_bytes,
            fixed_bytes,
            estimated_tokens
        );
    }
}
