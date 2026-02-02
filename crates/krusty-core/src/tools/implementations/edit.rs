//! Edit tool - Edit files by replacing text

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use similar::TextDiff;

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct EditTool;

#[derive(Deserialize)]
struct Params {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Exact string replacement in files. Requires unique old_string match (or use replace_all:true for bulk rename). Reports LSP errors after modification."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        // Use sandboxed resolve for multi-tenant path isolation
        let path = match ctx.sandboxed_resolve(&params.file_path) {
            Ok(p) => p,
            Err(e) => {
                let fallback = ctx.resolve_path(&params.file_path);
                if !fallback.exists() {
                    return ToolResult::error(format!("File not found: {}", params.file_path));
                }
                return ToolResult::error(e);
            }
        };

        if !path.exists() {
            return ToolResult::error(format!("File not found: {}", path.display()));
        }

        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to read file: {}", e)),
        };

        let count = content.matches(&params.old_string).count();

        if count == 0 {
            return ToolResult::error(format!("String not found in file: {:?}", params.old_string));
        }

        if count > 1 && !params.replace_all {
            return ToolResult::error(format!(
                "String found {} times. Use replace_all=true to replace all occurrences, or provide more context to make it unique.",
                count
            ));
        }

        let new_content = if params.replace_all {
            content.replace(&params.old_string, &params.new_string)
        } else {
            content.replacen(&params.old_string, &params.new_string, 1)
        };

        let diff = generate_compact_diff(&content, &new_content, &path);

        match fs::write(&path, &new_content).await {
            Ok(_) => {
                let replaced = if params.replace_all { count } else { 1 };
                let mut output = json!({
                    "message": format!("Replaced {} occurrence(s)", replaced),
                    "replacements": replaced,
                    "file_path": path.display().to_string()
                })
                .to_string();

                if !diff.is_empty() {
                    output.push_str("\n\n[DIFF]\n");
                    output.push_str(&diff);
                }

                ToolResult::success(output)
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
