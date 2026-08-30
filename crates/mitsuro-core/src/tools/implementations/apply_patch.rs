//! Apply Patch tool - Multi-file patch application with fuzzy line seeking
//!
//! Supports the established patch format:
//! ```text
//! *** Begin Patch
//! *** Update File: path/to/file.rs
//! @@optional context hint
//!  context line (unchanged)
//! -old line to remove
//! +new line to add
//!  more context
//! *** Add File: new/file.rs
//! +first line
//! +second line
//! *** Delete File: obsolete/file.rs
//! *** End Patch
//! ```

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::tools::matching;
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use super::mutation_diagnostics::collect_mutation_warnings;

pub struct ApplyPatchTool;

#[derive(Deserialize)]
struct Params {
    patch: String,
}

#[derive(Debug)]
enum PatchOp {
    Update { path: String, chunks: Vec<Chunk> },
    Add { path: String, content: String },
    Delete { path: String },
}

#[derive(Debug)]
struct Chunk {
    context: Vec<ChunkLine>,
}

#[derive(Debug)]
enum ChunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a multi-file patch. Supports Update/Add/Delete operations with fuzzy line matching. Standard '*** Begin Patch' and '*** End Patch' envelopes are preferred; duplicated trailing delimiter stars are normalized for provider compatibility. Empty or no-op updates are rejected."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use for coordinated multi-file changes when you have a complete patch.

Wrap content in exact *** Begin Patch / *** End Patch lines with no trailing characters. Operations: *** Update File, *** Add File, *** Delete File. Read pre-existing Update/Delete targets first; files created or changed by file tools this run need no re-read. Use -, +, and space-prefixed lines for removals, additions, and context. Every Update File operation must add or remove at least one line.

Prefer edit/multiedit for targeted 1-2 file changes."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "The patch content in the standard format (*** Begin Patch ... *** End Patch)"
                }
            },
            "required": ["patch"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let ops = match parse_patch(&params.patch) {
            Ok(ops) => ops,
            Err(error) => return ToolResult::error_with_code("invalid_patch", error),
        };

        let mut files_modified = Vec::new();
        let mut files_created = Vec::new();
        let mut files_deleted = Vec::new();
        let mut errors = Vec::new();

        for op in &ops {
            match op {
                PatchOp::Update { path, chunks } => match apply_update(path, chunks, ctx).await {
                    Ok(_) => files_modified.push(path.clone()),
                    Err(e) => errors.push(format!("{}: {}", path, e)),
                },
                PatchOp::Add { path, content } => match apply_add(path, content, ctx).await {
                    Ok(_) => files_created.push(path.clone()),
                    Err(e) => errors.push(format!("{}: {}", path, e)),
                },
                PatchOp::Delete { path } => match apply_delete(path, ctx).await {
                    Ok(_) => files_deleted.push(path.clone()),
                    Err(e) => errors.push(format!("{}: {}", path, e)),
                },
            }
        }

        if !errors.is_empty() {
            return ToolResult::error_with_details(
                "patch_partial_failure",
                "Patch partially failed",
                Some(json!({
                    "files_modified": files_modified,
                    "files_created": files_created,
                    "files_deleted": files_deleted,
                    "errors": errors,
                })),
                None,
            )
            .with_changed(
                !files_modified.is_empty()
                    || !files_created.is_empty()
                    || !files_deleted.is_empty(),
            );
        }

        let msg = format!(
            "Applied patch: {} modified, {} created, {} deleted",
            files_modified.len(),
            files_created.len(),
            files_deleted.len()
        );

        let diagnostic_paths = files_modified
            .iter()
            .chain(files_created.iter())
            .filter_map(|path| ctx.sandboxed_resolve(path).ok())
            .collect::<Vec<_>>();
        let progress_paths = files_modified
            .iter()
            .chain(files_created.iter())
            .chain(files_deleted.iter())
            .filter_map(|path| ctx.sandboxed_resolve_new_path(path).ok())
            .collect::<Vec<_>>();
        let warnings = collect_mutation_warnings(&diagnostic_paths, &ctx.working_dir).await;

        ToolResult::success_data_with(
            json!({
                "message": msg,
                "files_modified": files_modified,
                "files_created": files_created,
                "files_deleted": files_deleted,
            }),
            warnings,
            None,
            None,
        )
        .with_changed(true)
        .with_progress_change_paths(&progress_paths, ctx.file_resolution_root())
    }
}

