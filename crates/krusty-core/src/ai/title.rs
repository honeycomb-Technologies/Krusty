//! AI-powered session title generation
//!
//! Uses a fast/cheap model for concise title generation.

use super::client::AiClient;
use super::providers::ProviderId;

/// Get the fast/cheap model ID for title generation based on provider
fn get_title_model(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Anthropic => "claude-haiku-4-5-20251001",
        ProviderId::OpenRouter => "anthropic/claude-3.5-haiku",
        ProviderId::OpenCodeZen => "minimax-m2.1-free", // Free model that works
        ProviderId::ZAi => "glm-4.5-flash",
        ProviderId::MiniMax => "MiniMax-M2.1",
        ProviderId::Kimi => "kimi-k2-0905-preview",
    }
}

/// System prompt for title generation - designed for zero filler
const TITLE_SYSTEM_PROMPT: &str = "\
Generate a concise session title (3-6 words) that captures the main topic or task.

Rules:
- Output ONLY the title text, nothing else
- No quotes, prefixes, or explanations
- No punctuation at the end
- Use title case
- Be specific to the actual content";

/// Generate a session title from the first user message
///
/// Uses a fast/cheap model for title generation.
/// Falls back to truncation if API call fails.
pub async fn generate_title(client: &AiClient, first_message: &str) -> String {
    // Truncate input to avoid wasting tokens on very long messages
    let truncated: String = first_message.chars().take(500).collect();

    // Get the right model for this provider
    let model = get_title_model(client.provider_id());

    match client
        .call_simple(model, TITLE_SYSTEM_PROMPT, &truncated, 30)
        .await
    {
        Ok(title) if !title.is_empty() => {
            // Ensure title isn't too long (max 60 chars)
            if title.len() > 60 {
                format!("{}...", &title[..57])
            } else {
                title
            }
        }
        Ok(_) => fallback_title(first_message),
        Err(e) => {
            tracing::warn!("AI title generation failed: {}, using fallback", e);
            fallback_title(first_message)
        }
    }
}

/// System prompt for pinch title generation
const PINCH_TITLE_PROMPT: &str = "\
Generate a concise session title (3-6 words) for a continued conversation.

Context:
- This is a continuation of a previous session
- Focus on what comes NEXT, not what was done before
- If direction is provided, emphasize that

Rules:
- Output ONLY the title text, nothing else
- No quotes, prefixes, or explanations
- No punctuation at the end
- Use title case
- Don't use words like 'Continued' or 'Part 2'
- Be specific to the next phase of work";

/// Generate a title for a pinch session
///
/// Uses summary and user direction to create a meaningful title
/// for the continuation session.
pub async fn generate_pinch_title(
    client: &AiClient,
    parent_title: &str,
    summary: &str,
    direction: Option<&str>,
) -> String {
    // Build context for title generation
    let mut context = format!(
        "Previous session: {}\n\nSummary:\n{}",
        parent_title, summary
    );

    if let Some(dir) = direction {
        context.push_str(&format!("\n\nUser's direction for next phase: {}", dir));
    }

    // Truncate to avoid token waste
    let truncated: String = context.chars().take(800).collect();

    // Get the right model for this provider
    let model = get_title_model(client.provider_id());

    match client
        .call_simple(model, PINCH_TITLE_PROMPT, &truncated, 30)
        .await
    {
        Ok(title) if !title.is_empty() => {
            if title.len() > 60 {
                format!("{}...", &title[..57])
            } else {
                title
            }
        }
        Ok(_) => fallback_pinch_title(parent_title),
        Err(e) => {
            tracing::warn!("AI pinch title generation failed: {}, using fallback", e);
            fallback_pinch_title(parent_title)
        }
    }
}

/// Fallback pinch title
fn fallback_pinch_title(parent_title: &str) -> String {
    // Just use a simple continuation indicator
    let truncated: String = parent_title.chars().take(45).collect();
    format!("{} (cont.)", truncated)
}

/// Fallback title generation via simple truncation
fn fallback_title(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("").trim();
    let char_count = first_line.chars().count();

    if char_count <= 50 {
        return first_line.to_string();
    }

    // Truncate at word boundary
    let first_50: String = first_line.chars().take(50).collect();
    if let Some(last_space) = first_50.rfind(char::is_whitespace) {
        let char_idx = first_50[..last_space].chars().count();
        if char_idx > 20 {
            let prefix: String = first_line.chars().take(char_idx).collect();
            return format!("{}...", prefix.trim_end());
        }
    }

    let truncated: String = first_line.chars().take(47).collect();
    format!("{}...", truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_short() {
        assert_eq!(fallback_title("Fix the bug"), "Fix the bug");
    }

    #[test]
    fn test_fallback_long() {
        let long = "This is a very long message that should be truncated at a word boundary";
        let result = fallback_title(long);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 53); // 50 + "..."
    }
}
