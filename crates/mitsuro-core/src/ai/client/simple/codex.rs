use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::super::config::CallOptions;
use super::super::core::AiClient;
use super::openai::extract_openai_text;
use super::shared::trim_or_empty;
use super::SimpleCallResult;
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::model_profile::SystemPromptSections;
use crate::ai::models::ApiFormat;
use crate::ai::retry::safe_provider_event_error;
use crate::ai::types::{Content, ModelMessage, Role, Usage};
use crate::ai::usage::parse_openai_responses_usage;

impl AiClient {
    /// Simple call using ChatGPT Codex (Responses API) format
    ///
    /// Codex requires `stream: true`, so we stream and collect the response.
    pub(super) async fn call_simple_chatgpt_codex(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
        options: &CallOptions,
    ) -> Result<SimpleCallResult> {
        let messages = [ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: user_message.to_string(),
            }],
        }];
        let body = self.build_simple_codex_body(system_prompt, &messages, max_tokens, options);
        self.send_simple_codex_body(model, body).await
    }

    fn build_conversation_codex_body(
        &self,
        model: &str,
        system_prompt: &str,
        messages: &[ModelMessage],
        max_tokens: usize,
        options: &CallOptions,
    ) -> Value {
        let prompt_sections = self.system_prompt_sections(
            model,
            messages,
            Some(system_prompt),
            options.tools.as_deref(),
        );
        let stable_instructions = codex_cache_stable_instructions(&prompt_sections);
        let volatile_context = (!prompt_sections.session_context.trim().is_empty())
            .then_some(prompt_sections.session_context.as_str());
        let format_handler = OpenAIFormat::new(ApiFormat::OpenAIResponses);
        self.build_chatgpt_codex_body(
            messages,
            &stable_instructions,
            volatile_context,
            max_tokens,
            options,
            &format_handler,
        )
    }

    pub(super) async fn call_conversation_chatgpt_codex(
        &self,
        model: &str,
        system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
        options: &CallOptions,
    ) -> Result<SimpleCallResult> {
        let mut messages = conversation.to_vec();
        messages.push(ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: appended_user_message.to_string(),
            }],
        });
        let body = self.build_conversation_codex_body(
            model,
            system_prompt,
            &messages,
            max_tokens,
            options,
        );
        self.send_simple_codex_body(model, body).await
    }

    fn build_simple_codex_body(
        &self,
        system_prompt: &str,
        messages: &[ModelMessage],
        max_tokens: usize,
        options: &CallOptions,
    ) -> Value {
        let format_handler = OpenAIFormat::new(ApiFormat::OpenAIResponses);
        self.build_chatgpt_codex_body(
            messages,
            system_prompt,
            None,
            max_tokens,
            options,
            &format_handler,
        )
    }

    async fn send_simple_codex_body(&self, model: &str, body: Value) -> Result<SimpleCallResult> {
        use futures::StreamExt;

        debug!("ChatGPT Codex simple call to model: {}", model);
        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        // Stream and collect text
        let mut collected_text = String::new();
        let mut final_text = None;
        let mut usage = Usage::default();
        let mut usage_available = false;
        let mut pending = Vec::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            pending.extend_from_slice(&chunk);
            while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
                let line = pending.drain(..=newline).collect::<Vec<_>>();
                process_codex_sse_line(
                    &line,
                    &mut collected_text,
                    &mut final_text,
                    &mut usage,
                    &mut usage_available,
                )?;
            }
        }

        if !pending.is_empty() {
            process_codex_sse_line(
                &pending,
                &mut collected_text,
                &mut final_text,
                &mut usage,
                &mut usage_available,
            )?;
        }

        if collected_text.is_empty() {
            collected_text = final_text.unwrap_or_default();
        }

        Ok(SimpleCallResult {
            text: trim_or_empty(Some(&collected_text)),
            usage: usage_available.then_some(usage),
        })
    }
}

