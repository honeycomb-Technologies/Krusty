//! Glob tool - Find files by pattern

use async_trait::async_trait;
use glob::glob as glob_match;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct GlobTool;

#[derive(Deserialize)]
struct Params {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files by glob pattern (e.g., '**/*.rs', 'src/**/*.ts'). Returns up to 100 paths sorted by modification time (newest first). For large codebases, use specific patterns to narrow results."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use for file discovery by name/path instead of shell find/ls. Results are sorted newest-first and capped at 100.

Use specific subdirectory patterns in large repos. Examples: "**/*.rs", "src/**/*.ts", "*.{js,jsx}".

Use Grep for file contents and List for directory trees."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g., '**/*.rs', 'src/**/*.ts')"
                },
                "path": {
                    "type": "string",
                    "description": "Base directory to search in (default: current directory)"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        // Resolve and validate the base path under the configured filesystem access policy.
        let base_path = match &params.path {
            Some(path) => match ctx.sandboxed_resolve(path) {
                Ok(p) => p,
                Err(e) => return ToolResult::error(e),
            },
            None => ctx.working_dir.clone(),
        };
        let full_pattern = base_path.join(&params.pattern);
        let pattern_str = full_pattern.to_string_lossy();

        let entries = match glob_match(&pattern_str) {
            Ok(paths) => paths,
            Err(e) => return ToolResult::error(format!("Invalid pattern: {}", e)),
        };

        // Collect with mtime, filter to the configured access root when scoped, sort newest first, limit to 100
        let access_root = ctx.filesystem_access_root();
        let mut files: Vec<_> = entries
            .flatten()
            .filter_map(|entry| {
                // In scoped mode, deny paths that cannot be verified under the access root.
                if let Some(ref root) = access_root {
                    match entry.canonicalize() {
                        Ok(canonical) => {
                            if !canonical.starts_with(root) {
                                return None;
                            }
                        }
                        Err(_) => {
                            return None;
                        }
                    }
                }
                entry
                    .metadata()
                    .ok()
                    .map(|m| (entry, m.modified().unwrap_or(std::time::UNIX_EPOCH)))
            })
            .collect();

        files.sort_by_key(|entry| std::cmp::Reverse(entry.1));

        let matches: Vec<String> = files
            .iter()
            .take(100)
            .map(|(path, _)| path.display().to_string())
            .collect();

        ToolResult::success_data(json!({
            "matches": matches,
            "count": files.len(),
            "search_path": base_path.display().to_string()
        }))
    }
}
