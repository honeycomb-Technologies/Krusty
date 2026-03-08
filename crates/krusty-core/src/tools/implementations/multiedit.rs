//! MultiEdit tool - Apply multiple edits to a single file in one operation

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use similar::TextDiff;
use tokio::fs;

use crate::tools::matching;
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct MultiEditTool;

#[derive(Deserialize)]
struct Params {
    file_path: String,
    edits: Vec<EditOp>,
}

#[derive(Deserialize)]
struct EditOp {
    old_string: String,
    new_string: String,
}

#[async_trait]
impl Tool for MultiEditTool {
    fn name(&self) -> &str {
        "multiedit"
    }

    fn description(&self) -> &str {
        "Apply multiple edits to a single file in one operation. Each edit uses fuzzy matching. More efficient than multiple edit calls - reads and writes the file only once."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "edits": {
                    "type": "array",
                    "description": "Array of edit operations to apply sequentially",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": {
                                "type": "string",
                                "description": "The text to replace"
                            },
                            "new_string": {
                                "type": "string",
                                "description": "The replacement text"
                            }
                        },
                        "required": ["old_string", "new_string"]
                    },
                    "minItems": 1
                }
            },
            "required": ["file_path", "edits"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.edits.is_empty() {
            return ToolResult::error("At least one edit is required".to_string());
        }

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

        let original = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to read file: {}", e)),
        };

        let mut content = original.clone();
        let total = params.edits.len();
        let mut applied = 0;
        let mut errors: Vec<String> = Vec::new();

        for (i, edit) in params.edits.iter().enumerate() {
            match matching::fuzzy_find(&content, &edit.old_string) {
                Some(m) => {
                    content = format!(
                        "{}{}{}",
                        &content[..m.start],
                        &edit.new_string,
                        &content[m.end..],
                    );
                    applied += 1;

                    if m.pass > 1 {
                        tracing::debug!(
                            edit_index = i,
                            pass = m.pass,
                            "MultiEdit: fuzzy match on pass {}",
                            m.pass
                        );
                    }
                }
                None => {
                    let preview = if edit.old_string.chars().count() > 60 {
                        let prefix: String = edit.old_string.chars().take(57).collect();
                        format!("{}...", prefix)
                    } else {
                        edit.old_string.clone()
                    };
                    errors.push(format!("Edit {}: string not found: {:?}", i + 1, preview));
                }
            }
        }

        if applied == 0 {
            return ToolResult::error(format!("No edits could be applied:\n{}", errors.join("\n")));
        }

        // Generate diff before writing
        let diff = generate_compact_diff(&original, &content, &path);

        match fs::write(&path, &content).await {
            Ok(_) => {
                let mut msg = format!("Applied {}/{} edits", applied, total);
                if !errors.is_empty() {
                    msg.push_str(&format!(" ({} failed)", errors.len()));
                }

                let data = json!({
                    "message": msg,
                    "edits_applied": applied,
                    "edits_total": total,
                    "file_path": path.display().to_string(),
                    "partial": !errors.is_empty()
                });

                // Partial success still writes the file, so keep success with warnings.
                ToolResult::success_data_with(data, errors, Some(diff), None)
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
