use krusty_core::ai::client::{
    AiClient, AnthropicAdaptiveEffort, CallOptions, CodexReasoningEffort,
};
use krusty_core::ai::model_profile::{ModelProfile, PromptFamily};
use krusty_core::ai::models::ApiFormat;
use krusty_core::ai::providers::{get_model_family, ModelFamily, ProviderId};
use krusty_core::ai::types::{AiTool, ThinkingConfig};
use krusty_core::storage::SessionType;

use crate::types::ThinkingLevel;

/// Additional tools unlocked for Chat sessions when research mode is enabled.
const CHAT_RESEARCH_TOOLS: &[&str] = &["agent", "report"];

/// Tools exclusive to Mako sessions -- excluded from Code sessions.
const MAKO_ONLY_TOOLS: &[&str] = &["send_user_message", "sleep", "autonomous_task", "report"];

/// Filter tools based on the session type.
///
/// - **Code**: all registered tools except Mako-only tools.
/// - **Chat**: only the safe chat subset. When `research_enabled` is true,
///   the agent and report tools are included.
/// - **Mako**: all registered tools (Code tools + Mako extensions), executed
///   through the autonomous wake-driven runtime.
pub(super) fn filter_tools_for_session_type(
    tools: Vec<AiTool>,
    session_type: SessionType,
    research_enabled: bool,
) -> Vec<AiTool> {
    let before = tools.len();
    let result = filter_tools_inner(tools, session_type, research_enabled);
    tracing::info!(
        session_type = ?session_type,
        before_count = before,
        after_count = result.len(),
        tool_names = ?result.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        "Tool filter applied"
    );
    result
}

fn filter_tools_inner(
    tools: Vec<AiTool>,
    session_type: SessionType,
    research_enabled: bool,
) -> Vec<AiTool> {
    tools
        .into_iter()
        .filter(|tool| tool_allowed_for_session(&tool.name, session_type, research_enabled))
        .collect()
}

fn tool_allowed_for_session(
    tool_name: &str,
    session_type: SessionType,
    research_enabled: bool,
) -> bool {
    match session_type {
        SessionType::Code => !is_mako_only_tool(tool_name),
        SessionType::Chat => {
            is_base_chat_tool(tool_name) || (research_enabled && is_chat_research_tool(tool_name))
        }
        SessionType::Mako => true,
    }
}

fn is_base_chat_tool(tool_name: &str) -> bool {
    matches!(tool_name, "web_search" | "web_fetch")
}

fn is_chat_research_tool(tool_name: &str) -> bool {
    CHAT_RESEARCH_TOOLS.contains(&tool_name)
}

fn is_mako_only_tool(tool_name: &str) -> bool {
    MAKO_ONLY_TOOLS.contains(&tool_name)
}

pub(super) fn chat_system_prompt(research_enabled: bool) -> String {
    let tool_clause = if research_enabled {
        "You may use web_search and web_fetch for web research, and you may also use agent and report when deeper research or synthesis is needed."
    } else {
        "You may use web_search and web_fetch for web research when it would help the user."
    };

    format!(
        "You are Krusty, a friendly conversational assistant. This is a chat session. {tool_clause} You do not have direct file, shell, git, or local code-editing tools in this session. Do not claim capabilities you do not have. If the user needs hands-on coding or workspace changes, suggest switching to Code mode. Be helpful, natural, and conversational."
    )
}

pub(super) fn apply_thinking_config(
    ai_client: &AiClient,
    thinking_level: ThinkingLevel,
    options: &mut CallOptions,
) {
    if !thinking_level.is_enabled() {
        return;
    }

    let cfg = ai_client.config();
    options.thinking = Some(ThinkingConfig::default());

    if supports_codex_reasoning(cfg.provider_id, cfg.api_format, &cfg.model) {
        options.codex_reasoning_effort = Some(match thinking_level {
            ThinkingLevel::Off => return,
            ThinkingLevel::Low => CodexReasoningEffort::Low,
            ThinkingLevel::Medium => CodexReasoningEffort::Medium,
            ThinkingLevel::High => CodexReasoningEffort::High,
            ThinkingLevel::XHigh => CodexReasoningEffort::XHigh,
        });
    } else if supports_anthropic_adaptive_effort(cfg.provider_id, &cfg.model) {
        options.anthropic_adaptive_effort = Some(match thinking_level {
            ThinkingLevel::Off => return,
            ThinkingLevel::Low => AnthropicAdaptiveEffort::Low,
            ThinkingLevel::Medium => AnthropicAdaptiveEffort::Medium,
            ThinkingLevel::High => AnthropicAdaptiveEffort::High,
            ThinkingLevel::XHigh => AnthropicAdaptiveEffort::Max,
        });
    }
}

