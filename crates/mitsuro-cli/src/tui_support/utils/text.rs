//! Text Utilities - Unified text wrapping and formatting
//!
//! This module provides a single source of truth for text wrapping,
//! eliminating duplicate implementations across the codebase.
//!
//! IMPORTANT: All width calculations use unicode display width, not byte length.
//! This correctly handles multi-byte UTF-8 characters and wide characters (CJK, emoji).

use std::borrow::Cow;
use unicode_width::UnicodeWidthStr;

/// Get display width of a string (handles unicode properly)
#[inline]
fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Wrap a single line at word boundaries to fit within max_width
///
/// Words longer than max_width are force-broken by character.
/// Empty lines return a single empty string.
/// Uses unicode display width for proper terminal rendering.
pub fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![line.to_string()];
    }

    // Quick path for short lines (use display width, not byte length)
    if display_width(line) <= max_width {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in line.split_whitespace() {
        let word_width = display_width(word);

        if current.is_empty() {
            // First word on line - use it even if too long
            current = word.to_string();
            current_width = word_width;
        } else if current_width + 1 + word_width <= max_width {
            // Word fits with space
            current.push(' ');
            current.push_str(word);
            current_width += 1 + word_width;
        } else {
            // Word doesn't fit - push current line and start new one
            result.push(current);
            current = word.to_string();
            current_width = word_width;
        }
    }

    // Don't forget the last line
    if !current.is_empty() {
        result.push(current);
    }

    // Handle words longer than max_width by force-breaking them
    result
        .into_iter()
        .flat_map(|s| {
            if display_width(&s) > max_width {
                // Break by characters, respecting display width
                let mut chunks = Vec::new();
                let mut chunk = String::new();
                let mut chunk_width = 0usize;

                for c in s.chars() {
                    let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    if chunk_width + char_width > max_width && !chunk.is_empty() {
                        chunks.push(chunk);
                        chunk = String::new();
                        chunk_width = 0;
                    }
                    chunk.push(c);
                    chunk_width += char_width;
                }
                if !chunk.is_empty() {
                    chunks.push(chunk);
                }
                chunks
            } else {
                vec![s]
            }
        })
        .collect()
}

/// Wrap multi-line text at word boundaries
///
/// Preserves empty lines. Each input line is wrapped independently.
/// Uses unicode display width for proper terminal rendering.
pub fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.is_empty() {
        return vec![];
    }

    let mut result = Vec::new();

    for line in text.lines() {
        if line.is_empty() {
            result.push(String::new());
            continue;
        }

        result.extend(wrap_line(line, max_width));
    }

    result
}

/// Count how many lines a string will occupy when wrapped at max_width
///
/// This is more efficient than `wrap_line().len()` as it avoids allocations.
/// Must match the wrap_line algorithm exactly.
/// Uses unicode display width for proper terminal rendering.
pub fn count_wrapped_lines(line: &str, max_width: usize) -> usize {
    if max_width == 0 || line.is_empty() {
        return 1;
    }

    let line_width = display_width(line);
    if line_width <= max_width {
        return 1;
    }

    let mut line_count = 0;
    let mut current_width = 0usize;

    for word in line.split_whitespace() {
        let word_width = display_width(word);

        if current_width == 0 {
            // First word on line
            current_width = word_width;
        } else if current_width + 1 + word_width <= max_width {
            // Word fits with space
            current_width += 1 + word_width;
        } else {
            // Word doesn't fit - count current line and start new one
            // Handle words longer than max_width
            if current_width > max_width {
                line_count += current_width.div_ceil(max_width);
            } else {
                line_count += 1;
            }
            current_width = word_width;
        }
    }

    // Count the last line
    if current_width > 0 {
        if current_width > max_width {
            line_count += current_width.div_ceil(max_width);
        } else {
            line_count += 1;
        }
    }

    line_count.max(1)
}

/// Truncate a string to fit within max display width, adding ellipsis if needed.
///
/// Returns `Cow::Borrowed` if no truncation needed (zero allocation).
/// Returns `Cow::Owned` with ellipsis appended if truncation required.
/// Uses unicode display width for proper terminal rendering.
pub fn truncate_ellipsis(s: &str, max_width: usize) -> Cow<'_, str> {
    let current_width = display_width(s);
    if current_width <= max_width {
        return Cow::Borrowed(s);
    }

    // Need at least 4 chars for "X..." pattern
    if max_width < 4 {
        return Cow::Owned(s.chars().take(max_width).collect());
    }

    // Take chars up to max_width - 3 (for "...")
    let target_width = max_width - 3;
    let mut width = 0;
    let truncated: String = s
        .chars()
        .take_while(|c| {
            let char_width = unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
            if width + char_width <= target_width {
                width += char_width;
                true
            } else {
                false
            }
        })
        .collect();

    Cow::Owned(format!("{}...", truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_line_short() {
        assert_eq!(wrap_line("hello", 10), vec!["hello"]);
    }

    #[test]
    fn test_wrap_line_exact() {
        assert_eq!(wrap_line("hello world", 11), vec!["hello world"]);
    }

    #[test]
    fn test_wrap_line_breaks() {
        assert_eq!(wrap_line("hello world foo", 10), vec!["hello", "world foo"]);
    }

    #[test]
    fn test_wrap_line_long_word() {
        let result = wrap_line("superlongword", 5);
        assert_eq!(result, vec!["super", "longw", "ord"]);
    }

    #[test]
    fn test_count_matches_wrap() {
        let text = "the quick brown fox jumps over the lazy dog";
        for width in 5..50 {
            let wrapped = wrap_line(text, width);
            let counted = count_wrapped_lines(text, width);
            assert_eq!(wrapped.len(), counted, "Mismatch at width {}", width);
        }
    }

    #[test]
    fn test_wrap_text_multiline() {
        let text = "hello world\n\nfoo bar";
        let result = wrap_text(text, 8);
        assert_eq!(result, vec!["hello", "world", "", "foo bar"]);
    }
}
