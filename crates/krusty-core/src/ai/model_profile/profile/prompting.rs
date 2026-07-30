use crate::ai::client::core::KRUSTY_SYSTEM_PROMPT;
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;

use super::{ModelProfile, PromptFamily};

impl ModelProfile {
    pub fn layered_system_prompt(
        self,
        _provider: ProviderId,
        _api_format: ApiFormat,
        _model_id: &str,
        custom_system_prompt: Option<&str>,
    ) -> String {
        if let Some(custom) = custom_system_prompt
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return custom.to_string();
        }

        let mut prompt = KRUSTY_SYSTEM_PROMPT.to_string();

        let overlay = self.prompt_overlay().trim();
        if !overlay.is_empty() {
            prompt.push_str("\n\n## Model behavior\n\n");
            prompt.push_str(overlay);
        }

        prompt
    }

    fn prompt_overlay(self) -> &'static str {
        match self.prompt_family {
            PromptFamily::AnthropicClaude => {
                "Keep updates short, use tools decisively, and preserve explicit task state across long runs."
            }
            PromptFamily::OpenAiCodex => {
                "Keep prose compact, use parallel tools when independent, and resume from preserved state after compaction."
            }
            PromptFamily::OpenAiReasoning => {
                "Turn internal reasoning into the next concrete tool call or final answer; do not dump the plan in user-visible text every round."
            }
            PromptFamily::Grok => {
                "After each tool result, silently decide the next action—do not re-narrate the plan. If the latest user message asks for a direct reply or says not to call another tool, reply immediately without a tool. Never issue a no-op or placeholder tool call merely to bridge from a tool result to text. Skip redundant mid-turn status; speak only when something new matters, then keep going with tools."
            }
            PromptFamily::GoogleGemini => {
                "Ground decisions in tool evidence, keep calls narrow, and preserve the active task across long contexts."
            }
            PromptFamily::GenericCoding => "",
        }
    }
}
