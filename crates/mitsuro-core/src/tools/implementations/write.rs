//! Write tool - Create or overwrite files with diff output

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use similar::TextDiff;
use tokio::fs;
use tracing::info;

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use super::mutation_diagnostics::collect_mutation_warnings;

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

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use Write for new files; prefer Edit for existing files because Write replaces the whole file.

Read pre-existing files before overwriting; files created or changed by file tools this run need no re-read. Parent directories are created automatically. Max content size is 10MB.

Don't create documentation files unless explicitly requested."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to write"
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

        let path = if path.is_file() {
            match ctx.require_file_observation(&path) {
                Ok(path) => path,
                Err(e) => return ToolResult::error_with_code("read_required", e),
            }
        } else {
            path
        };

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
                let mut warnings =
                    collect_mutation_warnings(std::slice::from_ref(&path), &ctx.working_dir).await;
                if let Err(error) = ctx.record_file_mutation(&path) {
                    warnings.push(format!(
                        "File was written but its observation could not be recorded: {error}"
                    ));
                }
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

                let changed = old_content.as_deref() != Some(params.content.as_str());
                ToolResult::success_data_with(data, warnings, diff, None)
                    .with_changed(changed)
                    .with_progress_change_paths(
                        std::slice::from_ref(&path),
                        ctx.sandbox_root.as_deref().unwrap_or(&ctx.working_dir),
                    )
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
    use crate::tools::implementations::EditTool;

    #[tokio::test]
    async fn write_creates_new_file_without_prior_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("new.txt");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = WriteTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "content": "created\n",
                }),
                &ctx,
            )
            .await;

        assert!(
            !result.is_error,
            "unexpected write error: {}",
            result.output
        );
        let content = fs::read_to_string(&file_path)
            .await
            .expect("created file should read");
        assert_eq!(content, "created\n");
        let canonical = file_path
            .canonicalize()
            .expect("created file should canonicalize");
        assert!(ctx.has_file_observation(&canonical));
    }

    #[tokio::test]
    async fn write_can_overwrite_a_file_it_created_without_redundant_read() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("new.txt");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let created = WriteTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "content": "first\n",
                }),
                &ctx,
            )
            .await;
        assert!(
            !created.is_error,
            "unexpected create error: {}",
            created.output
        );

        let overwritten = WriteTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "content": "second\n",
                }),
                &ctx,
            )
            .await;

        assert!(
            !overwritten.is_error,
            "unexpected overwrite error: {}",
            overwritten.output
        );
        let content = fs::read_to_string(&file_path)
            .await
            .expect("overwritten file should read");
        assert_eq!(content, "second\n");
    }

    #[tokio::test]
    async fn edit_can_change_a_file_created_by_write_without_redundant_read() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("new.txt");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let created = WriteTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "content": "before\n",
                }),
                &ctx,
            )
            .await;
        assert!(
            !created.is_error,
            "unexpected create error: {}",
            created.output
        );

        let edited = EditTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "old_string": "before",
                    "new_string": "after",
                }),
                &ctx,
            )
            .await;

        assert!(!edited.is_error, "unexpected edit error: {}", edited.output);
        assert_eq!(
            fs::read_to_string(&file_path)
                .await
                .expect("edited file should read"),
            "after\n"
        );
    }

    #[tokio::test]
    async fn write_overwrite_requires_prior_file_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("existing.txt");
        fs::write(&file_path, "old\n")
            .await
            .expect("test file should write");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = WriteTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "content": "new\n",
                }),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["error"]["code"], "read_required");
        let content = fs::read_to_string(&file_path)
            .await
            .expect("existing file should read");
        assert_eq!(content, "old\n");
    }

    #[tokio::test]
    async fn write_overwrite_succeeds_after_prior_file_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("existing.txt");
        fs::write(&file_path, "old\n")
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

        let result = WriteTool
            .execute(
                json!({
                    "file_path": file_path.display().to_string(),
                    "content": "new\n",
                }),
                &ctx,
            )
            .await;

        assert!(
            !result.is_error,
            "unexpected write error: {}",
            result.output
        );
        let content = fs::read_to_string(&file_path)
            .await
            .expect("updated file should read");
        assert_eq!(content, "new\n");
    }
}
