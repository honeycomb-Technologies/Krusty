//! AI-powered conversation summarization for pinch
//!
//! Uses extended thinking when available (Anthropic) or falls back to
//! simple API calls for other providers. Produces a structured summary
//! for the next session.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::ai::client::AiClient;
use crate::ai::providers::ProviderId;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::RankedFile;

/// Default Sonnet model for Anthropic summarization with extended thinking
const ANTHROPIC_SUMMARIZATION_MODEL: &str = "claude-sonnet-4-5-20250929";

/// Extended thinking budget for thorough analysis (Anthropic only)
///
/// 32K tokens allows the model to deeply analyze the conversation context
/// and produce high-quality summaries. Extended thinking is more effective
/// than simple prompts for complex reasoning tasks.
const THINKING_BUDGET: u32 = 32000;

/// Max tokens for non-thinking summarization calls
///
/// 4000 tokens is sufficient for summary output on non-Anthropic providers
/// that don't support extended thinking. The output is structured JSON,
/// so it doesn't need to be verbose.
const SUMMARIZATION_MAX_TOKENS: usize = 4000;

/// System prompt for summarization
const SUMMARIZATION_SYSTEM_PROMPT: &str = r#"You are a specialized summarization agent for pinch (context continuation) between coding sessions.

Your task is to analyze a conversation history and produce a structured summary that will help the next session continue effectively.

## Output Format

You MUST respond with a valid JSON object (no markdown code blocks, no extra text):
{
  "work_summary": "2-3 paragraph summary of what was accomplished, focusing on the WHY and WHAT",
  "key_decisions": ["Important architectural or design decisions made"],
  "pending_tasks": ["Incomplete work or clearly identified next steps"],
  "important_files": ["Top 10 most relevant file paths for continuing work"]
}

## Guidelines

1. **Work Summary**: Focus on accomplishments, not mechanics. What was built? What problems were solved? What's the current state?

2. **Key Decisions**: Capture architectural choices, patterns adopted, trade-offs made. These are things the next session needs to understand.

3. **Pending Tasks**: Identify explicitly mentioned TODOs, incomplete work, or logical next steps. Be specific.

4. **Important Files**: List files most critical for continuing the work. Prioritize files that were modified or are central to the work.

## Priority

If the user provided preservation hints, weight those areas HEAVILY in your summary. The user knows what matters most."#;

/// Result from the summarization AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationResult {
    pub work_summary: String,
    pub key_decisions: Vec<String>,
    pub pending_tasks: Vec<String>,
    pub important_files: Vec<String>,
}

impl Default for SummarizationResult {
    fn default() -> Self {
        Self {
            work_summary: "No summary available.".to_string(),
            key_decisions: Vec::new(),
            pending_tasks: Vec::new(),
            important_files: Vec::new(),
        }
    }
}

