//! Glob tool - Find files by pattern

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use globset::GlobBuilder;
use ignore::{DirEntry, WalkBuilder};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct GlobTool;

const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_RESULTS: usize = 500;
const MAX_VISITED_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 32;
const SEARCH_BUDGET: Duration = Duration::from_secs(5);

const GENERATED_DIRECTORY_NAMES: &[&str] = &[
    ".cache",
    ".git",
    ".hg",
    ".next",
    ".npm",
    ".pnpm-store",
    ".rustup",
    ".svn",
    ".turbo",
    "build",
    "dist",
    "node_modules",
    "target",
];

#[derive(Deserialize)]
struct Params {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug)]
struct SearchOutcome {
    matches: Vec<String>,
    matched_count: usize,
    visited_count: usize,
    skipped_errors: usize,
    stopped_reason: Option<&'static str>,
}

fn should_visit(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return true;
    }

    let name = entry.file_name().to_string_lossy();
    !GENERATED_DIRECTORY_NAMES.contains(&name.as_ref())
}

fn search_files(
    base_path: &Path,
    pattern: &str,
    max_results: usize,
    max_visited_entries: usize,
    search_budget: Duration,
) -> Result<SearchOutcome, String> {
    let matcher = GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|error| format!("Invalid pattern: {error}"))?
        .compile_matcher();

    let mut builder = WalkBuilder::new(base_path);
    builder
        .standard_filters(true)
        .hidden(false)
        .follow_links(false)
        .max_depth(Some(MAX_DEPTH))
        .filter_entry(should_visit);

    let started_at = Instant::now();
    let mut newest = BinaryHeap::<Reverse<(SystemTime, PathBuf)>>::new();
    let mut matched_count = 0usize;
    let mut visited_count = 0usize;
    let mut skipped_errors = 0usize;
    let mut stopped_reason = None;

    for entry in builder.build() {
        if started_at.elapsed() >= search_budget {
            stopped_reason = Some("time_budget");
            break;
        }
        if visited_count >= max_visited_entries {
            stopped_reason = Some("entry_budget");
            break;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped_errors += 1;
                continue;
            }
        };
        visited_count += 1;

        if entry.depth() == 0 {
            continue;
        }
        let relative_path = entry.path().strip_prefix(base_path).unwrap_or(entry.path());
        if !matcher.is_match(relative_path) {
            continue;
        }

        matched_count += 1;
        let modified_at = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        newest.push(Reverse((modified_at, entry.into_path())));
        if newest.len() > max_results {
            newest.pop();
        }
    }

    let mut retained = newest
        .into_iter()
        .map(|Reverse(entry)| entry)
        .collect::<Vec<_>>();
    retained.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    Ok(SearchOutcome {
        matches: retained
            .into_iter()
            .map(|(_, path)| path.display().to_string())
            .collect(),
        matched_count,
        visited_count,
        skipped_errors,
        stopped_reason,
    })
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files by name/path instead of Bash find/ls (for example '**/*.rs' or 'src/**/*.ts'). Searches are ignore-aware and bounded; use a specific base path and pattern for large workspaces."
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
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "description": "Maximum paths to return (default 100, maximum 500)"
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
        let max_results = params
            .max_results
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .clamp(1, MAX_RESULTS);
        let search_base = base_path.clone();
        let pattern = params.pattern.clone();
        let outcome = match tokio::task::spawn_blocking(move || {
            search_files(
                &search_base,
                &pattern,
                max_results,
                MAX_VISITED_ENTRIES,
                SEARCH_BUDGET,
            )
        })
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => return ToolResult::error(error),
            Err(error) => return ToolResult::error(format!("Glob worker failed: {error}")),
        };

        let truncated = outcome.stopped_reason.is_some() || outcome.matched_count > max_results;
        let warnings = outcome
            .stopped_reason
            .map(|reason| {
                vec![format!(
                    "Search stopped at the {reason}; narrow the base path or pattern for complete results."
                )]
            })
            .unwrap_or_default();

        ToolResult::success_data_with(
            json!({
                "matches": outcome.matches,
                "count": outcome.matched_count,
                "search_path": base_path.display().to_string(),
                "truncated": truncated
            }),
            warnings,
            None,
            Some(json!({
                "visited_entries": outcome.visited_count,
                "skipped_errors": outcome.skipped_errors,
                "stopped_reason": outcome.stopped_reason
            })),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("mitsuro-glob-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create glob test directory");
        path
    }

    #[test]
    fn brace_patterns_match_multiple_file_names() {
        let root = temp_dir("braces");
        fs::write(root.join("Cargo.toml"), "").unwrap();
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(root.join("README.md"), "").unwrap();

        let outcome = search_files(
            &root,
            "**/{Cargo.toml,package.json}",
            100,
            1_000,
            Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(outcome.matched_count, 2);
        assert!(outcome
            .matches
            .iter()
            .any(|path| path.ends_with("Cargo.toml")));
        assert!(outcome
            .matches
            .iter()
            .any(|path| path.ends_with("package.json")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generated_directories_are_skipped_and_results_are_bounded() {
        let root = temp_dir("bounded");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target/deep")).unwrap();
        for index in 0..20 {
            fs::write(root.join("src").join(format!("file-{index}.rs")), "").unwrap();
        }
        fs::write(root.join("target/deep/generated.rs"), "").unwrap();

        let outcome = search_files(&root, "**/*.rs", 5, 1_000, Duration::from_secs(1)).unwrap();

        assert_eq!(outcome.matched_count, 20);
        assert_eq!(outcome.matches.len(), 5);
        assert!(outcome.matches.iter().all(|path| {
            Path::new(path)
                .strip_prefix(&root)
                .expect("match should remain under the search root")
                .components()
                .all(|component| component.as_os_str() != "target")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn entry_budget_returns_a_partial_success() {
        let root = temp_dir("entry-budget");
        for index in 0..20 {
            fs::write(root.join(format!("file-{index}.txt")), "").unwrap();
        }

        let outcome = search_files(&root, "**/*.txt", 100, 5, Duration::from_secs(1)).unwrap();

        assert_eq!(outcome.stopped_reason, Some("entry_budget"));
        assert!(outcome.visited_count <= 5);
        fs::remove_dir_all(root).unwrap();
    }
}
