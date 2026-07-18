use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{Content, ModelMessage, Role};

use super::profile::ModelProfile;
pub struct SystemPromptSections {
    pub profile: ModelProfile,
    pub base_prompt: String,
    /// Mako coordinator/persona/user context frozen for the current run.
    /// This is stable within a run and belongs in the cached prefix.
    pub identity_context: String,
    pub project_context: String,
    pub session_context: String,
}

impl SystemPromptSections {
    pub fn combined(&self) -> String {
        let mut sections = Vec::new();

        if !self.base_prompt.is_empty() {
            sections.push(self.base_prompt.as_str());
        }
        if !self.identity_context.is_empty() {
            sections.push(self.identity_context.as_str());
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
    _tool_prompts: &[(String, String)],
) -> SystemPromptSections {
    let profile = ModelProfile::resolve(provider, api_format, model_id);
    let (identity_context, project_context, session_context) =
        partition_system_messages_by_stability(messages);

    let base = profile.layered_system_prompt(provider, api_format, model_id, custom_system_prompt);

    SystemPromptSections {
        profile,
        base_prompt: base,
        identity_context,
        project_context,
        session_context,
    }
}

pub fn partition_system_messages(messages: &[ModelMessage]) -> (String, String) {
    let (identity, project, session) = partition_system_messages_by_stability(messages);
    let session = [identity.trim(), session.trim()]
        .into_iter()
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (project, session)
}

fn partition_system_messages_by_stability(messages: &[ModelMessage]) -> (String, String, String) {
    let mut identity_context = String::new();
    let mut project_context = String::new();
    let mut session_context = String::new();

    for message in messages.iter().filter(|m| m.role == Role::System) {
        if let Some(text) = first_text_block(&message.content) {
            if is_stable_identity_context(text) {
                append_context(&mut identity_context, text);
            } else if is_stable_project_context(text) {
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

    (identity_context, project_context, session_context)
}

fn append_context(target: &mut String, text: &str) {
    if !target.is_empty() {
        target.push_str("\n\n");
    }
    target.push_str(text);
}

fn is_stable_identity_context(text: &str) -> bool {
    [
        "[MAKO COORDINATOR]",
        "[MAKO SOUL",
        "[MAKO IDENTITY",
        "[MAKO USER",
        "[MAKO CREW IDENTITY",
        "[MAKO CREW SOUL",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn is_stable_project_context(text: &str) -> bool {
    [
        "[PROJECT INSTRUCTIONS",
        "[PROJECT SETTINGS]",
        "[MAKO PROJECT OVERLAY",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn first_text_block(content: &[Content]) -> Option<&str> {
    content.iter().find_map(|block| match block {
        Content::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::build_system_prompt_sections;
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::ProviderId;

    #[test]
    fn tool_schema_is_the_only_tool_contract_in_the_system_prompt() {
        let sections = build_system_prompt_sections(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.5-codex",
            &[],
            None,
            &[(
                "bash".to_string(),
                "LEGACY EXTENDED TOOL MANUAL MUST NOT BE INJECTED".to_string(),
            )],
        );

        assert!(!sections.base_prompt.contains("LEGACY EXTENDED TOOL MANUAL"));
        assert!(sections.base_prompt.len() <= 5_000);
    }

    #[test]
    fn mako_identity_is_frozen_into_its_own_stable_section() {
        use crate::ai::types::{Content, ModelMessage, Role};

        let messages = vec![
            ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: "[MAKO SOUL - MAKO_SOUL.md]\nWarm and exact.".to_string(),
                }],
            },
            ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: "[MAKO HEARTBEAT - MAKO_HEARTBEAT.md]\nCheck work.".to_string(),
                }],
            },
        ];
        let sections = build_system_prompt_sections(
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.5-codex",
            &messages,
            None,
            &[],
        );

        assert!(sections.identity_context.contains("Warm and exact"));
        assert!(!sections.identity_context.contains("Check work"));
        assert!(sections.session_context.contains("Check work"));
    }

    #[test]
    fn mako_prompt_tiers_keep_persona_and_project_stable_but_operations_volatile() {
        use crate::ai::types::{Content, ModelMessage, Role};

        let system = |text: &str| ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: text.to_string(),
            }],
        };
        let first = vec![
            system("[MAKO COORDINATOR]\nCoordinate deliberately."),
            system("[MAKO SOUL - profile:local]\nWarm and candid."),
            system("[MAKO IDENTITY - profile:local]\nName: Mako"),
            system("[MAKO USER - profile:local]\nUse concise updates."),
            system("[MAKO CREW SOUL - reviewer]\nEvidence first."),
            system("[PROJECT INSTRUCTIONS - AGENTS.md]\nValidate changes."),
            system("[MAKO PROJECT OVERLAY - MAKO.md]\nProject cadence."),
            system("[PROJECT SETTINGS]\nAutonomous mode."),
            system("[MAKO HEARTBEAT - snapshot-revision:4]\nCheck queue A."),
            system("[MAKO CHANNELS - snapshot-revision:4]\nMain thread."),
            system("[RECALLED CONVERSATION EPISODES]\nFallible evidence."),
        ];
        let mut second = first.clone();
        second[8] = system("[MAKO HEARTBEAT - snapshot-revision:5]\nCheck queue B.");
        second[9] = system("[MAKO CHANNELS - snapshot-revision:5]\nMobile push.");

        let first = build_system_prompt_sections(
            ProviderId::Anthropic,
            ApiFormat::Anthropic,
            "claude-sonnet-4-5",
            &first,
            None,
            &[],
        );
        let second = build_system_prompt_sections(
            ProviderId::Anthropic,
            ApiFormat::Anthropic,
            "claude-sonnet-4-5",
            &second,
            None,
            &[],
        );

        assert_eq!(first.identity_context, second.identity_context);
        assert_eq!(first.project_context, second.project_context);
        assert_ne!(first.session_context, second.session_context);
        for marker in [
            "MAKO COORDINATOR",
            "MAKO SOUL",
            "MAKO IDENTITY",
            "MAKO USER",
            "MAKO CREW SOUL",
        ] {
            assert!(first.identity_context.contains(marker));
        }
        assert!(first.project_context.contains("PROJECT INSTRUCTIONS"));
        assert!(first.project_context.contains("MAKO PROJECT OVERLAY"));
        assert!(first.project_context.contains("PROJECT SETTINGS"));
        assert!(first.session_context.contains("MAKO HEARTBEAT"));
        assert!(first.session_context.contains("MAKO CHANNELS"));
        assert!(first
            .session_context
            .contains("RECALLED CONVERSATION EPISODES"));
        assert!(!first.identity_context.contains("MAKO HEARTBEAT"));
        assert!(!first.project_context.contains("MAKO CHANNELS"));
    }
}
