//! Output truncation utilities for tool results
//!
//! Dual-limit truncation (lines + bytes) with head/tail modes.

/// Result of a truncation operation
pub struct TruncationResult {
    pub text: String,
    pub was_truncated: bool,
    pub lines_shown: usize,
    pub lines_total: usize,
    pub bytes_shown: usize,
    pub bytes_total: usize,
}

impl TruncationResult {
    /// Format a truncation notice for appending to output
    pub fn notice(&self) -> Option<String> {
        if !self.was_truncated {
            return None;
        }
        Some(format!(
            "\n[Output truncated: showed {} of {} lines ({}/{} bytes)]",
            self.lines_shown, self.lines_total, self.bytes_shown, self.bytes_total,
        ))
    }
}

/// Tail-truncate: keep the last N lines/bytes.
/// Best for bash output where recent output is most relevant.
pub fn truncate_tail(text: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let bytes_total = text.len();
    let lines: Vec<&str> = text.lines().collect();
    let lines_total = lines.len();

    if lines_total <= max_lines && bytes_total <= max_bytes {
        return TruncationResult {
            text: text.to_string(),
            was_truncated: false,
            lines_shown: lines_total,
            lines_total,
            bytes_shown: bytes_total,
            bytes_total,
        };
    }

    // Apply line limit (take from end)
    let line_limited = if lines_total > max_lines {
        &lines[lines_total - max_lines..]
    } else {
        &lines[..]
    };

    // Join and apply byte limit
    let joined = line_limited.join("\n");
    let (final_text, lines_shown) = if joined.len() > max_bytes {
        // Take last max_bytes bytes, aligned to line boundary
        let skip = joined.len() - max_bytes;
        // Find next newline after skip point
        let start = joined[skip..]
            .find('\n')
            .map(|pos| skip + pos + 1)
            .unwrap_or(skip);
        let trimmed = &joined[start..];
        let shown = trimmed.lines().count();
        (trimmed.to_string(), shown)
    } else {
        let shown = line_limited.len();
        (joined, shown)
    };

    let bytes_shown = final_text.len();
    TruncationResult {
        text: final_text,
        was_truncated: true,
        lines_shown,
        lines_total,
        bytes_shown,
        bytes_total,
    }
}

/// Head-truncate: keep the first N lines/bytes.
/// Best for file reads where the beginning is most relevant.
pub fn truncate_head(text: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let bytes_total = text.len();
    let lines: Vec<&str> = text.lines().collect();
    let lines_total = lines.len();

    if lines_total <= max_lines && bytes_total <= max_bytes {
        return TruncationResult {
            text: text.to_string(),
            was_truncated: false,
            lines_shown: lines_total,
            lines_total,
            bytes_shown: bytes_total,
            bytes_total,
        };
    }

    // Apply line limit (take from start)
    let line_limited = if lines_total > max_lines {
        &lines[..max_lines]
    } else {
        &lines[..]
    };

    // Join and apply byte limit
    let joined = line_limited.join("\n");
    let (final_text, lines_shown) = if joined.len() > max_bytes {
        // Take first max_bytes bytes, aligned to line boundary
        let cutoff = joined[..max_bytes].rfind('\n').unwrap_or(max_bytes);
        let trimmed = &joined[..cutoff];
        let shown = trimmed.lines().count();
        (trimmed.to_string(), shown)
    } else {
        let shown = line_limited.len();
        (joined, shown)
    };

    let bytes_shown = final_text.len();
    TruncationResult {
        text: final_text,
        was_truncated: true,
        lines_shown,
        lines_total,
        bytes_shown,
        bytes_total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_truncation_needed() {
        let text = "line1\nline2\nline3";
        let result = truncate_tail(text, 100, 100_000);
        assert!(!result.was_truncated);
        assert_eq!(result.text, text);
        assert_eq!(result.lines_shown, 3);
    }

    #[test]
    fn test_tail_truncate_by_lines() {
        let text = "line1\nline2\nline3\nline4\nline5";
        let result = truncate_tail(text, 2, 100_000);
        assert!(result.was_truncated);
        assert_eq!(result.lines_shown, 2);
        assert!(result.text.contains("line4"));
        assert!(result.text.contains("line5"));
        assert!(!result.text.contains("line1"));
    }

    #[test]
    fn test_head_truncate_by_lines() {
        let text = "line1\nline2\nline3\nline4\nline5";
        let result = truncate_head(text, 2, 100_000);
        assert!(result.was_truncated);
        assert_eq!(result.lines_shown, 2);
        assert!(result.text.contains("line1"));
        assert!(result.text.contains("line2"));
        assert!(!result.text.contains("line5"));
    }

    #[test]
    fn test_tail_truncate_by_bytes() {
        let text = "a".repeat(100) + "\n" + &"b".repeat(100);
        let result = truncate_tail(&text, 1000, 50);
        assert!(result.was_truncated);
        assert!(result.bytes_shown <= 100);
    }

    #[test]
    fn test_truncation_notice() {
        let text = "line1\nline2\nline3\nline4\nline5";
        let result = truncate_tail(text, 2, 100_000);
        let notice = result.notice().unwrap();
        assert!(notice.contains("2 of 5 lines"));
    }
}