fn supports_codex_reasoning(
    provider_id: ProviderId,
    api_format: ApiFormat,
    model_id: &str,
) -> bool {
    ModelProfile::resolve(provider_id, api_format, model_id).prompt_family
        == PromptFamily::OpenAiCodex
}

fn supports_anthropic_adaptive_effort(provider_id: ProviderId, model_id: &str) -> bool {
    provider_id == ProviderId::Anthropic && is_claude_opus_4_6_family(model_id)
}

fn is_claude_opus_4_6_family(model_id: &str) -> bool {
    let normalized = normalize_model_id(model_id);
    get_model_family(normalized.as_str()) == Some(ModelFamily::ClaudeOpus4_6)
        || normalized.contains("claude-opus-4-6")
        || normalized.starts_with("opus-4-6")
}

fn normalize_model_id(model_id: &str) -> String {
    model_id
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '.'], "-")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool(name: &str) -> AiTool {
        AiTool {
            name: name.to_string(),
            description: format!("{name} description"),
            input_schema: json!({"type": "object"}),
            prompt: None,
        }
    }

    #[test]
    fn chat_system_prompt_matches_default_chat_tool_policy() {
        let prompt = chat_system_prompt(false);

        assert!(prompt.contains("web_search and web_fetch"));
        assert!(!prompt.contains("agent and report when deeper research"));
        assert!(prompt.contains("switching to Code mode"));
    }

    #[test]
    fn chat_system_prompt_mentions_research_tools_when_enabled() {
        let prompt = chat_system_prompt(true);

        assert!(prompt.contains("web_search and web_fetch"));
        assert!(prompt.contains("agent and report when deeper research or synthesis is needed"));
    }

    #[test]
    fn chat_policy_only_allows_base_tools_by_default() {
        let filtered = filter_tools_inner(
            vec![
                tool("web_search"),
                tool("web_fetch"),
                tool("agent"),
                tool("bash"),
            ],
            SessionType::Chat,
            false,
        );

        let names = filtered
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["web_search", "web_fetch"]);
    }

    #[test]
    fn chat_research_policy_unlocks_research_tools() {
        let filtered = filter_tools_inner(
            vec![
                tool("web_search"),
                tool("agent"),
                tool("report"),
                tool("bash"),
            ],
            SessionType::Chat,
            true,
        );

        let names = filtered
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["web_search", "agent", "report"]);
    }

    #[test]
    fn code_policy_excludes_mako_only_tools() {
        let filtered = filter_tools_inner(
            vec![tool("bash"), tool("sleep"), tool("report")],
            SessionType::Code,
            false,
        );

        let names = filtered
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["bash"]);
    }

    #[test]
    fn mako_policy_keeps_all_tools() {
        let filtered = filter_tools_inner(
            vec![tool("bash"), tool("sleep"), tool("report")],
            SessionType::Mako,
            false,
        );

        let names = filtered
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["bash", "sleep", "report"]);
    }

    #[test]
    fn codex_reasoning_support_uses_shared_model_profile_resolution() {
        assert!(supports_codex_reasoning(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.3-codex"
        ));
        assert!(!supports_codex_reasoning(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.4"
        ));
    }

    #[test]
    fn anthropic_adaptive_effort_support_accepts_versioned_opus_variants() {
        assert!(supports_anthropic_adaptive_effort(
            ProviderId::Anthropic,
            "claude-opus-4.6-20250320"
        ));
        assert!(supports_anthropic_adaptive_effort(
            ProviderId::Anthropic,
            " claude opus 4.6 "
        ));
        assert!(!supports_anthropic_adaptive_effort(
            ProviderId::OpenRouter,
            "anthropic/claude-opus-4.6"
        ));
        assert!(!supports_anthropic_adaptive_effort(
            ProviderId::Anthropic,
            "claude-sonnet-4.5"
        ));
    }
}
