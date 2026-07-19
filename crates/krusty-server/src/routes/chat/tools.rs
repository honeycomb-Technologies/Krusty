use krusty_core::ai::client::{AnthropicAdaptiveEffort, CallOptions, CodexReasoningEffort};
use krusty_core::ai::providers::ReasoningControl;
use krusty_core::ai::types::{AiTool, ThinkingConfig};
use krusty_core::storage::SessionType;

use crate::types::ThinkingLevel;

/// Tools exclusive to Mako sessions -- excluded from Code sessions.
const MAKO_ONLY_TOOLS: &[&str] = &["send_user_message", "sleep", "autonomous_task", "report"];

/// Return true only when a Code turn is deterministically non-tool-bearing.
///
/// The system prompt carries the general policy, while this request-level guard
/// prevents providers from inventing workspace-orientation calls for greetings
/// or disobeying an explicit, leading no-tool response directive. Structured
/// content always retains tools because it can replace the fallback message
/// with an attachment-derived task that this predicate has not inspected.
pub(super) fn should_suppress_code_tools(message: &str, has_structured_content: bool) -> bool {
    if has_structured_content {
        return false;
    }

    is_explicit_no_tool_response_directive(message) || is_narrow_casual_greeting(message)
}

/// Match an imperative only when it begins the message and immediately asks
/// for a response. This intentionally does not search the whole message: task
/// text that quotes or discusses the same words must keep the Code toolset.
fn is_explicit_no_tool_response_directive(message: &str) -> bool {
    let normalized = message.trim_start().to_ascii_lowercase();
    const NO_TOOL_PREFIXES: &[&str] = &[
        "without calling any tools",
        "without calling any tool",
        "without calling a tool",
        "without calling tools",
        "without using any tools",
        "without using any tool",
        "without using a tool",
        "without using tools",
        "do not call any tools",
        "do not call any tool",
        "do not call a tool",
        "do not call tools",
        "do not use any tools",
        "do not use any tool",
        "do not use a tool",
        "do not use tools",
        "don't call any tools",
        "don't call any tool",
        "don't call a tool",
        "don't call tools",
        "don't use any tools",
        "don't use any tool",
        "don't use a tool",
        "don't use tools",
        "don’t call any tools",
        "don’t call any tool",
        "don’t call a tool",
        "don’t call tools",
        "don’t use any tools",
        "don’t use any tool",
        "don’t use a tool",
        "don’t use tools",
        "no tool calls",
        "no tools",
    ];

    NO_TOOL_PREFIXES.iter().any(|prefix| {
        let Some(remainder) = normalized.strip_prefix(prefix) else {
            return false;
        };

        // Reject a prefix that merely starts a longer word (for example,
        // "without calling any toolchain...").
        if remainder.chars().next().is_some_and(char::is_alphanumeric) {
            return false;
        }

        let remainder = remainder.trim_start_matches(|character: char| {
            character.is_whitespace()
                || matches!(character, ',' | '.' | ':' | ';' | '-' | '\u{2014}')
        });
        let remainder = ["please ", "just ", "simply "]
            .iter()
            .find_map(|lead_in| remainder.strip_prefix(lead_in))
            .unwrap_or(remainder);

        ["reply", "respond", "answer", "say", "output", "return"]
            .iter()
            .any(|verb| {
                remainder.strip_prefix(verb).is_some_and(|after_verb| {
                    after_verb
                        .chars()
                        .next()
                        .is_none_or(|character| !character.is_alphanumeric())
                })
            })
    })
}

fn is_narrow_casual_greeting(message: &str) -> bool {
    let normalized = message.trim().to_lowercase();
    if normalized.is_empty() || normalized.len() > 80 {
        return false;
    }

    let tokens = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() || tokens.len() > 5 {
        return false;
    }

    const INTENT_WORDS: &[&str] = &[
        "hi",
        "hello",
        "hey",
        "yo",
        "sup",
        "howdy",
        "morning",
        "afternoon",
        "evening",
        "thanks",
        "thank",
    ];
    const CASUAL_WORDS: &[&str] = &[
        "hi",
        "hello",
        "hey",
        "yo",
        "sup",
        "howdy",
        "good",
        "morning",
        "afternoon",
        "evening",
        "what",
        "whats",
        "s",
        "up",
        "thanks",
        "thank",
        "you",
        "there",
        "boss",
        "sir",
        "buddy",
        "dude",
        "man",
        "krusty",
    ];

    tokens.iter().any(|token| INTENT_WORDS.contains(token))
        && tokens.iter().all(|token| CASUAL_WORDS.contains(token))
}

