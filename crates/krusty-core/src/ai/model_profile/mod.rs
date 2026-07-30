//! Model-specific prompt profiles and system prompt assembly.
//!
//! Keeps model-family steering in one place so streaming and non-streaming calls
//! build the same layered instructions.

mod profile;
mod prompts;

pub use crate::ai::transport_policy::StreamDrainPolicy;
pub use profile::{ModelProfile, PromptFamily};
pub use prompts::{
    build_system_prompt_sections, partition_system_messages, PromptSection, PromptSectionKind,
    PromptStability, SystemPromptSections,
};

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
    }

    #[test]
    fn resolves_grok_family_before_openai_responses_transport_family() {
        let profile =
            ModelProfile::resolve(ProviderId::Grok, ApiFormat::OpenAIResponses, "grok-4.5");

        assert_eq!(profile.prompt_family, PromptFamily::Grok);
    }

    #[test]
    fn grok_overlay_forbids_placeholder_tools_after_direct_steering() {
        let sections = build_system_prompt_sections(
            ProviderId::Grok,
            ApiFormat::OpenAIResponses,
            "grok-4.5",
            &[],
            None,
            &[],
        );

        assert!(sections.base_prompt.contains("latest user message"));
        assert!(sections.base_prompt.contains("Never issue a no-op"));
        assert!(sections.base_prompt.contains("Skip redundant mid-turn status"));
        assert!(!sections.base_prompt.contains("8–12 word"));
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
    fn layered_prompt_keeps_one_compact_model_overlay() {
        let sections = build_system_prompt_sections(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.3-codex",
            &[],
            None,
            &[],
        );

        assert!(sections.base_prompt.contains("## Model behavior"));
        assert!(sections.base_prompt.contains("Keep prose compact"));
        assert!(!sections.base_prompt.contains("## Provider Guidance"));
        assert!(!sections.base_prompt.contains("## Capability Guidance"));
        assert!(sections.base_prompt.len() <= 5_000);
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
