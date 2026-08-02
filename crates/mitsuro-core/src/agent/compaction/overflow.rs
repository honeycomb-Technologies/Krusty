//! Detect provider context-overflow errors for reactive compaction.

/// Returns true when an API/stream error indicates the request exceeded the model context window.
pub fn is_context_overflow_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "413",
        "request too large",
        "payload too large",
        "context length",
        "context window",
        "context_length",
        "maximum context",
        "max context",
        "too many tokens",
        "token limit",
        "tokens exceed",
        "exceeds the context",
        "exceeds context",
        "input is too long",
        "prompt is too long",
        "prompt too long",
        "message is too long",
        "messages are too long",
        "context overflow",
        "context limit",
    ];

    MARKERS.iter().any(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::is_context_overflow_error;

    #[test]
    fn detects_http_413_errors() {
        assert!(is_context_overflow_error(
            "AI error: HTTP 413 Payload Too Large from provider"
        ));
    }

    #[test]
    fn detects_context_window_phrasing() {
        assert!(is_context_overflow_error(
            "prompt is too long: maximum context length is 200000 tokens"
        ));
    }

    #[test]
    fn ignores_unrelated_errors() {
        assert!(!is_context_overflow_error("rate limit exceeded"));
        assert!(!is_context_overflow_error("invalid api key"));
    }
}
