//! Model-specific prompt profiles and system prompt assembly.
//!
//! Keeps model-family steering in one place so streaming and non-streaming calls
//! build the same layered instructions.

mod profile;
mod prompts;

pub use profile::{CompactionBudgets, ModelProfile, PromptFamily, StreamDrainPolicy};
pub use prompts::{build_system_prompt_sections, partition_system_messages, SystemPromptSections};

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
