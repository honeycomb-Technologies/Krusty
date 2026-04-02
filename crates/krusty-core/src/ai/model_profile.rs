//! Model-specific prompt profiles and system prompt assembly.
//!
//! Keeps model-family steering in one place so streaming and non-streaming calls
//! build the same layered instructions.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::ai::client::core::KRUSTY_SYSTEM_PROMPT;
use crate::ai::models::{resolve_context_window, ApiFormat};
use crate::ai::providers::ProviderId;
use crate::ai::types::{Content, ModelMessage, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptFamily {
    AnthropicClaude,
    OpenAiCodex,
    OpenAiReasoning,
    GoogleGemini,
    #[default]
    GenericCoding,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelProfile {
    pub prompt_family: PromptFamily,
    pub usable_context_ratio: f32,
    pub auto_compact_threshold_ratio: f32,
    pub compaction_target_ratio: f32,
    pub hard_failure_ratio: f32,
    pub stream_idle_timeout_secs: u64,
    pub supports_reasoning_summary: bool,
    pub prefer_parallel_tool_calls: bool,
}

impl ModelProfile {
    pub fn resolve(provider: ProviderId, api_format: ApiFormat, model_id: &str) -> Self {
        let normalized = normalize_model_id(model_id);

        if normalized.contains("claude") || normalized.starts_with("anthropic/") {
            return Self {
                prompt_family: PromptFamily::AnthropicClaude,
                usable_context_ratio: 0.74,
                auto_compact_threshold_ratio: 0.82,
                compaction_target_ratio: 0.64,
                hard_failure_ratio: 0.90,
                stream_idle_timeout_secs: 900,
                supports_reasoning_summary: false,
                prefer_parallel_tool_calls: true,
            };
        }

        if matches!(api_format, ApiFormat::Google)
            || normalized.contains("gemini")
            || normalized.starts_with("google/")
        {
            return Self {
                prompt_family: PromptFamily::GoogleGemini,
                usable_context_ratio: 0.78,
                auto_compact_threshold_ratio: 0.85,
                compaction_target_ratio: 0.68,
                hard_failure_ratio: 0.93,
                stream_idle_timeout_secs: 900,
                supports_reasoning_summary: false,
                prefer_parallel_tool_calls: true,
            };
        }

        if normalized.contains("codex") {
            return Self {
                prompt_family: PromptFamily::OpenAiCodex,
                usable_context_ratio: 0.7,
                auto_compact_threshold_ratio: 0.78,
                compaction_target_ratio: 0.60,
                hard_failure_ratio: 0.88,
                stream_idle_timeout_secs: 1_200,
                supports_reasoning_summary: true,
                prefer_parallel_tool_calls: true,
            };
        }

        if matches!(api_format, ApiFormat::OpenAIResponses)
            || uses_openai_responses_family(&normalized)
        {
            return Self {
                prompt_family: PromptFamily::OpenAiReasoning,
                usable_context_ratio: 0.72,
                auto_compact_threshold_ratio: 0.8,
                compaction_target_ratio: 0.62,
                hard_failure_ratio: 0.90,
                stream_idle_timeout_secs: 1_200,
                supports_reasoning_summary: true,
                prefer_parallel_tool_calls: true,
            };
        }

        if provider == ProviderId::OpenAI || normalized.starts_with("openai/") {
            return Self {
                prompt_family: PromptFamily::OpenAiReasoning,
                usable_context_ratio: 0.75,
                auto_compact_threshold_ratio: 0.82,
                compaction_target_ratio: 0.64,
                hard_failure_ratio: 0.90,
                stream_idle_timeout_secs: 900,
                supports_reasoning_summary: false,
                prefer_parallel_tool_calls: true,
            };
        }

        Self {
            prompt_family: PromptFamily::GenericCoding,
            usable_context_ratio: 0.75,
            auto_compact_threshold_ratio: 0.84,
            compaction_target_ratio: 0.64,
            hard_failure_ratio: 0.92,
            stream_idle_timeout_secs: 900,
            supports_reasoning_summary: false,
            prefer_parallel_tool_calls: true,
        }
    }

    pub fn compaction_budgets(self, context_window: usize) -> CompactionBudgets {
        let context_window = context_window.max(1);
        let trigger_tokens = ratio_to_tokens(context_window, self.auto_compact_threshold_ratio);
        let target_tokens = ratio_to_tokens(context_window, self.compaction_target_ratio)
            .min(trigger_tokens.saturating_sub(1).max(1));
        let hard_failure_tokens = ratio_to_tokens(context_window, self.hard_failure_ratio)
            .max(trigger_tokens.saturating_add(1));

        CompactionBudgets {
            trigger_tokens,
            target_tokens,
            hard_failure_tokens,
        }
    }

    pub fn stream_drain_policy(self) -> StreamDrainPolicy {
        match self.prompt_family {
            PromptFamily::OpenAiCodex => StreamDrainPolicy {
                smooth_batch_limit: 18,
                moderate_batch_limit: 48,
                catch_up_batch_limit: 128,
                moderate_backlog_threshold: 28,
                catch_up_backlog_threshold: 96,
                moderate_backlog_age: Duration::from_millis(35),
                catch_up_backlog_age: Duration::from_millis(100),
                hard_queue_limit: 512,
            },
            PromptFamily::OpenAiReasoning => StreamDrainPolicy {
                smooth_batch_limit: 14,
                moderate_batch_limit: 40,
                catch_up_batch_limit: 112,
                moderate_backlog_threshold: 24,
                catch_up_backlog_threshold: 88,
                moderate_backlog_age: Duration::from_millis(40),
                catch_up_backlog_age: Duration::from_millis(110),
                hard_queue_limit: 448,
            },
            PromptFamily::AnthropicClaude | PromptFamily::GoogleGemini => StreamDrainPolicy {
                smooth_batch_limit: 12,
                moderate_batch_limit: 28,
                catch_up_batch_limit: 80,
                moderate_backlog_threshold: 20,
                catch_up_backlog_threshold: 72,
                moderate_backlog_age: Duration::from_millis(40),
                catch_up_backlog_age: Duration::from_millis(120),
                hard_queue_limit: 384,
            },
            PromptFamily::GenericCoding => StreamDrainPolicy::default(),
        }
    }

    pub fn layered_system_prompt(
        self,
        provider: ProviderId,
        api_format: ApiFormat,
        model_id: &str,
        custom_system_prompt: Option<&str>,
    ) -> String {
        if let Some(custom) = custom_system_prompt
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return custom.to_string();
        }

        let mut prompt = KRUSTY_SYSTEM_PROMPT.to_string();

        let provider_overlay = provider_prompt_overlay(provider);
        if !provider_overlay.is_empty() {
            prompt.push_str("\n\n## Provider Guidance\n\n");
            prompt.push_str(provider_overlay.trim());
        }

        let overlay = self.prompt_overlay().trim();
        if !overlay.is_empty() {
            prompt.push_str("\n\n## Model Guidance\n\n");
            prompt.push_str(overlay);
        }

        let capability_overlay = capability_prompt_overlay(self, provider, api_format, model_id);
        if !capability_overlay.is_empty() {
            prompt.push_str("\n\n## Capability Guidance\n\n");
            prompt.push_str(&capability_overlay);
        }

        prompt
    }

    fn prompt_overlay(self) -> &'static str {
        match self.prompt_family {
            PromptFamily::AnthropicClaude => {
                r#"- Give short execution updates, then act.
- Use tools decisively when repository evidence is needed.
- Keep plan state explicit across long tool loops so work continues cleanly after compaction."#
            }
            PromptFamily::OpenAiCodex => {
                r#"- Continue through tool-use loops until the requested engineering outcome is actually reached.
- Keep assistant prose compact during long runs; spend tokens on actions and precise tool inputs.
- When context is compacted, resume implementation from the preserved objective instead of restarting analysis."#
            }
            PromptFamily::OpenAiReasoning => {
                r#"- Prefer concrete repository evidence over abstract discussion.
- Use reasoning to choose the next correct engineering step, then execute it without stalling.
- After tool results arrive, reassess and continue rather than handing control back early."#
            }
            PromptFamily::GoogleGemini => {
                r#"- Ground decisions in explicit file and tool evidence.
- Keep tool requests tightly scoped and avoid speculative broad refactors.
- Preserve the active task through long contexts by relying on summarized state instead of re-planning from scratch."#
            }
            PromptFamily::GenericCoding => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SystemPromptSections {
    pub profile: ModelProfile,
    pub base_prompt: String,
    pub project_context: String,
    pub session_context: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionBudgets {
    pub trigger_tokens: usize,
    pub target_tokens: usize,
    pub hard_failure_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamDrainPolicy {
    pub smooth_batch_limit: usize,
    pub moderate_batch_limit: usize,
    pub catch_up_batch_limit: usize,
    pub moderate_backlog_threshold: usize,
    pub catch_up_backlog_threshold: usize,
    pub moderate_backlog_age: Duration,
    pub catch_up_backlog_age: Duration,
    pub hard_queue_limit: usize,
}

impl Default for StreamDrainPolicy {
    fn default() -> Self {
        Self {
            smooth_batch_limit: 12,
            moderate_batch_limit: 32,
            catch_up_batch_limit: 96,
            moderate_backlog_threshold: 24,
            catch_up_backlog_threshold: 80,
            moderate_backlog_age: Duration::from_millis(40),
            catch_up_backlog_age: Duration::from_millis(120),
            hard_queue_limit: 384,
        }
    }
}

impl SystemPromptSections {
    pub fn combined(&self) -> String {
        let mut sections = Vec::new();

        if !self.base_prompt.is_empty() {
            sections.push(self.base_prompt.as_str());
        }
        if !self.project_context.is_empty() {
            sections.push(self.project_context.as_str());
        }
        if !self.session_context.is_empty() {
            sections.push(self.session_context.as_str());
        }

        sections.join("\n\n---\n\n")
    }
}

pub fn build_system_prompt_sections(
    provider: ProviderId,
    api_format: ApiFormat,
    model_id: &str,
    messages: &[ModelMessage],
    custom_system_prompt: Option<&str>,
    tool_prompts: &[(String, String)],
) -> SystemPromptSections {
    let profile = ModelProfile::resolve(provider, api_format, model_id);
    let (project_context, session_context) = partition_system_messages(messages);

    let mut base =
        profile.layered_system_prompt(provider, api_format, model_id, custom_system_prompt);
    let tool_guidance = build_tool_guidance_section(tool_prompts);
    if !tool_guidance.is_empty() {
        base.push_str("\n\n");
        base.push_str(&tool_guidance);
    }

    SystemPromptSections {
        profile,
        base_prompt: base,
        project_context,
        session_context,
    }
}

fn build_tool_guidance_section(tool_prompts: &[(String, String)]) -> String {
    let prompts: Vec<_> = tool_prompts
        .iter()
        .filter(|(_, prompt)| !prompt.is_empty())
        .collect();

    if prompts.is_empty() {
        return String::new();
    }

    let mut section = String::from("# Tool Guidance\n\n");
    for (name, prompt) in &prompts {
        section.push_str(&format!("## {}\n{}\n\n", name, prompt));
    }
    section
}

pub fn partition_system_messages(messages: &[ModelMessage]) -> (String, String) {
    let mut project_context = String::new();
    let mut session_context = String::new();

    for message in messages.iter().filter(|m| m.role == Role::System) {
        if let Some(text) = first_text_block(&message.content) {
            if text.starts_with("[PROJECT INSTRUCTIONS") {
                if !project_context.is_empty() {
                    project_context.push_str("\n\n");
                }
                project_context.push_str(text);
            } else {
                if !session_context.is_empty() {
                    session_context.push_str("\n\n");
                }
                session_context.push_str(text);
            }
        }
    }

    (project_context, session_context)
}

fn first_text_block(content: &[Content]) -> Option<&str> {
    content.iter().find_map(|block| match block {
        Content::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

fn normalize_model_id(model_id: &str) -> String {
    model_id.trim().to_ascii_lowercase()
}

fn uses_openai_responses_family(model_id: &str) -> bool {
    let normalized = model_id.strip_prefix("openai/").unwrap_or(model_id);

    normalized.contains("codex")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
        || gpt_major_version(normalized).is_some_and(|major| major >= 5)
}

fn provider_prompt_overlay(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Anthropic => {
            r#"- Keep tool and plan state explicit so long-running sessions survive compaction cleanly.
- Prefer steady, incremental repository work over broad speculative rewrites."#
        }
        ProviderId::OpenAI => {
            r#"- Preserve exact task continuity across tool turns and continue execution instead of re-explaining the plan.
- Keep tool arguments precise; avoid wasting reasoning budget on restating known context."#
        }
        ProviderId::OpenRouter => {
            r#"- Normalize around the requested engineering task even if upstream model behavior varies.
- Favor portable tool-use patterns and avoid provider-specific assumptions in the response."#
        }
        ProviderId::MiniMax | ProviderId::ZAi => {
            r#"- Be explicit about intent before mutating files or processes.
- Keep responses and tool inputs deterministic so provider quirks do not derail the run."#
        }
    }
}

fn capability_prompt_overlay(
    profile: ModelProfile,
    provider: ProviderId,
    api_format: ApiFormat,
    model_id: &str,
) -> String {
    let mut lines = Vec::new();
    let context_window = resolve_context_window(provider, model_id, api_format);

    if context_window >= 400_000 {
        lines.push(
            "- Use the larger context window to preserve continuity, but still keep summaries and tool evidence concise.",
        );
    }

    if matches!(api_format, ApiFormat::OpenAIResponses) {
        lines.push(
            "- Keep tool and continuation state explicit because Responses-style turns may span richer item streams than chat-format turns.",
        );
    }

    if profile.supports_reasoning_summary {
        lines.push(
            "- Condense reasoning into actionable conclusions so summarized state remains useful after compaction.",
        );
    }

    if profile.prefer_parallel_tool_calls {
        lines.push(
            "- When several independent reads are needed, batch them in one turn instead of serializing unnecessary tool calls.",
        );
    }

    lines.join("\n")
}

fn ratio_to_tokens(context_window: usize, ratio: f32) -> usize {
    ((context_window as f64 * ratio as f64).round() as usize)
        .max(1)
        .min(context_window)
}

fn gpt_major_version(model_id: &str) -> Option<u32> {
    let suffix = model_id.strip_prefix("gpt-")?;
    let digits = suffix
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())?;
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        build_system_prompt_sections, partition_system_messages, ModelProfile, PromptFamily,
    };
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::ProviderId;
    use crate::ai::types::{Content, ModelMessage, Role};

    fn text_message(role: Role, text: &str) -> ModelMessage {
        ModelMessage {
            role,
            content: vec![Content::Text {
                text: text.to_string(),
            }],
        }
    }

    #[test]
    fn resolves_codex_family_for_manual_newer_gpt_model_ids() {
        let profile = ModelProfile::resolve(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-6.4-codex",
        );

        assert_eq!(profile.prompt_family, PromptFamily::OpenAiCodex);
        assert!(profile.supports_reasoning_summary);
    }

    #[test]
    fn partitions_project_and_session_system_messages() {
        let messages = vec![
            text_message(Role::System, "[PROJECT INSTRUCTIONS]\nUse Rust."),
            text_message(Role::System, "[ACTIVE PLAN]\nFinish the refactor."),
        ];

        let (project, session) = partition_system_messages(&messages);
        assert!(project.contains("Use Rust"));
        assert!(session.contains("ACTIVE PLAN"));
    }

    #[test]
    fn layered_prompt_includes_overlay_for_default_agent_prompt() {
        let sections = build_system_prompt_sections(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.3-codex",
            &[],
            None,
            &[],
        );

        assert!(sections.base_prompt.contains("## Model Guidance"));
        assert!(sections.base_prompt.contains("## Provider Guidance"));
        assert!(sections.base_prompt.contains("## Capability Guidance"));
        assert!(sections.base_prompt.contains("tool-use loops"));
    }

    #[test]
    fn codex_profiles_use_more_aggressive_stream_drain_policy() {
        let codex = ModelProfile::resolve(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.3-codex",
        )
        .stream_drain_policy();
        let generic = ModelProfile::resolve(
            ProviderId::Anthropic,
            ApiFormat::Anthropic,
            "claude-sonnet-4.5",
        )
        .stream_drain_policy();

        assert!(codex.catch_up_batch_limit > generic.catch_up_batch_limit);
        assert!(codex.hard_queue_limit > generic.hard_queue_limit);
    }

    #[test]
    fn custom_system_prompt_bypasses_model_overlay() {
        let sections = build_system_prompt_sections(
            ProviderId::Anthropic,
            ApiFormat::Anthropic,
            "claude-sonnet-4.5",
            &[],
            Some("Summarize only."),
            &[],
        );

        assert_eq!(sections.base_prompt, "Summarize only.");
        assert!(!sections.base_prompt.contains("## Model Guidance"));
    }
}
