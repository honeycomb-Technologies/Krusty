//! File reference parser for user input
//!
//! Parses file paths and URLs from input text.
//! Supports:
//! - Bracketed paths: [/path/to/file.pdf]
//! - Raw paths: /path/to/file.pdf (when pasted)
//! - URLs: https://example.com/image.png

use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};

/// A segment of parsed input
#[derive(Debug, Clone)]
pub enum InputSegment {
    /// Plain text content
    Text(String),
    /// Local file path (image or PDF)
    ImagePath(PathBuf),
    /// URL to an image
    ImageUrl(String),
    /// Clipboard image reference (from paste)
    ClipboardImage(String),
}

// Bracketed patterns: [path] or [url]
static BRACKET_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]+)\]").unwrap());

// Raw file paths with supported extensions (pdf, images)
// Matches: /path/file.ext, ./file.ext, ../file.ext, ~/file.ext
static RAW_PATH_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:^|[\s])(((?:/|\.{1,2}/|~/)[\w./-]+)\.(pdf|png|jpe?g|gif|webp))(?:[\s]|$)")
        .unwrap()
});

/// Parse input text for file references
///
/// Recognizes:
/// - `[./path/to/image.png]` - bracketed relative paths
/// - `[/absolute/path/image.jpg]` - bracketed absolute paths
/// - `[https://example.com/image.png]` - bracketed URLs
/// - `[clipboard:id]` - clipboard image references
/// - `/path/to/file.pdf` - raw paths (when pasted)
/// - `./file.png` - raw relative paths
pub fn parse_input(text: &str, working_dir: &Path) -> Vec<InputSegment> {
    // First pass: find all bracketed patterns and raw paths
    let mut matches: Vec<(usize, usize, InputSegment)> = Vec::new();

    // Find bracketed patterns
    for cap in BRACKET_PATTERN.captures_iter(text) {
        let Some(full_match) = cap.get(0) else {
            continue;
        };
        let Some(inner_match) = cap.get(1) else {
            continue;
        };
        let inner = inner_match.as_str().trim();

        let segment = if inner.starts_with("http://") || inner.starts_with("https://") {
            InputSegment::ImageUrl(inner.to_string())
        } else if inner.starts_with("clipboard:") {
            InputSegment::ClipboardImage(inner.to_string())
        } else if has_supported_extension(inner) {
            let path = if Path::new(inner).is_absolute() {
                PathBuf::from(inner)
            } else {
                working_dir.join(inner)
            };
            InputSegment::ImagePath(path)
        } else {
            // Not a recognized file type — skip, leave as plain text
            continue;
        };

        matches.push((full_match.start(), full_match.end(), segment));
    }

    // Find raw file paths (not inside brackets)
    for cap in RAW_PATH_PATTERN.captures_iter(text) {
        let Some(path_match) = cap.get(1) else {
            continue;
        };
        let start = path_match.start();
        let end = path_match.end();

        // Skip if this overlaps with a bracket match
        let overlaps = matches
            .iter()
            .any(|(s, e, _)| (start >= *s && start < *e) || (end > *s && end <= *e));
        if overlaps {
            continue;
        }

        let path_str = path_match.as_str();
        let path = if let Some(rest) = path_str.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(rest)
            } else {
                PathBuf::from(path_str)
            }
        } else if path_str == "~" {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"))
        } else if Path::new(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            working_dir.join(path_str)
        };

        matches.push((start, end, InputSegment::ImagePath(path)));
    }

    // Sort by start position
    matches.sort_by_key(|(start, _, _)| *start);

    // Build segments with text between matches
    let mut segments = Vec::new();
    let mut last_end = 0;

    for (start, end, segment) in matches {
        // Add text before this match
        if start > last_end {
            let text_before = &text[last_end..start];
            let trimmed = text_before.trim();
            if !trimmed.is_empty() {
                segments.push(InputSegment::Text(trimmed.to_string()));
            }
        }
        segments.push(segment);
        last_end = end;
    }

    // Add remaining text
    if last_end < text.len() {
        let remaining = text[last_end..].trim();
        if !remaining.is_empty() {
            segments.push(InputSegment::Text(remaining.to_string()));
        }
    }

    // If no matches, return the whole text
    if segments.is_empty() && !text.trim().is_empty() {
        segments.push(InputSegment::Text(text.trim().to_string()));
    }

    segments
}

/// Supported file extensions for attachment
const SUPPORTED_EXTENSIONS: &[&str] = &["pdf", "png", "jpg", "jpeg", "gif", "webp"];