fn parse_patch(patch: &str) -> Result<Vec<PatchOp>, String> {
    let lines: Vec<&str> = patch.trim().lines().collect();
    if !lines
        .first()
        .is_some_and(|line| is_patch_envelope(line, "*** Begin Patch"))
        || !lines
            .last()
            .is_some_and(|line| is_patch_envelope(line, "*** End Patch"))
    {
        return Err(
            "Patch must start with exactly '*** Begin Patch' and end with exactly '*** End Patch'"
                .to_string(),
        );
    }

    let mut ops = Vec::new();
    let mut i = 1;

    while i < lines.len() {
        let line = lines[i].trim();

        if line == "*** End Patch" {
            break;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            let path = path.trim().to_string();
            i += 1;
            let mut chunks = Vec::new();
            let mut current_chunk = Vec::new();

            while i < lines.len() {
                let l = lines[i];
                let lt = l.trim();

                if lt.starts_with("*** ") {
                    break;
                }

                // Skip @@ context hints
                if lt.starts_with("@@") {
                    if !current_chunk.is_empty() {
                        chunks.push(Chunk {
                            context: std::mem::take(&mut current_chunk),
                        });
                    }
                    i += 1;
                    continue;
                }

                if let Some(rest) = l.strip_prefix('-') {
                    current_chunk.push(ChunkLine::Remove(rest.to_string()));
                } else if let Some(rest) = l.strip_prefix('+') {
                    current_chunk.push(ChunkLine::Add(rest.to_string()));
                } else if let Some(rest) = l.strip_prefix(' ') {
                    current_chunk.push(ChunkLine::Context(rest.to_string()));
                } else if l.is_empty() {
                    current_chunk.push(ChunkLine::Context(String::new()));
                } else {
                    // Treat unrecognized lines as context
                    current_chunk.push(ChunkLine::Context(l.to_string()));
                }

                i += 1;
            }

            if !current_chunk.is_empty() {
                chunks.push(Chunk {
                    context: current_chunk,
                });
            }

            let has_change = chunks.iter().any(|chunk| {
                chunk
                    .context
                    .iter()
                    .any(|line| matches!(line, ChunkLine::Remove(_) | ChunkLine::Add(_)))
            });
            if !has_change {
                return Err(format!(
                    "Update File operation for '{path}' contains no added or removed lines"
                ));
            }
            ops.push(PatchOp::Update { path, chunks });
        } else if let Some(path) = line.strip_prefix("*** Add File: ") {
            let path = path.trim().to_string();
            i += 1;
            let mut content_lines = Vec::new();

            while i < lines.len() {
                let l = lines[i];
                if l.trim().starts_with("*** ") {
                    break;
                }
                if let Some(rest) = l.strip_prefix('+') {
                    content_lines.push(rest.to_string());
                } else {
                    content_lines.push(l.to_string());
                }
                i += 1;
            }

            ops.push(PatchOp::Add {
                path,
                content: content_lines.join("\n"),
            });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            let path = path.trim().to_string();
            i += 1;
            ops.push(PatchOp::Delete { path });
        } else {
            i += 1;
        }
    }

    if ops.is_empty() {
        return Err("No Add, Update, or Delete operations found in patch".to_string());
    }

    Ok(ops)
}

fn is_patch_envelope(line: &str, expected: &str) -> bool {
    let line = line.trim();
    line == expected
        || line
            .strip_suffix(" ***")
            .is_some_and(|line| line == expected)
}