/// Build the user message for summarization
///
/// Includes:
/// - Full conversation history (condensed)
/// - Key file contents
/// - CLAUDE.md / KRAB.md
/// - User's preservation hints
pub fn build_summarization_prompt(
    conversation: &[ModelMessage],
    preservation_hints: Option<&str>,
    ranked_files: &[RankedFile],
    file_contents: &[(String, String)], // (path, content)
    project_context: Option<&str>,      // CLAUDE.md content
) -> String {
    let mut prompt = String::new();

    // User's preservation hints (highest priority)
    if let Some(hints) = preservation_hints {
        prompt.push_str("## USER'S PRESERVATION PRIORITIES (IMPORTANT)\n\n");
        prompt.push_str(hints);
        prompt.push_str("\n\nWeight these areas heavily in your summary.\n\n");
    }

    // Project context
    if let Some(ctx) = project_context {
        prompt.push_str("## PROJECT CONTEXT (CLAUDE.md)\n\n");
        // Truncate if too long
        if ctx.len() > 5000 {
            prompt.push_str(&ctx[..5000]);
            prompt.push_str("\n...[truncated]\n");
        } else {
            prompt.push_str(ctx);
        }
        prompt.push_str("\n\n");
    }

    // Ranked files for reference
    if !ranked_files.is_empty() {
        prompt.push_str("## FILES BY ACTIVITY\n\n");
        for (i, file) in ranked_files.iter().take(20).enumerate() {
            let reasons = if file.reasons.is_empty() {
                String::new()
            } else {
                format!(" ({})", file.reasons.join(", "))
            };
            prompt.push_str(&format!("{}. {}{}\n", i + 1, file.path, reasons));
        }
        prompt.push('\n');
    }

    // Key file contents
    if !file_contents.is_empty() {
        prompt.push_str("## KEY FILE CONTENTS\n\n");
        for (path, content) in file_contents.iter().take(10) {
            prompt.push_str(&format!("### {}\n```\n", path));
            // Truncate long files
            if content.len() > 3000 {
                prompt.push_str(&content[..3000]);
                prompt.push_str("\n...[truncated]\n");
            } else {
                prompt.push_str(content);
            }
            prompt.push_str("\n```\n\n");
        }
    }

    // Conversation history
    prompt.push_str("## CONVERSATION HISTORY\n\n");
    for msg in conversation {
        let role_str = match msg.role {
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
            Role::System => continue, // Skip system messages
            Role::Tool => continue,   // Skip tool role messages
        };

        for content in &msg.content {
            match content {
                Content::Text { text } => {
                    // Truncate very long messages
                    let preview = if text.len() > 2000 {
                        format!(
                            "{}...[truncated, {} chars total]",
                            &text[..2000],
                            text.len()
                        )
                    } else {
                        text.clone()
                    };
                    prompt.push_str(&format!("{}: {}\n\n", role_str, preview));
                }
                Content::ToolUse { name, input, .. } => {
                    let input_preview = summarize_tool_input(name, input);
                    prompt.push_str(&format!(
                        "{}: [Tool: {} - {}]\n\n",
                        role_str, name, input_preview
                    ));
                }
                Content::ToolResult {
                    output, is_error, ..
                } => {
                    let status = if is_error.unwrap_or(false) {
                        "ERROR"
                    } else {
                        "OK"
                    };
                    // Brief preview of result from the JSON output
                    let preview = output
                        .as_str()
                        .map(|s| {
                            if s.len() > 200 {
                                format!("{}...", &s[..200])
                            } else {
                                s.to_string()
                            }
                        })
                        .unwrap_or_else(|| "[output]".to_string());
                    prompt.push_str(&format!(
                        "{}: [Result: {} - {}]\n\n",
                        role_str, status, preview
                    ));
                }
                Content::Thinking { thinking, .. } => {
                    // Brief thinking preview
                    if thinking.len() > 500 {
                        prompt.push_str(&format!(
                            "{}: [Thinking: {}...]\n\n",
                            role_str,
                            &thinking[..500]
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    prompt
}

/// Summarize tool input for the prompt
fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    match tool_name.to_lowercase().as_str() {
        "read" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|p| format!("reading {}", p))
            .unwrap_or_else(|| "reading file".to_string()),
        "write" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|p| format!("writing {}", p))
            .unwrap_or_else(|| "writing file".to_string()),
        "edit" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|p| format!("editing {}", p))
            .unwrap_or_else(|| "editing file".to_string()),
        "bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| {
                if c.len() > 50 {
                    format!("{}...", &c[..50])
                } else {
                    c.to_string()
                }
            })
            .unwrap_or_else(|| "running command".to_string()),
        "glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| format!("searching for {}", p))
            .unwrap_or_else(|| "glob search".to_string()),
        "grep" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| format!("searching for '{}'", p))
            .unwrap_or_else(|| "grep search".to_string()),
        _ => format!("{} operation", tool_name),
    }
}