fn codex_cache_stable_instructions(prompt_sections: &SystemPromptSections) -> String {
    [
        prompt_sections.base_prompt.as_str(),
        prompt_sections.identity_context.as_str(),
        prompt_sections.project_context.as_str(),
    ]
    .into_iter()
    .filter(|section| !section.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n---\n\n")
}

fn process_codex_sse_line(
    line: &[u8],
    collected_text: &mut String,
    final_text: &mut Option<String>,
    usage: &mut Usage,
    usage_available: &mut bool,
) -> Result<()> {
    let line = String::from_utf8_lossy(line);
    let line = line.trim_end_matches(['\r', '\n']);
    let Some(data) = line
        .strip_prefix("data: ")
        .or_else(|| line.strip_prefix("data:"))
    else {
        return Ok(());
    };
    if data == "[DONE]" || data.trim().is_empty() {
        return Ok(());
    }

    let json: Value = serde_json::from_str(data).map_err(|_| {
        anyhow::Error::msg(safe_provider_event_error(
            "ChatGPT Codex simple response was invalid JSON",
            None,
            Some("invalid_request_error"),
            Some(data),
        ))
    })?;
    let event_type = json.get("type").and_then(Value::as_str).unwrap_or_default();
    if event_type == "error" || event_type.contains(".failed") {
        let message = json
            .get("error")
            .and_then(|error| error.get("message").or(Some(error)))
            .and_then(Value::as_str)
            .or_else(|| json.get("message").and_then(Value::as_str));
        let code = json.pointer("/error/code").and_then(Value::as_str);
        let category = json
            .pointer("/error/type")
            .and_then(Value::as_str)
            .or(Some(event_type));
        anyhow::bail!(safe_provider_event_error(
            "ChatGPT Codex simple call failed",
            code,
            category,
            message,
        ));
    }

    if event_type == "response.output_text.delta" {
        if let Some(delta) = json.get("delta").and_then(Value::as_str) {
            collected_text.push_str(delta);
        }
    }
    if matches!(event_type, "response.completed" | "response.done") {
        if let Some(response) = json.get("response") {
            *final_text = extract_openai_text(response).map(ToString::to_string);
        }
    }
    if let Some(snapshot) = parse_openai_responses_usage(&json) {
        usage.merge_snapshot(&snapshot);
        *usage_available = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{process_codex_sse_line, AiClient};
    use crate::ai::client::{AiClientConfig, CallOptions};
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::ProviderId;
    use crate::ai::types::{Content, ModelMessage, Role, Usage};

    #[test]
    fn conversation_body_preserves_history_and_appended_instruction() {
        let client = AiClient::new(
            AiClientConfig {
                model: "gpt-5.5".to_string(),
                provider_id: ProviderId::OpenAI,
                api_format: ApiFormat::OpenAIResponses,
                base_url: Some("https://chatgpt.com/backend-api/codex/responses".to_string()),
                ..Default::default()
            },
            "test-key".to_string(),
        );
        let mut messages = vec![
            ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: "original request".to_string(),
                }],
            },
            ModelMessage {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "prior response".to_string(),
                }],
            },
        ];
        messages.push(ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "summarize now".to_string(),
            }],
        });

        let body = client.build_simple_codex_body(
            "summary system prompt",
            &messages,
            1_024,
            &CallOptions::default(),
        );
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["text"], "original request");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["text"], "prior response");
        assert_eq!(input[2]["role"], "user");
        assert_eq!(input[2]["content"][0]["text"], "summarize now");
        assert_eq!(body["instructions"], "summary system prompt");
    }

    #[test]
    fn conversation_body_preserves_stable_identity_and_volatile_session_layers() {
        let client = AiClient::new(
            AiClientConfig {
                model: "gpt-5.5".to_string(),
                provider_id: ProviderId::OpenAI,
                api_format: ApiFormat::OpenAIResponses,
                base_url: Some("https://chatgpt.com/backend-api/codex/responses".to_string()),
                ..Default::default()
            },
            "test-key".to_string(),
        );
        let messages = vec![
            ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: "[HIVE SOUL - HIVE_SOUL.md]\nidentity-layer".into(),
                }],
            },
            ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: "[PROJECT INSTRUCTIONS - AGENTS.md]\nproject-layer".into(),
                }],
            },
            ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: "[HIVE HEARTBEAT]\nsession-layer".into(),
                }],
            },
            ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: "continue".into(),
                }],
            },
        ];

        let body = client.build_conversation_codex_body(
            "gpt-5.5",
            "base-layer",
            &messages,
            1_024,
            &CallOptions::default(),
        );
        let instructions = body["instructions"].as_str().expect("instructions");
        let base = instructions.find("base-layer").expect("base layer");
        let identity = instructions.find("identity-layer").expect("identity layer");
        let project = instructions.find("project-layer").expect("project layer");
        assert!(base < identity && identity < project);
        assert!(!instructions.contains("session-layer"));

        let input = body["input"].as_array().expect("input array");
        assert_eq!(input.len(), 2, "system messages must not leak into input");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["role"], "developer");
        assert!(input[1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("session-layer"));
    }

    #[test]
    fn completed_event_preserves_text_and_usage() {
        let mut text = String::new();
        let mut final_text = None;
        let mut usage = Usage::default();
        let mut usage_available = false;
        process_codex_sse_line(
            br#"data: {"type":"response.completed","response":{"output":[{"type":"message","content":[{"type":"output_text","text":"hello"}]}],"usage":{"input_tokens":1000,"output_tokens":50,"input_tokens_details":{"cached_tokens":700}}}}
"#,
            &mut text,
            &mut final_text,
            &mut usage,
            &mut usage_available,
        )
        .expect("event");

        assert_eq!(final_text.as_deref(), Some("hello"));
        assert!(usage_available);
        assert_eq!(usage.prompt_tokens, 300);
        assert_eq!(usage.cache_read_input_tokens, 700);
        assert_eq!(usage.logical_total_tokens(), 1_050);
    }

    #[test]
    fn delta_event_collects_text() {
        let mut text = String::new();
        let mut final_text = None;
        let mut usage = Usage::default();
        let mut usage_available = false;
        process_codex_sse_line(
            br#"data: {"type":"response.output_text.delta","delta":"hi"}
"#,
            &mut text,
            &mut final_text,
            &mut usage,
            &mut usage_available,
        )
        .expect("event");
        assert_eq!(text, "hi");
    }

    #[test]
    fn error_event_never_reflects_message_type_or_code() {
        const MESSAGE_SENTINEL: &str = "CODEX_SIMPLE_MESSAGE_SENTINEL_148b";
        const TYPE_SENTINEL: &str = "CODEX_SIMPLE_TYPE_SENTINEL_55cb";
        const CODE_SENTINEL: &str = "CODEX_SIMPLE_CODE_SENTINEL_210e";
        let line = format!(
            "data: {{\"type\":\"response.failed\",\"error\":{{\"message\":\"{MESSAGE_SENTINEL}\",\"type\":\"{TYPE_SENTINEL}\",\"code\":\"{CODE_SENTINEL}\"}}}}\n"
        );
        let mut text = String::new();
        let mut final_text = None;
        let mut usage = Usage::default();
        let mut usage_available = false;
        let error = process_codex_sse_line(
            line.as_bytes(),
            &mut text,
            &mut final_text,
            &mut usage,
            &mut usage_available,
        )
        .expect_err("provider error event should fail")
        .to_string();

        for sentinel in [MESSAGE_SENTINEL, TYPE_SENTINEL, CODE_SENTINEL] {
            assert!(!error.contains(sentinel));
        }
        assert!(error.contains("message_fingerprint=sha256:"));
        assert!(error.contains("category_fingerprint=sha256:"));
        assert!(error.contains("code_fingerprint=sha256:"));
    }

    #[test]
    fn invalid_json_event_never_reflects_response_content() {
        const SENTINEL: &str = "CODEX_SIMPLE_JSON_SENTINEL_79af";
        let line = format!("data: {{\"message\":\"{SENTINEL}\"\n");
        let mut text = String::new();
        let mut final_text = None;
        let mut usage = Usage::default();
        let mut usage_available = false;
        let error = process_codex_sse_line(
            line.as_bytes(),
            &mut text,
            &mut final_text,
            &mut usage,
            &mut usage_available,
        )
        .expect_err("invalid provider JSON should fail")
        .to_string();

        assert!(!error.contains(SENTINEL));
        assert!(error.contains("message_fingerprint=sha256:"));
    }
}
