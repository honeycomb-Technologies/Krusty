//! Helpers for rendering edit diffs with source file line numbers.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Find the 1-indexed line number where `old_string` starts in the current file.
pub fn find_start_line_in_file(
    working_dir: &Path,
    file_path: &str,
    old_string: &str,
) -> Option<usize> {
    if old_string.is_empty() {
        return Some(1);
    }

    let path = resolve_edit_path(working_dir, file_path);
    let content = std::fs::read_to_string(path).ok()?;
    let start = content.find(old_string)?;
    Some(
        content[..start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1,
    )
}

/// Extract the first old-file hunk start line from a tool result payload.
pub fn start_line_from_tool_output(output: &str) -> Option<usize> {
    let diff = serde_json::from_str::<Value>(output)
        .ok()
        .and_then(|value| {
            value
                .get("diff")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| output.to_string());

    start_line_from_unified_diff(&diff)
}

fn resolve_edit_path(working_dir: &Path, file_path: &str) -> PathBuf {
    let path = Path::new(file_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_dir.join(path)
    }
}

fn start_line_from_unified_diff(diff: &str) -> Option<usize> {
    diff.lines()
        .find_map(parse_unified_hunk_old_start)
        .map(|line| line.max(1))
}

fn parse_unified_hunk_old_start(line: &str) -> Option<usize> {
    let rest = line.strip_prefix("@@ -")?;
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_start_line_from_json_diff_payload() {
        let output = r#"{"diff":"--- a\n+++ a\n@@ -42,7 +42,8 @@\n-old\n+new\n"}"#;
        assert_eq!(start_line_from_tool_output(output), Some(42));
    }

    #[test]
    fn extracts_start_line_from_plain_diff_payload() {
        let output = "--- a\n+++ a\n@@ -17 +17 @@\n-old\n+new\n";
        assert_eq!(start_line_from_tool_output(output), Some(17));
    }

    #[test]
    fn finds_start_line_in_file() {
        let temp = match tempfile::TempDir::new() {
            Ok(temp) => temp,
            Err(error) => panic!("failed to create temp dir: {error}"),
        };
        let path = temp.path().join("sample.txt");
        if let Err(error) = std::fs::write(&path, "one\ntwo\nneedle\nfour\n") {
            panic!("failed to write sample file: {error}");
        }

        assert_eq!(
            find_start_line_in_file(temp.path(), "sample.txt", "needle\n"),
            Some(3)
        );
    }
}
