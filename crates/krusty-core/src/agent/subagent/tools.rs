//! Sub-agent tool implementations
//!
//! Read-only tools for explorers, read-write tools for builders.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent::build_context::{BuilderInterface, SharedBuildContext};
use crate::agent::cache::SharedExploreCache;
use crate::ai::types::AiTool;
use crate::tools::implementations::{BashTool, EditTool, GlobTool, GrepTool, ReadTool, WriteTool};
use crate::tools::registry::{Tool, ToolContext, ToolResult};

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

/// Sub-agent tools - read-only access with shared cache
pub(crate) struct SubAgentTools {
    glob: GlobTool,
    grep: GrepTool,
    read: ReadTool,
    cache: Arc<SharedExploreCache>,
}

impl SubAgentTools {
    pub fn new(cache: Arc<SharedExploreCache>) -> Self {
        Self {
            glob: GlobTool,
            grep: GrepTool,
            read: ReadTool,
            cache,
        }
    }

    pub fn get_ai_tools(&self) -> Vec<AiTool> {
        vec![
            AiTool {
                name: "glob".to_string(),
                description: self.glob.description().to_string(),
                input_schema: self.glob.parameters_schema(),
            },
            AiTool {
                name: "grep".to_string(),
                description: self.grep.description().to_string(),
                input_schema: self.grep.parameters_schema(),
            },
            AiTool {
                name: "read".to_string(),
                description: self.read.description().to_string(),
                input_schema: self.read.parameters_schema(),
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
            "glob" => {
                // Check cache for glob results
                let pattern = params
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let base_dir = ctx.working_dir.clone();

                if let Some(cached_paths) = self.cache.get_glob(&pattern, &base_dir) {
                    // Return cached result formatted as the tool would
                    let output = cached_paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    return Some(ToolResult {
                        output: if output.is_empty() {
                            "No matches found".to_string()
                        } else {
                            output
                        },
                        is_error: false,
                    });
                }

                // Execute and cache
                let result = self.glob.execute(params, ctx).await;
                if !result.is_error {
                    // Parse paths from output and cache
                    let paths: Vec<PathBuf> = result
                        .output
                        .lines()
                        .filter(|l| !l.is_empty() && *l != "No matches found")
                        .map(PathBuf::from)
                        .collect();
                    self.cache.put_glob(pattern, base_dir, paths);
                }
                Some(result)
            }
            "grep" => {
                // Grep caching is trickier due to many parameters
                // For now, just execute without caching (grep results vary by flags)
                Some(self.grep.execute(params, ctx).await)
            }
            "read" => {
                // Check cache for file content
                let file_path = params
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from);

                if let Some(path) = file_path {
                    // Only cache full file reads (no offset/limit)
                    let has_offset = params.get("offset").is_some();
                    let has_limit = params.get("limit").is_some();

                    if !has_offset && !has_limit {
                        if let Some(cached) = self.cache.get_file(&path) {
                            // Format like the read tool does (with line numbers)
                            let output = cached
                                .content
                                .lines()
                                .enumerate()
                                .map(|(i, line)| format!("{:>6}→{}", i + 1, line))
                                .collect::<Vec<_>>()
                                .join("\n");
                            return Some(ToolResult {
                                output,
                                is_error: false,
                            });
                        }
                    }

                    // Execute and cache (only full reads)
                    let result = self.read.execute(params, ctx).await;
                    if !result.is_error && !has_offset && !has_limit {
                        // Extract raw content (strip line numbers)
                        let raw_content: String = result
                            .output
                            .lines()
                            .map(|line| {
                                // Line format: "    123→content" - find the → and take after it
                                if let Some(pos) = line.find('→') {
                                    &line[pos + '→'.len_utf8()..]
                                } else {
                                    line
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        self.cache.put_file(path, raw_content);
                    }
                    Some(result)
                } else {
                    Some(self.read.execute(params, ctx).await)
                }
            }
            _ => None,
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
            },
            AiTool {
                name: "grep".to_string(),
                description: self.grep.description().to_string(),
                input_schema: self.grep.parameters_schema(),
            },
            AiTool {
                name: "read".to_string(),
                description: self.read.description().to_string(),
                input_schema: self.read.parameters_schema(),
            },
            AiTool {
                name: "write".to_string(),
                description: self.write.description().to_string(),
                input_schema: self.write.parameters_schema(),
            },
            AiTool {
                name: "edit".to_string(),
                description: self.edit.description().to_string(),
                input_schema: self.edit.parameters_schema(),
            },
            AiTool {
                name: "bash".to_string(),
                description: self.bash.description().to_string(),
                input_schema: self.bash.parameters_schema(),
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
                        return Some(ToolResult {
                            output: "Missing file_path parameter".to_string(),
                            is_error: true,
                        })
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
                        return Some(ToolResult {
                            output: format!("Cannot write: {}", e),
                            is_error: true,
                        })
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
                        return Some(ToolResult {
                            output: "Missing file_path parameter".to_string(),
                            is_error: true,
                        })
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
                        return Some(ToolResult {
                            output: format!("Cannot edit: {}", e),
                            is_error: true,
                        })
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
                let file_path = params
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_default();
                let exports: Vec<String> = params
                    .get("exports")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let description = params
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let interface = BuilderInterface {
                    builder_id: self.builder_id.clone(),
                    file_path: file_path.clone(),
                    exports: exports.clone(),
                    description,
                };

                self.context.register_interface(interface);

                Some(ToolResult {
                    output: format!(
                        "Registered interface: {} exports from {}",
                        exports.len(),
                        file_path.display()
                    ),
                    is_error: false,
                })
            }
            _ => None,
        }
    }
}
