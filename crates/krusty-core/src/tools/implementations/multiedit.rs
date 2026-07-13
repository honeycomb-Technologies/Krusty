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

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use for 3+ edits to the same file. Read the file first; each edit still needs a unique old_string match.

Edits apply sequentially, so later edits see earlier changes. Prefer this over multiple separate edit calls for one file."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to modify"
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

        let path = match ctx.require_file_observation(&path) {
            Ok(path) => path,
            Err(e) => return ToolResult::error_with_code("read_required", e),
        };

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
                        edit.new_string,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn multiedit_requires_prior_file_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
        fs::write(&file_path, "one\ntwo\n")
            .await
            .expect("test file should write");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = MultiEditTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "edits": [{ "old_string": "one", "new_string": "ONE" }],
                }),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["error"]["code"], "read_required");
    }

    #[tokio::test]
    async fn multiedit_succeeds_after_prior_file_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
        fs::write(&file_path, "one\ntwo\n")
            .await
            .expect("test file should write");
        let canonical = file_path
            .canonicalize()
            .expect("test file should canonicalize");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        ctx.record_file_observation(canonical);

        let result = MultiEditTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "edits": [
                        { "old_string": "one", "new_string": "ONE" },
                        { "old_string": "two", "new_string": "TWO" }
                    ],
                }),
                &ctx,
            )
            .await;

        assert!(
            !result.is_error,
            "unexpected multiedit error: {}",
            result.output
        );
        let updated = fs::read_to_string(&file_path)
            .await
            .expect("updated file should read");
        assert_eq!(updated, "ONE\nTWO\n");
    }
}
