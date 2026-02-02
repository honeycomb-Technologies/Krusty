//! Write tool - Create or overwrite files

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
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
        "Create new files or completely overwrite existing files. WARNING: Overwrites without backup - prefer 'edit' tool for modifying existing files. Creates parent directories if needed. Reports LSP errors after write. Max 10MB content."
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

        // Check content size to prevent disk exhaustion
        if params.content.len() > MAX_WRITE_SIZE {
            return ToolResult::error(format!(
                "Content too large: {} bytes (max {} MB)",
                params.content.len(),
                MAX_WRITE_SIZE / (1024 * 1024)
            ));
        }

        // Resolve path with sandbox enforcement (handles non-existent parent directories securely)
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

        // Create parent directories if needed
        if let Some(parent) = path.parent().filter(|p| !p.exists()) {
            info!("Write tool: creating parent directory {:?}", parent);
            if let Err(e) = fs::create_dir_all(parent).await {
                return ToolResult::error(format!("Failed to create directory: {}", e));
            }
        }

        match fs::write(&path, &params.content).await {
            Ok(_) => {
                let output = json!({
                    "message": format!("Successfully wrote {} lines", params.content.lines().count()),
                    "bytes_written": params.content.len(),
                    "file_path": path.display().to_string()
                })
                .to_string();

                ToolResult::success(output)
            }
            Err(e) => ToolResult::error(format!("Failed to write file: {}", e)),
        }
    }
}
