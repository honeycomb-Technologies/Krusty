//! Write tool - Create or overwrite files with diff output

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use similar::TextDiff;
use tokio::fs;
use tracing::info;

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

/// Maximum content size to write (10 MB)
const MAX_WRITE_SIZE: usize = 10 * 1024 * 1024;

pub struct WriteTool;

#[derive(Deserialize)]
struct Params {
    file_path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Create new files or completely overwrite existing files. Shows diff when overwriting. WARNING: Overwrites without backup - prefer 'edit' tool for modifying existing files. Creates parent directories if needed. Max 10MB content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.content.len() > MAX_WRITE_SIZE {
            return ToolResult::error(format!(
                "Content too large: {} bytes (max {} MB)",
                params.content.len(),
                MAX_WRITE_SIZE / (1024 * 1024)
            ));
        }

        let path = match ctx.sandboxed_resolve_new_path(&params.file_path) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult::error(format!("Access denied: {}", e));
            }
        };
        info!(
            "Write tool: resolved path = {:?}, working_dir = {:?}",
            path, ctx.working_dir
        );

        // Read existing content for diff (before writing)
        let old_content = if path.is_file() {
            fs::read_to_string(&path).await.ok()
        } else {
            None
        };

        // Create parent directories if needed
        if let Some(parent) = path.parent().filter(|p| !p.exists()) {
            info!("Write tool: creating parent directory {:?}", parent);
            if let Err(e) = fs::create_dir_all(parent).await {
                return ToolResult::error(format!("Failed to create directory: {}", e));
            }
        }

        match fs::write(&path, &params.content).await {
            Ok(_) => {
                let line_count = params.content.lines().count();

                let data = match &old_content {
                    Some(_) => json!({
                        "message": format!("Successfully overwrote file ({} lines)", line_count),
                        "bytes_written": params.content.len(),
                        "line_count": line_count,
                        "file_path": path.display().to_string()
                    }),
                    None => json!({
                        "message": format!("Created new file ({} lines)", line_count),
                        "bytes_written": params.content.len(),
                        "line_count": line_count,
                        "file_path": path.display().to_string()
                    }),
                };

                let diff = if let Some(old) = &old_content {
                    let diff = generate_compact_diff(old, &params.content, &path);
                    if !diff.is_empty() {
                        Some(diff)
                    } else {
                        None
                    }
                } else {
                    None
                };

                ToolResult::success_data_with(data, Vec::new(), diff, None)
            }
            Err(e) => ToolResult::error(format!("Failed to write file: {}", e)),
        }
    }
}

fn generate_compact_diff(old: &str, new: &str, path: &std::path::Path) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut output = String::new();
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        output.push_str(&format!("{}", hunk));
    }
    if output.is_empty() {
        return String::new();
    }
    format!("--- {}\n+++ {}\n{}", path.display(), path.display(), output)
}