fn has_supported_extension(s: &str) -> bool {
    let path = Path::new(s);
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Check if input contains any file references
pub fn has_file_references(text: &str) -> bool {
    if RAW_PATH_PATTERN.is_match(text) {
        return true;
    }
    BRACKET_PATTERN.captures_iter(text).any(|cap| {
        let Some(inner_match) = cap.get(1) else {
            return false;
        };
        let inner = inner_match.as_str().trim();
        inner.starts_with("http://")
            || inner.starts_with("https://")
            || inner.starts_with("clipboard:")
            || has_supported_extension(inner)
    })
}

/// Backwards compat alias
pub fn has_image_references(text: &str) -> bool {
    has_file_references(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_text() {
        let segments = parse_input("Hello world", Path::new("/home"));
        assert_eq!(segments.len(), 1);
        assert!(matches!(&segments[0], InputSegment::Text(t) if t == "Hello world"));
    }

    #[test]
    fn test_parse_bracketed_image() {
        let segments = parse_input("Check [./image.png]", Path::new("/home"));
        assert_eq!(segments.len(), 2);
        assert!(matches!(&segments[0], InputSegment::Text(t) if t == "Check"));
        assert!(matches!(&segments[1], InputSegment::ImagePath(p) if p.ends_with("image.png")));
    }

    #[test]
    fn test_parse_url() {
        let segments = parse_input("[https://example.com/img.jpg]", Path::new("/home"));
        assert_eq!(segments.len(), 1);
        assert!(
            matches!(&segments[0], InputSegment::ImageUrl(u) if u == "https://example.com/img.jpg")
        );
    }

    #[test]
    fn test_parse_clipboard_image_reference() {
        let segments = parse_input("Describe [clipboard:abc-123]", Path::new("/home"));
        assert_eq!(segments.len(), 2);
        assert!(matches!(&segments[0], InputSegment::Text(t) if t == "Describe"));
        assert!(
            matches!(&segments[1], InputSegment::ClipboardImage(id) if id == "clipboard:abc-123")
        );
        assert!(has_file_references("Describe [clipboard:abc-123]"));
    }

    #[test]
    fn test_parse_raw_absolute_path() {
        let segments = parse_input("/home/user/doc.pdf", Path::new("/work"));
        assert_eq!(segments.len(), 1);
        assert!(
            matches!(&segments[0], InputSegment::ImagePath(p) if p.to_str().unwrap() == "/home/user/doc.pdf")
        );
    }

    #[test]
    fn test_parse_raw_path_with_text() {
        let segments = parse_input("analyze /tmp/file.png please", Path::new("/home"));
        assert_eq!(segments.len(), 3);
        assert!(matches!(&segments[0], InputSegment::Text(t) if t == "analyze"));
        assert!(
            matches!(&segments[1], InputSegment::ImagePath(p) if p.to_str().unwrap() == "/tmp/file.png")
        );
        assert!(matches!(&segments[2], InputSegment::Text(t) if t == "please"));
    }

    #[test]
    fn test_parse_raw_relative_path() {
        let segments = parse_input("./docs/report.pdf", Path::new("/home/user"));
        assert_eq!(segments.len(), 1);
        assert!(
            matches!(&segments[0], InputSegment::ImagePath(p) if p.ends_with("docs/report.pdf"))
        );
    }

    #[test]
    fn test_brackets_without_file_extension_are_plain_text() {
        let text = "error [ble: exit 101] happened";
        let segments = parse_input(text, Path::new("/home"));
        assert_eq!(segments.len(), 1);
        assert!(matches!(&segments[0], InputSegment::Text(t) if t == text));
        assert!(!has_file_references(text));
    }

    #[test]
    fn test_brackets_with_supported_extension_are_file_paths() {
        let segments = parse_input("[screenshot.png]", Path::new("/home"));
        assert_eq!(segments.len(), 1);
        assert!(matches!(&segments[0], InputSegment::ImagePath(_)));
        assert!(has_file_references("[screenshot.png]"));
    }

    #[test]
    fn test_mixed_brackets_file_and_non_file() {
        let segments = parse_input("[note] check [./photo.jpg]", Path::new("/home"));
        assert_eq!(segments.len(), 2);
        assert!(matches!(&segments[0], InputSegment::Text(t) if t == "[note] check"));
        assert!(matches!(&segments[1], InputSegment::ImagePath(p) if p.ends_with("photo.jpg")));
    }
}
