//! Zero-token session title derivation.
//!
//! Titles are presentation metadata. Generating them must not issue a hidden
//! provider request, consume a user's quota, or compete with the active turn.

use super::client::AiClient;

const MAX_TITLE_CHARS: usize = 64;
const MAX_TITLE_WORDS: usize = 10;

/// Derive a compact, Unicode-safe title from user-authored text.
pub fn derive_title(content: &str) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return String::new();
    }

    let mut title = String::new();
    let mut truncated = false;
    for (index, word) in normalized.split_whitespace().enumerate() {
        let separator = usize::from(!title.is_empty());
        if index == MAX_TITLE_WORDS
            || title.chars().count() + separator + word.chars().count() > MAX_TITLE_CHARS
        {
            if title.is_empty() {
                title.extend(word.chars().take(MAX_TITLE_CHARS.saturating_sub(1)));
            }
            truncated = true;
            break;
        }
        if separator == 1 {
            title.push(' ');
        }
        title.push_str(word);
    }

    if truncated {
        while title.chars().count() > MAX_TITLE_CHARS.saturating_sub(1) {
            title.pop();
        }
        title.push('…');
    }
    title
}

/// Backward-compatible async wrapper. The client is intentionally unused:
/// title generation never performs a provider call.
pub async fn generate_title(_client: &AiClient, first_message: &str) -> String {
    derive_title(first_message)
}

/// Derive the next session's title from explicit direction first, then the
/// compacted summary, and finally the parent title.
pub fn derive_pinch_title(parent_title: &str, summary: &str, direction: Option<&str>) -> String {
    if let Some(direction) = direction.filter(|value| !value.trim().is_empty()) {
        return derive_title(direction);
    }

    if let Some(summary_line) = summary
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let title = derive_title(summary_line);
        if !title.is_empty() {
            return title;
        }
    }

    continuation_title(parent_title)
}

/// Backward-compatible async wrapper with no provider request.
pub async fn generate_pinch_title(
    _client: &AiClient,
    parent_title: &str,
    summary: &str,
    direction: Option<&str>,
) -> String {
    derive_pinch_title(parent_title, summary, direction)
}

fn continuation_title(parent_title: &str) -> String {
    const SUFFIX: &str = " (cont.)";
    let available = MAX_TITLE_CHARS.saturating_sub(SUFFIX.chars().count());
    let mut title: String = parent_title.trim().chars().take(available).collect();
    if title.is_empty() {
        return "Continued session".to_string();
    }
    title.push_str(SUFFIX);
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_compact_and_whitespace_normalized() {
        assert_eq!(
            derive_title("  Build  a\nsmall provider parity dashboard for me please  "),
            "Build a small provider parity dashboard for me please"
        );
    }

    #[test]
    fn title_has_a_unicode_safe_bound() {
        let title = derive_title(
            "🦀 implement reliable cancellation streaming persistence telemetry provider parity and graceful recovery now",
        );
        assert!(title.chars().count() <= MAX_TITLE_CHARS);
        assert!(title.ends_with('…'));

        let long_unbroken = derive_title(&"🦀".repeat(100));
        assert_eq!(long_unbroken.chars().count(), MAX_TITLE_CHARS);
        assert!(long_unbroken.ends_with('…'));
    }

    #[test]
    fn pinch_title_prefers_user_direction_without_provider_work() {
        assert_eq!(
            derive_pinch_title(
                "Old topic",
                "Previous work summary",
                Some("Verify the mobile release on a physical device")
            ),
            "Verify the mobile release on a physical device"
        );
    }

    #[test]
    fn empty_pinch_context_has_a_stable_fallback() {
        assert_eq!(
            derive_pinch_title("Core audit", "", None),
            "Core audit (cont.)"
        );
        assert_eq!(derive_pinch_title("", "", None), "Continued session");
    }
}
