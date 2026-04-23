//! Sub-agent tool implementations
//!
//! Read-write tools for builder agents with file locking.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ai::types::AiTool;
use crate::tools::implementations::{BashTool, EditTool, GlobTool, GrepTool, ReadTool, WriteTool};
use crate::tools::registry::{Tool, ToolContext, ToolResult};

use super::build_context::{BuilderInterface, SharedBuildContext};

/// RAII guard for builder file locks
///
/// Automatically releases the lock when dropped, ensuring
/// locks are never leaked due to early returns or panics.
struct FileLockGuard {
    path: PathBuf,
    builder_id: String,
    context: Arc<SharedBuildContext>,
    locked: bool,
}

impl FileLockGuard {
    /// Try to acquire a file lock with exponential backoff
    async fn acquire(
        context: Arc<SharedBuildContext>,
        path: PathBuf,
        builder_id: String,
    ) -> Result<Self, String> {
        use crate::agent::constants::retry;

        let start = Instant::now();

        for (attempt, delay) in retry::DELAYS_MS.iter().enumerate() {
            match context.acquire_lock(path.clone(), builder_id.clone(), "write/edit".to_string()) {
                Ok(()) => {
                    // Record wait time if we had to wait
                    let wait_time = start.elapsed();
                    if wait_time > retry::LOG_THRESHOLD {
                        context.record_lock_wait(path.clone(), wait_time);
                    }
                    return Ok(Self {
                        path,
                        builder_id,
                        context,
                        locked: true,
                    });
                }
                Err(holder) => {
                    if attempt < retry::DELAYS_MS.len() - 1 {
                        tracing::debug!(
                            builder = %builder_id,
                            path = %path.display(),
                            holder = %holder,
                            attempt = attempt,
                            "File locked, backoff {}ms",
                            delay
                        );
                        tokio::time::sleep(Duration::from_millis(*delay)).await;
                    } else {
                        // Record the failed wait time too
                        let wait_time = start.elapsed();
                        context.record_lock_wait(path.clone(), wait_time);
                        return Err(format!(
                            "File {} locked by {} (tried {}x, waited {:.1}s)",
                            path.display(),
                            holder,
                            retry::MAX_ATTEMPTS,
                            wait_time.as_secs_f64()
                        ));
                    }
                }
            }
        }
        Err("Lock acquisition failed".to_string())
    }
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        if self.locked {
            self.context.release_lock(&self.path, &self.builder_id);
            tracing::debug!(path = %self.path.display(), "File lock released via RAII guard");
        }
    }
}

/// Builder agent tools - read/write access with shared build context
pub struct BuilderTools {
    glob: GlobTool,
    grep: GrepTool,
    read: ReadTool,
    write: WriteTool,
    edit: EditTool,
    bash: BashTool,
    context: Arc<SharedBuildContext>,
    builder_id: String,
}

impl BuilderTools {
    pub fn new(context: Arc<SharedBuildContext>, builder_id: String) -> Self {
        Self {
            glob: GlobTool,
            grep: GrepTool,
            read: ReadTool,
            write: WriteTool,
            edit: EditTool,
            bash: BashTool,
            context,
            builder_id,
        }
    }

