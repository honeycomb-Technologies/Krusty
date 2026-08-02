//! Shared patterns for file reference detection

use regex::Regex;
use std::sync::LazyLock;

/// Pattern for bracketed file paths (any file type)
/// Matches: [path/to/file.rs], [image.png], [document.pdf], etc.
/// The path inside brackets can be any valid file path with an extension
pub static FILE_REF_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]\s]+\.[a-zA-Z0-9]+)\]").unwrap());