async fn apply_update(path: &str, chunks: &[Chunk], ctx: &ToolContext) -> Result<(), String> {
    let resolved = ctx
        .sandboxed_resolve(path)
        .map_err(|e| format!("Path error: {}", e))?;
    let resolved = ctx
        .require_file_observation(&resolved)
        .map_err(|e| format!("Read-before-patch error: {}", e))?;

    let content = fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("Failed to read: {}", e))?;

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    // Collect all replacements first, then apply in reverse order
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();

    for chunk in chunks {
        // Extract context/remove lines for seeking and add lines for replacement
        let mut seek_pattern: Vec<String> = Vec::new();
        let mut new_lines: Vec<String> = Vec::new();
        for cl in &chunk.context {
            match cl {
                ChunkLine::Context(s) => {
                    seek_pattern.push(s.clone());
                    new_lines.push(s.clone());
                }
                ChunkLine::Remove(s) => {
                    seek_pattern.push(s.clone());
                }
                ChunkLine::Add(s) => {
                    new_lines.push(s.clone());
                }
            }
        }

        if seek_pattern.is_empty() {
            continue;
        }

        // Use seek_sequence to find where this chunk applies
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let pattern_refs: Vec<&str> = seek_pattern.iter().map(|s| s.as_str()).collect();
        let is_eof = chunk
            .context
            .last()
            .is_some_and(|cl| matches!(cl, ChunkLine::Context(_)));

        let start =
            matching::seek_sequence(&line_refs, &pattern_refs, 0, is_eof).ok_or_else(|| {
                format!(
                    "Could not find matching location for chunk starting with: {:?}",
                    seek_pattern.first().unwrap_or(&String::new())
                )
            })?;

        let end = start + seek_pattern.len();
        replacements.push((start, end, new_lines));
    }

    // Apply replacements in reverse order to preserve indices
    replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.0));

    for (start, end, new_lines) in replacements {
        let end = end.min(lines.len());
        lines.splice(start..end, new_lines);
    }

    let new_content = lines.join("\n");
    // Preserve trailing newline if original had one
    let final_content = if content.ends_with('\n') && !new_content.ends_with('\n') {
        format!("{}\n", new_content)
    } else {
        new_content
    };

    if final_content == content {
        return Err("Patch produced no file changes".to_string());
    }

    fs::write(&resolved, &final_content)
        .await
        .map_err(|e| format!("Failed to write: {}", e))?;
    ctx.record_file_observation(resolved);

    Ok(())
}

async fn apply_add(path: &str, content: &str, ctx: &ToolContext) -> Result<(), String> {
    let resolved = ctx
        .sandboxed_resolve_new_path(path)
        .map_err(|e| format!("Path error: {}", e))?;

    if resolved.exists() {
        return Err(
            "Add File target already exists; read it and use Update File to modify it".to_string(),
        );
    }

    // Create parent directories
    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {}", e))?;
    }

    fs::write(&resolved, content)
        .await
        .map_err(|e| format!("Failed to write new file: {}", e))?;
    if let Err(error) = ctx.record_file_mutation(&resolved) {
        tracing::warn!(
            path = %resolved.display(),
            error,
            "Added file but could not record its observation"
        );
    }

    Ok(())
}

