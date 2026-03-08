//! Edit tool - Edit files by replacing text with fuzzy matching cascade

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use similar::TextDiff;

use crate::tools::matching;
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
        "String replacement in files with fuzzy matching. Handles whitespace and unicode differences automatically. Requires unique old_string match (or use replace_all:true for bulk rename)."
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
                    "description": "Replace all occurrences (default: false). Only uses exact matching.",
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

        if params.replace_all {
            // replace_all: exact matching only (fuzzy replace-all is dangerous)
            let count = content.matches(&params.old_string).count();
            if count == 0 {
                return ToolResult::error(format!(
                    "String not found in file: {:?}",
                    params.old_string
                ));
            }

            let new_content = content.replace(&params.old_string, &params.new_string);
            let diff = generate_compact_diff(&content, &new_content, &path);

            match fs::write(&path, &new_content).await {
                Ok(_) => {
                    let data = json!({
                        "message": format!("Replaced {} occurrence(s)", count),
                        "replacements": count,
                        "file_path": path.display().to_string()
                    });

                    ToolResult::success_data_with(data, Vec::new(), Some(diff), None)
                }
                Err(e) => ToolResult::error(format!("Failed to write file: {}", e)),
            }
        } else {
            // Single replacement: use fuzzy matching cascade
            let exact_count = content.matches(&params.old_string).count();

            if exact_count > 1 {
                return ToolResult::error(format!(
                    "String found {} times. Use replace_all=true to replace all occurrences, or provide more context to make it unique.",
                    exact_count
                ));
            }

            match matching::fuzzy_find(&content, &params.old_string) {
                Some(m) => {
                    if m.pass > 1 {
                        tracing::debug!(
                            pass = m.pass,
                            file = %path.display(),
                            "Fuzzy edit matched on pass {}",
                            m.pass
                        );
                    }

                    let new_content = format!(
                        "{}{}{}",
                        &content[..m.start],
                        &params.new_string,
                        &content[m.end..]
                    );

                    let diff = generate_compact_diff(&content, &new_content, &path);

                    match fs::write(&path, &new_content).await {
                        Ok(_) => {
                            let mut msg = "Replaced 1 occurrence".to_string();
                            let mut warnings = Vec::new();
                            if m.pass > 1 {
                                msg.push_str(&format!(" (fuzzy match pass {})", m.pass));
                                warnings.push(format!("Used fuzzy matching pass {}", m.pass));
                            }

                            let data = json!({
                                "message": msg,
                                "replacements": 1,
                                "file_path": path.display().to_string(),
                                "match_pass": m.pass
                            });

                            ToolResult::success_data_with(data, warnings, Some(diff), None)
                        }
                        Err(e) => ToolResult::error(format!("Failed to write file: {}", e)),
                    }
                }
                None => {
                    ToolResult::error(format!("String not found in file: {:?}", params.old_string))
                }
            }
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
