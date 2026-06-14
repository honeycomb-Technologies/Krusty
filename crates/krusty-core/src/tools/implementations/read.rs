//! Read tool - Read file contents with file suggestions on not-found

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

/// Maximum file size to read into memory (10 MB)
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum number of file suggestions on not-found
const MAX_SUGGESTIONS: usize = 5;

pub struct ReadTool;

#[derive(Deserialize)]
struct Params {
    file_path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read file contents. Supports line offset/limit for large files. Detects binary files. Suggests similar filenames when file not found."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Prefer absolute paths for clarity; relative paths resolve against the current working directory. Default reads 2000 lines; use offset/limit for large files.

Results include 1-indexed line numbers. Binary files are rejected; images/PDFs can be read for multimodal analysis.

Read a file before modifying it: edit, multiedit, and overwrite-via-write enforce this."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "The line number to start reading from (1-indexed)"
                },
                "limit": {
                    "type": "integer",
                    "description": "The number of lines to read"
                }
            },
            "required": ["file_path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        // Resolve under the configured filesystem access policy.
        let path = match ctx.sandboxed_resolve(&params.file_path) {
            Ok(p) => p,
            Err(e) => {
                let fallback = ctx.resolve_path(&params.file_path);
                if !fallback.exists() {
                    let suggestions = find_suggestions(&params.file_path, ctx);
                    let mut msg = format!("File not found: {}", params.file_path);
                    if !suggestions.is_empty() {
                        msg.push_str("\n\nDid you mean:\n");
                        for s in &suggestions {
                            msg.push_str(&format!("  - {}\n", s));
                        }
                    }
                    return ToolResult::error(msg);
                }
                return ToolResult::error(e);
            }
        };

        if !path.is_file() {
            return ToolResult::error(format!("Path is not a file: {}", path.display()));
        }

        let canonical_path = match fs::canonicalize(&path).await {
            Ok(path) => path,
            Err(e) => return ToolResult::error(format!("Failed to resolve file path: {}", e)),
        };

        // Check file size before reading to prevent memory exhaustion
        let metadata = match fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => return ToolResult::error(format!("Failed to read file metadata: {}", e)),
        };

        if metadata.len() > MAX_FILE_SIZE {
            return ToolResult::error(format!(
                "File too large: {} bytes (max {} MB). Use offset/limit to read portions.",
                metadata.len(),
                MAX_FILE_SIZE / (1024 * 1024)
            ));
        }

        let content = match fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(e) => return ToolResult::error(format!("Failed to read file: {}", e)),
        };

        // Check for binary
        let check_len = content.len().min(8192);
        if content[..check_len].contains(&0) {
            let size = content.len();
            let size_str = match size {
                0..1024 => format!("{} bytes", size),
                1024..1_048_576 => format!("{:.1} KB", size as f64 / 1024.0),
                _ => format!("{:.1} MB", size as f64 / 1_048_576.0),
            };
            ctx.record_file_observation(canonical_path);
            return ToolResult::success_data(json!({
                "content": format!("Binary file: {} ({})", path.display(), size_str),
                "total_lines": 0,
                "lines_returned": 0,
                "start_line": 1
            }));
        }

        let content = match String::from_utf8(content) {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("File is not valid UTF-8: {}", e)),
        };

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = params.offset.unwrap_or(1).saturating_sub(1);
        let limit = params.limit.unwrap_or(2000);
        let end = (start + limit).min(total_lines);

        if start >= total_lines {
            return ToolResult::error(format!(
                "Start line {} is beyond file length ({})",
                start + 1,
                total_lines
            ));
        }

        let content = lines[start..end].join("\n");

        ctx.record_file_observation(canonical_path);

        ToolResult::success_data(json!({
            "content": content,
            "total_lines": total_lines,
            "lines_returned": end - start,
            "start_line": start + 1
        }))
    }
}

/// Search for files with a similar name when the requested path is not found.
fn find_suggestions(file_path: &str, ctx: &ToolContext) -> Vec<String> {
    let path = std::path::Path::new(file_path);
    let filename = match path.file_name().and_then(|f| f.to_str()) {
        Some(f) => f,
        None => return Vec::new(),
    };

    let search_root = ctx
        .filesystem_access_root()
        .unwrap_or_else(|| ctx.working_dir.clone());

    let pattern = format!("**/{}", filename);
    let full_pattern = format!("{}/{}", search_root.display(), pattern);

    let mut suggestions = Vec::new();
    let matches = glob::glob(&full_pattern).ok();

    if let Some(paths) = matches {
        for entry in paths.flatten() {
            suggestions.push(entry.display().to_string());
            if suggestions.len() >= MAX_SUGGESTIONS {
                break;
            }
        }
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn successful_read_records_canonical_file_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
        fs::write(&file_path, "hello\nworld\n")
            .await
            .expect("test file should write");
        let canonical = file_path
            .canonicalize()
            .expect("test file should canonicalize");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = ReadTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "limit": 1,
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error, "unexpected read error: {}", result.output);
        assert!(ctx.has_file_observation(&canonical));
        assert_eq!(ctx.observed_files_snapshot(), vec![canonical]);
    }

    #[tokio::test]
    async fn failed_read_does_not_record_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let missing = temp_dir.path().join("missing.txt");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = ReadTool
            .execute(json!({ "file_path": missing.display().to_string() }), &ctx)
            .await;

        assert!(result.is_error);
        assert!(ctx.observed_files_snapshot().is_empty());
    }
}