    pub fn get_ai_tools(&self) -> Vec<AiTool> {
        vec![
            AiTool {
                name: "glob".to_string(),
                description: self.glob.description().to_string(),
                input_schema: self.glob.parameters_schema(),
                prompt: self.glob.prompt().map(|s| s.to_string()),
            },
            AiTool {
                name: "grep".to_string(),
                description: self.grep.description().to_string(),
                input_schema: self.grep.parameters_schema(),
                prompt: self.grep.prompt().map(|s| s.to_string()),
            },
            AiTool {
                name: "read".to_string(),
                description: self.read.description().to_string(),
                input_schema: self.read.parameters_schema(),
                prompt: self.read.prompt().map(|s| s.to_string()),
            },
            AiTool {
                name: "write".to_string(),
                description: self.write.description().to_string(),
                input_schema: self.write.parameters_schema(),
                prompt: self.write.prompt().map(|s| s.to_string()),
            },
            AiTool {
                name: "edit".to_string(),
                description: self.edit.description().to_string(),
                input_schema: self.edit.parameters_schema(),
                prompt: self.edit.prompt().map(|s| s.to_string()),
            },
            AiTool {
                name: "bash".to_string(),
                description: self.bash.description().to_string(),
                input_schema: self.bash.parameters_schema(),
                prompt: self.bash.prompt().map(|s| s.to_string()),
            },
            AiTool {
                name: "register_interface".to_string(),
                description: "Register your component's interface so other builders can use it. \
                             Call this after creating your module to advertise its exports."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to the file containing the interface"
                        },
                        "exports": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "List of exported function/class/type names"
                        },
                        "description": {
                            "type": "string",
                            "description": "Brief description of what this interface provides"
                        }
                    },
                    "required": ["file_path", "exports", "description"]
                }),
                prompt: None,
            },
        ]
    }

    pub async fn execute(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        match name {
            "glob" => Some(self.glob.execute(params, ctx).await),
            "grep" => Some(self.grep.execute(params, ctx).await),
            "read" => Some(self.read.execute(params, ctx).await),
            "write" => {
                // Get file path and acquire lock before writing
                let path = match params.get("file_path").and_then(|v| v.as_str()) {
                    Some(p) => PathBuf::from(p),
                    None => {
                        return Some(ToolResult::invalid_parameters(
                            "Missing file_path parameter",
                        ))
                    }
                };

                // Acquire lock with RAII guard (auto-releases on drop)
                let _guard = match FileLockGuard::acquire(
                    self.context.clone(),
                    path.clone(),
                    self.builder_id.clone(),
                )
                .await
                {
                    Ok(guard) => guard,
                    Err(e) => {
                        return Some(ToolResult::error_with_code(
                            "file_locked",
                            format!("Cannot write: {}", e),
                        ));
                    }
                };

                let result = self.write.execute(params.clone(), ctx).await;

                // Track line changes for the build context
                if !result.is_error {
                    if let Some(content) = params.get("content").and_then(|v| v.as_str()) {
                        let lines_added = content.lines().count();
                        self.context.record_line_changes(lines_added, 0);
                    }
                    self.context
                        .record_modification(path.clone(), self.builder_id.clone());
                }

                // Lock released automatically when _guard drops
                Some(result)
            }
            "edit" => {
                // Get file path and acquire lock before editing
                let path = match params.get("file_path").and_then(|v| v.as_str()) {
                    Some(p) => PathBuf::from(p),
                    None => {
                        return Some(ToolResult::invalid_parameters(
                            "Missing file_path parameter",
                        ))
                    }
                };

                // Acquire lock with RAII guard (auto-releases on drop)
                let _guard = match FileLockGuard::acquire(
                    self.context.clone(),
                    path.clone(),
                    self.builder_id.clone(),
                )
                .await
                {
                    Ok(guard) => guard,
                    Err(e) => {
                        return Some(ToolResult::error_with_code(
                            "file_locked",
                            format!("Cannot edit: {}", e),
                        ));
                    }
                };

                let result = self.edit.execute(params.clone(), ctx).await;

                // Track line changes for edits
                if !result.is_error {
                    let old_lines = params
                        .get("old_string")
                        .and_then(|v| v.as_str())
                        .map(|s| s.lines().count())
                        .unwrap_or(0);
                    let new_lines = params
                        .get("new_string")
                        .and_then(|v| v.as_str())
                        .map(|s| s.lines().count())
                        .unwrap_or(0);
                    if new_lines > old_lines {
                        self.context.record_line_changes(new_lines - old_lines, 0);
                    } else {
                        self.context.record_line_changes(0, old_lines - new_lines);
                    }
                    self.context
                        .record_modification(path.clone(), self.builder_id.clone());
                }

                // Lock released automatically when _guard drops
                Some(result)
            }
            "bash" => Some(self.bash.execute(params, ctx).await),
            "register_interface" => {
                // Register an interface for other builders to see
                let Some(file_path) = params.get("file_path").and_then(|v| v.as_str()) else {
                    return Some(ToolResult::invalid_parameters(
                        "Missing required field: file_path",
                    ));
                };
                let file_path = PathBuf::from(file_path);
                let exports: Vec<String> = params
                    .get("exports")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if exports.is_empty() {
                    return Some(ToolResult::invalid_parameters(
                        "Missing required field: exports",
                    ));
                }
                let Some(description) = params.get("description").and_then(|v| v.as_str()) else {
                    return Some(ToolResult::invalid_parameters(
                        "Missing required field: description",
                    ));
                };
                let description = description.to_string();

                let interface = BuilderInterface {
                    builder_id: self.builder_id.clone(),
                    file_path: file_path.clone(),
                    exports: exports.clone(),
                    description,
                };

                self.context.register_interface(interface);

                Some(ToolResult::success_data(json!({
                    "message": format!(
                        "Registered interface: {} exports from {}",
                        exports.len(),
                        file_path.display()
                    ),
                    "file_path": file_path.display().to_string(),
                    "exports_count": exports.len()
                })))
            }
            _ => None,
        }
    }
}