/// Filter tools based on the session type.
///
/// - **Code**: all registered tools except Mako-only tools.
/// - **Chat**: only the safe conversational web subset.
/// - **Mako**: all registered tools (Code tools + Mako extensions), executed
///   through the autonomous wake-driven runtime.
pub(super) fn filter_tools_for_session_type(
    tools: Vec<AiTool>,
    session_type: SessionType,
) -> Vec<AiTool> {
    let before = tools.len();
    let result = filter_tools_inner(tools, session_type);
    tracing::info!(
        session_type = ?session_type,
        before_count = before,
        after_count = result.len(),
        tool_names = ?result.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        "Tool filter applied"
    );
    result
}

fn filter_tools_inner(tools: Vec<AiTool>, session_type: SessionType) -> Vec<AiTool> {
    tools
        .into_iter()
        .filter(|tool| tool_allowed_for_session(&tool.name, session_type))
        .collect()
}

fn tool_allowed_for_session(tool_name: &str, session_type: SessionType) -> bool {
    match session_type {
        SessionType::Code => !is_mako_only_tool(tool_name),
        SessionType::Chat => is_base_chat_tool(tool_name),
        SessionType::Mako => true,
    }
}

fn is_base_chat_tool(tool_name: &str) -> bool {
    matches!(tool_name, "web_search" | "web_fetch")
}

fn is_mako_only_tool(tool_name: &str) -> bool {
    MAKO_ONLY_TOOLS.contains(&tool_name)
}

pub(super) fn chat_system_prompt() -> String {
    "You are Krusty, a friendly conversational assistant. This is a chat session. You may use web_search and web_fetch for web research whenever they would improve accuracy or usefulness. Present the result directly in the conversation rather than creating a separate report or paper. You do not have direct file, shell, git, or local code-editing tools in this session. Do not claim capabilities you do not have. If the user needs hands-on coding or workspace changes, suggest starting a Code conversation. Be helpful, natural, and conversational."
        .to_string()
}

pub(super) fn apply_thinking_config(thinking_level: ThinkingLevel, options: &mut CallOptions) {
    if !thinking_level.is_enabled() {
        return;
    }

    options.thinking = Some(ThinkingConfig::default());

    match options.reasoning_control {
        Some(ReasoningControl::OpenAiEffort) => {
            options.codex_reasoning_effort = raw_codex_effort_for_level(thinking_level);
        }
        Some(ReasoningControl::AnthropicAdaptive) => {
            options.anthropic_adaptive_effort = anthropic_effort_for_level(thinking_level);
        }
        Some(ReasoningControl::OutputOnly) => {
            options.thinking = None;
        }
        Some(ReasoningControl::AnthropicBudget | ReasoningControl::Boolean) | None => {}
    }
}

fn raw_codex_effort_for_level(thinking_level: ThinkingLevel) -> Option<CodexReasoningEffort> {
    Some(match thinking_level {
        ThinkingLevel::Off => return None,
        ThinkingLevel::Minimal => CodexReasoningEffort::Minimal,
        ThinkingLevel::Low => CodexReasoningEffort::Low,
        ThinkingLevel::Medium => CodexReasoningEffort::Medium,
        ThinkingLevel::High => CodexReasoningEffort::High,
        ThinkingLevel::XHigh => CodexReasoningEffort::XHigh,
        ThinkingLevel::Max | ThinkingLevel::Ultra => CodexReasoningEffort::Max,
    })
}

