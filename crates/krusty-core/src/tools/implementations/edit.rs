//! Edit tool - Edit files by replacing text with fuzzy matching cascade

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use similar::TextDiff;

use crate::tools::matching;
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use super::mutation_diagnostics::collect_mutation_warnings;

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

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Read pre-existing files first; files created or changed by file tools this run need no re-read. old_string must match exactly one location; add surrounding context when needed.

Preserve exact indentation and prefer small replacements. Use replace_all:true only for exact bulk renames."#,
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

        let path = match ctx.require_file_observation(&path) {
            Ok(path) => path,
            Err(e) => return ToolResult::error_with_code("read_required", e),
        };

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
                    ctx.record_file_observation(path.clone());
                    let warnings =
                        collect_mutation_warnings(std::slice::from_ref(&path), &ctx.working_dir)
                            .await;
                    let data = json!({
                        "message": format!("Replaced {} occurrence(s)", count),
                        "replacements": count,
                        "file_path": path.display().to_string()
                    });

                    ToolResult::success_data_with(data, warnings, Some(diff), None)
                        .with_changed(new_content != content)
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
                        params.new_string,
                        &content[m.end..]
                    );

                    let diff = generate_compact_diff(&content, &new_content, &path);

                    match fs::write(&path, &new_content).await {
                        Ok(_) => {
                            ctx.record_file_observation(path.clone());
                            let mut msg = "Replaced 1 occurrence".to_string();
                            let mut warnings = collect_mutation_warnings(
                                std::slice::from_ref(&path),
                                &ctx.working_dir,
                            )
                            .await;
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
                                .with_changed(new_content != content)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn edit_requires_prior_file_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
        fs::write(&file_path, "before\n")
            .await
            .expect("test file should write");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = EditTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "old_string": "before",
                    "new_string": "after",
                }),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["error"]["code"], "read_required");
    }

    #[tokio::test]
    async fn edit_succeeds_after_prior_file_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
        fs::write(&file_path, "before\n")
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

        let result = EditTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "old_string": "before",
                    "new_string": "after",
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error, "unexpected edit error: {}", result.output);
        let updated = fs::read_to_string(&file_path)
            .await
            .expect("updated file should read");
        assert_eq!(updated, "after\n");
    }
}
