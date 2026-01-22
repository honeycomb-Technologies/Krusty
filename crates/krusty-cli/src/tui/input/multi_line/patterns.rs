//! Shared patterns for file reference detection

use regex::Regex;
use std::sync::LazyLock;

/// Pattern for bracketed file paths (images and PDFs)
/// Matches: [path/to/file.png], [image.jpg], [document.pdf], etc.
pub static FILE_REF_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+\.(png|jpe?g|gif|webp|pdf))\]").unwrap());
