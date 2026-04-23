use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::super::core::AiClient;
use super::shared::trim_or_empty;
use crate::ai::transform::apply_request_body_transform;

impl AiClient {
    /// Simple call using ChatGPT Codex (Responses API) format
    ///
    /// Codex requires `stream: true`, so we stream and collect the response.
    pub(super) async fn call_simple_chatgpt_codex(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String> {
        use futures::StreamExt;

        // Build Codex-format request body
        let body = serde_json::json!({
            "model": model,
            "instructions": system_prompt,
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": user_message
                }]
            }],
            "tools": [],
            "store": false,
            "stream": true  // Required by Codex
        });

        debug!("ChatGPT Codex simple call to model: {}", model);

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        // Stream and collect text
        let mut collected_text = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);

            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        continue;
                    }
                    if let Ok(json) = serde_json::from_str::<Value>(data) {
                        // Handle text delta events
                        if json.get("type").and_then(|t| t.as_str())
                            == Some("response.output_text.delta")
                        {
                            if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                                collected_text.push_str(delta);
                            }
                        }
                    }
                }
            }
        }

        Ok(trim_or_empty(Some(&collected_text)))
    }
}