/// Generate a summary using the appropriate method for the provider
///
/// For Anthropic: Uses Sonnet 4.5 with extended thinking for deep analysis
/// For other providers: Uses the current model with a simple call
pub async fn generate_summary(
    client: &AiClient,
    conversation: &[ModelMessage],
    preservation_hints: Option<&str>,
    ranked_files: &[RankedFile],
    file_contents: &[(String, String)],
    project_context: Option<&str>,
    current_model: Option<&str>,
) -> Result<SummarizationResult> {
    let prompt = build_summarization_prompt(
        conversation,
        preservation_hints,
        ranked_files,
        file_contents,
        project_context,
    );

    tracing::info!(
        "Starting summarization with {} conversation messages, {} file contents",
        conversation.len(),
        file_contents.len()
    );

    let provider = client.provider_id();
    let response = if provider == ProviderId::Anthropic {
        // Anthropic: Use extended thinking for deep analysis
        tracing::info!(
            "Using extended thinking with model {}",
            ANTHROPIC_SUMMARIZATION_MODEL
        );
        client
            .call_with_thinking(
                ANTHROPIC_SUMMARIZATION_MODEL,
                SUMMARIZATION_SYSTEM_PROMPT,
                &prompt,
                THINKING_BUDGET,
            )
            .await?
    } else {
        // Other providers: Use simple call with current model
        let model = current_model.unwrap_or_else(|| get_fallback_model(provider));
        tracing::info!(
            "Using simple call with model {} for provider {:?}",
            model,
            provider
        );
        client
            .call_simple(
                model,
                SUMMARIZATION_SYSTEM_PROMPT,
                &prompt,
                SUMMARIZATION_MAX_TOKENS,
            )
            .await?
    };

    // Parse the JSON response
    parse_summary_response(&response)
}

/// Get a reasonable fallback model for summarization based on provider
fn get_fallback_model(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Anthropic => ANTHROPIC_SUMMARIZATION_MODEL,
        ProviderId::OpenRouter => "anthropic/claude-3.5-haiku", // Fast and cheap on OpenRouter
        ProviderId::OpenCodeZen => "minimax-m2.1-free",         // Free tier model
        ProviderId::ZAi => "GLM-4.5-Air",                       // Fast GLM model
        ProviderId::MiniMax => "MiniMax-M2.1",                  // MiniMax default
        ProviderId::Kimi => "kimi-for-coding",                  // Kimi Code API model
        ProviderId::OpenAI => "gpt-4o-mini",                    // Fast and cheap OpenAI model
    }
}

/// Parse the JSON response from the summarization AI
fn parse_summary_response(response: &str) -> Result<SummarizationResult> {
    // Try to extract JSON from the response
    let json_str = extract_json(response);

    serde_json::from_str(&json_str).map_err(|e| {
        tracing::warn!(
            "Failed to parse summary JSON: {}. Response: {}",
            e,
            response
        );
        anyhow::anyhow!("Failed to parse summarization response: {}", e)
    })
}

/// Extract JSON from response, handling potential markdown wrapping
fn extract_json(response: &str) -> String {
    let trimmed = response.trim();

    // If it starts with {, assume it's raw JSON
    if trimmed.starts_with('{') {
        return trimmed.to_string();
    }

    // Try to extract from markdown code block
    if let Some(start) = trimmed.find("```json") {
        let after_marker = &trimmed[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim().to_string();
        }
    }

    // Try plain code block
    if let Some(start) = trimmed.find("```") {
        let after_marker = &trimmed[start + 3..];
        let content_start = after_marker.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_marker[content_start..];
        if let Some(end) = content.find("```") {
            return content[..end].trim().to_string();
        }
    }

    // Last resort: find first { and last }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if end > start {
            return trimmed[start..=end].to_string();
        }
    }

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_raw() {
        let input = r#"{"work_summary": "test", "key_decisions": [], "pending_tasks": [], "important_files": []}"#;
        let result = extract_json(input);
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
    }

    #[test]
    fn test_extract_json_markdown() {
        let input = r#"Here's the summary:

```json
{"work_summary": "test", "key_decisions": [], "pending_tasks": [], "important_files": []}
```

Done!"#;
        let result = extract_json(input);
        assert!(result.starts_with('{'));
    }

    #[test]
    fn test_parse_summary() {
        let input = r#"{"work_summary": "Built a feature", "key_decisions": ["Used Rust"], "pending_tasks": ["Add tests"], "important_files": ["src/main.rs"]}"#;
        let result = parse_summary_response(input).unwrap();
        assert_eq!(result.work_summary, "Built a feature");
        assert_eq!(result.key_decisions, vec!["Used Rust"]);
    }
}
