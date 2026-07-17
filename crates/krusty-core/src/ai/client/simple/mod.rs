//! Simple (non-streaming) API calls
//!
//! Used for quick tasks like title generation where streaming is overkill.
//! Also provides `call_with_conversation` for cache-safe fork operations
//! (summarization/compaction) that reuse the parent conversation's cached prefix.

mod anthropic;
mod codex;
mod google;
mod openai;
mod shared;

use anyhow::Result;

use super::config::CallOptions;
use super::core::AiClient;
use crate::ai::types::{ModelMessage, Usage};

/// Text and normalized provider usage from one non-streaming request.
///
/// Usage remains optional because some OpenAI/Anthropic-compatible gateways do
/// not return it. The legacy text-only methods are retained as wrappers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimpleCallResult {
    pub text: String,
    pub usage: Option<Usage>,
}

impl AiClient {
    /// Make a simple non-streaming API call
    ///
    /// Used for quick tasks like title generation where streaming is overkill.
    /// Returns the text content directly. Routes to appropriate format handler.
    pub async fn call_simple(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        self.call_simple_with_usage(model, system_prompt, user_message, max_tokens)
            .await
            .map(|result| result.text)
    }

    /// Make a non-streaming API call while preserving provider usage.
    pub async fn call_simple_with_usage(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<SimpleCallResult> {
        let options = self.canonical_call_options(
            model,
            &CallOptions {
                max_tokens: Some(max_tokens),
                system_prompt: Some(system_prompt.to_string()),
                ..Default::default()
            },
        );
        let prompt_sections =
            self.system_prompt_sections(model, &[], options.system_prompt.as_deref(), None);
        let system_prompt = prompt_sections.combined();
        let max_tokens = options.max_tokens.unwrap_or(max_tokens);

        if self.config().uses_chatgpt_codex_format() {
            return self
                .call_simple_chatgpt_codex(
                    model,
                    &system_prompt,
                    user_message,
                    max_tokens,
                    &options,
                )
                .await;
        }

        if self.config().uses_openai_format() {
            return self
                .call_simple_openai(model, &system_prompt, user_message, max_tokens)
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_simple_google(model, &system_prompt, user_message, max_tokens)
                .await;
        }

        self.call_simple_anthropic(model, &system_prompt, user_message, max_tokens, &options)
            .await
    }

    /// Non-streaming API call that reuses the parent conversation's cached prefix.
    ///
    /// Instead of flattening conversation into a single user message (which shares
    /// zero cache prefix with the parent conversation), this sends the actual
    /// conversation messages as API messages and appends a new user instruction.
    ///
    /// The cached prefix from the parent conversation (system prompt + conversation
    /// history) is fully reused, so the only uncached tokens are the appended message.
    /// This follows Thariq's lesson: "When we run compaction, we use the exact same
    /// system prompt, user context, system context, and tool definitions."
    pub async fn call_with_conversation(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        self.call_with_conversation_with_usage(
            model,
            base_system_prompt,
            conversation,
            appended_user_message,
            max_tokens,
        )
        .await
        .map(|result| result.text)
    }

    /// Cache-safe conversation call that also preserves provider usage.
    pub async fn call_with_conversation_with_usage(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<SimpleCallResult> {
        let options = self.canonical_call_options(
            model,
            &CallOptions {
                max_tokens: Some(max_tokens),
                system_prompt: Some(base_system_prompt.to_string()),
                ..Default::default()
            },
        );
        let max_tokens = options.max_tokens.unwrap_or(max_tokens);

        if self.config().uses_chatgpt_codex_format() {
            return self
                .call_conversation_chatgpt_codex(
                    model,
                    options
                        .system_prompt
                        .as_deref()
                        .unwrap_or(base_system_prompt),
                    conversation,
                    appended_user_message,
                    max_tokens,
                    &options,
                )
                .await;
        }

        if self.config().uses_openai_format() {
            return self
                .call_conversation_openai(
                    model,
                    options
                        .system_prompt
                        .as_deref()
                        .unwrap_or(base_system_prompt),
                    conversation,
                    appended_user_message,
                    max_tokens,
                )
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_conversation_google(
                    model,
                    options
                        .system_prompt
                        .as_deref()
                        .unwrap_or(base_system_prompt),
                    conversation,
                    appended_user_message,
                    max_tokens,
                )
                .await;
        }

        self.call_conversation_anthropic(
            model,
            options
                .system_prompt
                .as_deref()
                .unwrap_or(base_system_prompt),
            conversation,
            appended_user_message,
            max_tokens,
            &options,
        )
        .await
    }
}
