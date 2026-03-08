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
        "Apply a multi-file patch. Supports Update/Add/Delete operations with fuzzy line matching. Use the standard patch format with '*** Begin Patch' and '*** End Patch' markers."
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

        let ops = parse_patch(&params.patch);

        if ops.is_empty() {
            return ToolResult::error_with_code(
                "invalid_patch",
                "No operations found in patch. Ensure it uses '*** Begin Patch' / '*** End Patch' format."
                    .to_string(),
            );
        }

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
            );
        }

        let msg = format!(
            "Applied patch: {} modified, {} created, {} deleted",
            files_modified.len(),
            files_created.len(),
            files_deleted.len()
        );

        ToolResult::success_data(json!({
            "message": msg,
            "files_modified": files_modified,
            "files_created": files_created,
            "files_deleted": files_deleted,
        }))
    }
}

fn parse_patch(patch: &str) -> Vec<PatchOp> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut ops = Vec::new();
    let mut i = 0;

    // Skip to "*** Begin Patch" if present
    while i < lines.len() {
        if lines[i].trim() == "*** Begin Patch" {
            i += 1;
            break;
        }
        i += 1;
    }

    // If no "Begin Patch" marker, start from beginning
    if i >= lines.len() {
        i = 0;
    }

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

    ops
}

async fn apply_update(path: &str, chunks: &[Chunk], ctx: &ToolContext) -> Result<(), String> {
    let resolved = ctx
        .sandboxed_resolve(path)
        .map_err(|e| format!("Path error: {}", e))?;

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
    replacements.sort_by(|a, b| b.0.cmp(&a.0));

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

    fs::write(&resolved, &final_content)
        .await
        .map_err(|e| format!("Failed to write: {}", e))?;

    Ok(())
}

async fn apply_add(path: &str, content: &str, ctx: &ToolContext) -> Result<(), String> {
    let resolved = ctx
        .sandboxed_resolve_new_path(path)
        .map_err(|e| format!("Path error: {}", e))?;

    // Create parent directories
    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {}", e))?;
    }

    fs::write(&resolved, content)
        .await
        .map_err(|e| format!("Failed to write new file: {}", e))?;

    Ok(())
}

async fn apply_delete(path: &str, ctx: &ToolContext) -> Result<(), String> {
    let resolved = ctx
        .sandboxed_resolve(path)
        .map_err(|e| format!("Path error: {}", e))?;

    fs::remove_file(&resolved)
        .await
        .map_err(|e| format!("Failed to delete: {}", e))?;

    Ok(())
}
