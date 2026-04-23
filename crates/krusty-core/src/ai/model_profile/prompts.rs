use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{Content, ModelMessage, Role};

use super::profile::ModelProfile;
pub struct SystemPromptSections {
    pub profile: ModelProfile,
    pub base_prompt: String,
    pub project_context: String,
    pub session_context: String,
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
