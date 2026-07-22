use serde::{Deserialize, Serialize};
mod prompting;
mod resolution;

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
}
