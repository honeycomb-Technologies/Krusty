use crate::ai::client::core::KRUSTY_SYSTEM_PROMPT;
use crate::ai::models::{resolve_context_window, ApiFormat};
use crate::ai::providers::ProviderId;

use super::{ModelProfile, PromptFamily};

impl ModelProfile {
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

fn provider_prompt_overlay(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Anthropic => {
            r#"- Keep tool and plan state explicit so long-running sessions survive compaction cleanly.
- Prefer steady, incremental repository work over broad speculative rewrites."#
        }
        ProviderId::OpenAI | ProviderId::Grok => {
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
