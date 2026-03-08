//! List directory tool - Breadth-first directory listing with depth/limit

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use tokio::fs;

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

const DEFAULT_DEPTH: usize = 2;
const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 10_000;

pub struct ListTool;

#[derive(Deserialize)]
struct Params {
    path: String,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

struct Entry {
    display_path: String,
    is_dir: bool,
}

#[async_trait]
impl Tool for ListTool {
    fn name(&self) -> &str {
        "list"
    }

    fn description(&self) -> &str {
        "List directory contents recursively. Shows files and subdirectories with tree structure. Use depth to control recursion (default 2) and limit to cap entries (default 200)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory path to list"
                },
                "depth": {
                    "type": "number",
                    "description": "Maximum recursion depth (default: 2)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of entries to return (default: 200)"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let max_depth = params.depth.unwrap_or(DEFAULT_DEPTH);
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);

        let path = match ctx.sandboxed_resolve(&params.path) {
            Ok(p) => p,
            Err(e) => {
                let fallback = ctx.resolve_path(&params.path);
                if !fallback.exists() {
                    return ToolResult::error(format!("Directory not found: {}", params.path));
                }
                return ToolResult::error(e);
            }
        };

        if !path.is_dir() {
            return ToolResult::error(format!("Path is not a directory: {}", path.display()));
        }

        // BFS traversal
        let mut entries: Vec<Entry> = Vec::new();
        let mut queue: VecDeque<(std::path::PathBuf, usize)> = VecDeque::new();
        queue.push_back((path.clone(), 0));

        while let Some((dir, depth)) = queue.pop_front() {
            if entries.len() >= limit {
                break;
            }

            let mut dir_entries = match fs::read_dir(&dir).await {
                Ok(rd) => rd,
                Err(_) => continue,
            };

            let mut children: Vec<(String, std::path::PathBuf, bool)> = Vec::new();

            while let Ok(Some(entry)) = dir_entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();

                // Skip hidden files/dirs
                if name.starts_with('.') {
                    continue;
                }

                let entry_path = entry.path();
                let is_dir = entry
                    .file_type()
                    .await
                    .map(|ft| ft.is_dir())
                    .unwrap_or(false);

                children.push((name, entry_path, is_dir));
            }

            // Sort case-insensitive, directories first
            children.sort_by(|a, b| {
                b.2.cmp(&a.2) // dirs first
                    .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            });

            for (_name, entry_path, is_dir) in children {
                if entries.len() >= limit {
                    break;
                }

                let relative = entry_path
                    .strip_prefix(&path)
                    .unwrap_or(&entry_path)
                    .to_string_lossy()
                    .to_string();

                let display = if is_dir {
                    format!("{}/", relative)
                } else {
                    relative
                };

                entries.push(Entry {
                    display_path: display,
                    is_dir,
                });

                if is_dir && depth < max_depth {
                    queue.push_back((entry_path, depth + 1));
                }
            }
        }

        let total = entries.len();
        let dir_count = entries.iter().filter(|e| e.is_dir).count();
        let file_count = total - dir_count;

        let listing: Vec<String> = entries.iter().map(|e| e.display_path.clone()).collect();
        let output_text = listing.join("\n");

        ToolResult::success_data(json!({
            "output": output_text,
            "total_entries": total,
            "directories": dir_count,
            "files": file_count,
            "truncated": total >= limit
        }))
    }
}