fn anthropic_effort_for_level(thinking_level: ThinkingLevel) -> Option<AnthropicAdaptiveEffort> {
    Some(match thinking_level {
        ThinkingLevel::Off => return None,
        ThinkingLevel::Minimal | ThinkingLevel::Low => AnthropicAdaptiveEffort::Low,
        ThinkingLevel::Medium => AnthropicAdaptiveEffort::Medium,
        ThinkingLevel::High => AnthropicAdaptiveEffort::High,
        ThinkingLevel::XHigh => AnthropicAdaptiveEffort::XHigh,
        ThinkingLevel::Max | ThinkingLevel::Ultra => AnthropicAdaptiveEffort::Max,
    })
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
        let prompt = chat_system_prompt();

        assert!(prompt.contains("web_search and web_fetch"));
        assert!(prompt.contains("directly in the conversation"));
        assert!(prompt.contains("starting a Code conversation"));
    }

    #[test]
    fn code_tool_suppression_guard_blocks_only_narrow_greetings() {
        for greeting in [
            "Sup boss",
            "hello",
            "Hey Krusty!",
            "good morning",
            "thanks sir",
        ] {
            assert!(should_suppress_code_tools(greeting, false), "{greeting}");
        }

        for task in [
            "hello, fix the failing tests",
            "what's up with src/main.rs?",
            "good morning, inspect the repository",
            "sup boss please run cargo check",
        ] {
            assert!(!should_suppress_code_tools(task, false), "{task}");
        }

        assert!(!should_suppress_code_tools("hello", true));
    }

    #[test]
    fn code_tool_suppression_guard_honors_leading_no_tool_directives() {
        for directive in [
            "Without calling any tool, reply exactly KRUSTY_NO_TOOL_OK.",
            "  WITHOUT USING TOOLS:\nrespond with ready",
            "Do not call any tools. Just answer yes.",
            "Don't use tools; please reply with OK",
            "Don’t call tools; answer only yes",
            "No tool calls \u{2014} output only pong",
        ] {
            assert!(should_suppress_code_tools(directive, false), "{directive}");
        }
    }

    #[test]
    fn code_tool_suppression_guard_does_not_match_quoted_or_task_text() {
        for task in [
            "Add a test for the phrase \"Without calling any tool, reply exactly OK\"",
            "\"Without calling any tool, reply exactly OK\" is a fixture; locate it",
            "Explain why `without calling any tool, reply exactly` is effective",
            "Without calling any toolchain, reply with the build status",
            "Without calling any tool, inspect src/main.rs and fix the bug",
            "No tools are registered; investigate the registry",
        ] {
            assert!(!should_suppress_code_tools(task, false), "{task}");
        }

        assert!(!should_suppress_code_tools(
            "Without calling any tool, reply exactly ATTACHMENT_OK",
            true
        ));
    }

    #[test]
    fn chat_policy_only_allows_conversational_web_tools() {
        let filtered = filter_tools_inner(
            vec![
                tool("web_search"),
                tool("web_fetch"),
                tool("agent"),
                tool("bash"),
            ],
            SessionType::Chat,
        );

        let names = filtered
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["web_search", "web_fetch"]);
    }

    #[test]
    fn code_policy_excludes_mako_only_tools() {
        let filtered = filter_tools_inner(
            vec![tool("bash"), tool("sleep"), tool("report")],
            SessionType::Code,
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
        );

        let names = filtered
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["bash", "sleep", "report"]);
    }

    #[test]
    fn extended_levels_map_to_provider_efforts() {
        assert_eq!(
            raw_codex_effort_for_level(ThinkingLevel::Minimal),
            Some(CodexReasoningEffort::Minimal)
        );
        assert_eq!(
            raw_codex_effort_for_level(ThinkingLevel::Max),
            Some(CodexReasoningEffort::Max)
        );
        assert_eq!(
            raw_codex_effort_for_level(ThinkingLevel::Ultra),
            Some(CodexReasoningEffort::Max)
        );
        assert_eq!(
            anthropic_effort_for_level(ThinkingLevel::XHigh),
            Some(AnthropicAdaptiveEffort::XHigh)
        );
        assert_eq!(
            anthropic_effort_for_level(ThinkingLevel::Ultra),
            Some(AnthropicAdaptiveEffort::Max)
        );
    }

    #[test]
    fn thinking_config_uses_catalog_reasoning_control() {
        let mut options = CallOptions {
            reasoning_control: Some(ReasoningControl::AnthropicAdaptive),
            ..Default::default()
        };
        apply_thinking_config(ThinkingLevel::XHigh, &mut options);
        assert!(options.thinking.is_some());
        assert_eq!(
            options.anthropic_adaptive_effort,
            Some(AnthropicAdaptiveEffort::XHigh)
        );

        let mut output_only = CallOptions {
            reasoning_control: Some(ReasoningControl::OutputOnly),
            ..Default::default()
        };
        apply_thinking_config(ThinkingLevel::High, &mut output_only);
        assert!(output_only.thinking.is_none());
    }
}