async fn apply_delete(path: &str, ctx: &ToolContext) -> Result<(), String> {
    let resolved = ctx
        .sandboxed_resolve(path)
        .map_err(|e| format!("Path error: {}", e))?;
    let resolved = ctx
        .require_file_observation(&resolved)
        .map_err(|e| format!("Read-before-patch error: {}", e))?;

    fs::remove_file(&resolved)
        .await
        .map_err(|e| format!("Failed to delete: {}", e))?;
    ctx.forget_file_observation(&resolved);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_patch(path: &std::path::Path) -> String {
        format!(
            "*** Begin Patch\n*** Update File: {}\n-old\n+new\n*** End Patch",
            path.display()
        )
    }

    fn delete_patch(path: &std::path::Path) -> String {
        format!(
            "*** Begin Patch\n*** Delete File: {}\n*** End Patch",
            path.display()
        )
    }

    fn add_patch(path: &std::path::Path) -> String {
        format!(
            "*** Begin Patch\n*** Add File: {}\n+created\n*** End Patch",
            path.display()
        )
    }

    #[tokio::test]
    async fn update_file_requires_prior_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
        fs::write(&file_path, "old\n")
            .await
            .expect("test file should write");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = ApplyPatchTool
            .execute(json!({ "patch": update_patch(&file_path) }), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("Read-before-patch error"));
        let content = fs::read_to_string(&file_path)
            .await
            .expect("file should remain readable");
        assert_eq!(content, "old\n");
    }

    #[tokio::test]
    async fn update_file_succeeds_after_prior_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
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

        let result = ApplyPatchTool
            .execute(json!({ "patch": update_patch(&file_path) }), &ctx)
            .await;

        assert!(
            !result.is_error,
            "unexpected patch error: {}",
            result.output
        );
        let content = fs::read_to_string(&file_path)
            .await
            .expect("updated file should read");
        assert_eq!(content, "new\n");
    }

    #[tokio::test]
    async fn add_then_update_needs_no_redundant_read() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("sample.txt");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let added = ApplyPatchTool
            .execute(json!({ "patch": add_patch(&file_path) }), &ctx)
            .await;
        assert!(!added.is_error, "unexpected add error: {}", added.output);

        let updated = ApplyPatchTool
            .execute(
                json!({
                    "patch": format!(
                        "*** Begin Patch\n*** Update File: {}\n-created\n+updated\n*** End Patch",
                        file_path.display()
                    )
                }),
                &ctx,
            )
            .await;

        assert!(
            !updated.is_error,
            "unexpected update error: {}",
            updated.output
        );
        assert_eq!(
            fs::read_to_string(&file_path)
                .await
                .expect("updated file should read"),
            "updated"
        );
    }

    #[tokio::test]
    async fn delete_file_requires_prior_observation() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("delete.txt");
        fs::write(&file_path, "remove me\n")
            .await
            .expect("test file should write");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = ApplyPatchTool
            .execute(json!({ "patch": delete_patch(&file_path) }), &ctx)
            .await;

        assert!(result.is_error);
        assert!(file_path.exists());
    }

    #[tokio::test]
    async fn delete_invalidates_observation_for_a_recreated_path() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("delete.txt");
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
        ctx.record_file_observation(canonical.clone());

        let deleted = ApplyPatchTool
            .execute(json!({ "patch": delete_patch(&file_path) }), &ctx)
            .await;
        assert!(
            !deleted.is_error,
            "unexpected delete error: {}",
            deleted.output
        );
        assert!(!ctx.has_file_observation(&canonical));

        fs::write(&file_path, "old\n")
            .await
            .expect("external recreation should write");
        let update = ApplyPatchTool
            .execute(json!({ "patch": update_patch(&file_path) }), &ctx)
            .await;

        assert!(update.is_error);
        assert!(update.output.contains("Read-before-patch error"));
    }

    #[tokio::test]
    async fn add_file_rejects_existing_target_without_overwrite() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should create");
        let file_path = temp_dir.path().join("existing.txt");
        fs::write(&file_path, "existing\n")
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

        let result = ApplyPatchTool
            .execute(json!({ "patch": add_patch(&file_path) }), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("Add File target already exists"));
        let content = fs::read_to_string(&file_path)
            .await
            .expect("existing file should read");
        assert_eq!(content, "existing\n");
    }

    #[test]
    fn parser_normalizes_duplicated_envelope_markers() {
        let provider_variant =
            "*** Begin Patch ***\n*** Add File: sample.txt\n+hello\n*** End Patch ***";
        assert!(parse_patch(provider_variant).is_ok());
    }

    #[test]
    fn parser_rejects_unrelated_envelope_text() {
        let malformed = "BEGIN PATCH\n*** Add File: sample.txt\n+hello\nEND PATCH";
        assert!(parse_patch(malformed).is_err());
    }

    #[test]
    fn parser_rejects_context_only_update_as_a_false_success() {
        let no_op = "*** Begin Patch\n*** Update File: sample.txt\n unchanged\n*** End Patch";
        assert!(parse_patch(no_op).is_err());
    }

    #[test]
    fn parser_preserves_every_line_in_a_multiline_update() {
        let patch = "*** Begin Patch\n*** Update File: sample.txt\n-old one\n-old two\n+new one\n+new two\n*** End Patch";
        let ops = parse_patch(patch).expect("valid patch should parse");
        let PatchOp::Update { chunks, .. } = &ops[0] else {
            panic!("expected update operation");
        };
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].context.len(), 4);
    }
}
