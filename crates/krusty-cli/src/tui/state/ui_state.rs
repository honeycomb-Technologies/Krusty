//! Block UI State - ID-based state management for blocks
//!
//! Instead of storing UI state (collapsed, scroll) in block structs,
//! we store it in a HashMap keyed by stable IDs (tool_use_id or content hash).
//! This decouples UI state from block reconstruction.

use std::collections::HashMap;

/// UI state for all blocks, keyed by stable ID
#[derive(Debug, Default)]
pub struct BlockUiStates {
    /// Collapsed state per block (default: true for thinking, false for tools)
    collapsed: HashMap<String, bool>,
    /// Scroll offset per block
    scroll_offset: HashMap<String, u16>,
}

impl BlockUiStates {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set collapsed state
    pub fn set_collapsed(&mut self, id: &str, collapsed: bool) {
        self.collapsed.insert(id.to_string(), collapsed);
    }

    /// Clear all state (for new session)
    pub fn clear(&mut self) {
        self.collapsed.clear();
        self.scroll_offset.clear();
    }

    /// Import state from database records
    pub fn import(&mut self, states: Vec<(String, bool, u16)>) {
        for (id, collapsed, scroll) in states {
            self.collapsed.insert(id.clone(), collapsed);
            self.scroll_offset.insert(id, scroll);
        }
    }

    /// Export state for database storage
    pub fn export(&self) -> Vec<(String, bool, u16)> {
        self.collapsed
            .keys()
            .chain(self.scroll_offset.keys())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .map(|id| {
                (
                    id.clone(),
                    self.collapsed.get(id).copied().unwrap_or(true),
                    self.scroll_offset.get(id).copied().unwrap_or(0),
                )
            })
            .collect()
    }
}

/// Cached tool result data for rendering
#[derive(Debug, Clone)]
pub struct ToolResultData {
    /// The actual output content
    pub output: String,
    /// Exit code (for bash)
    pub exit_code: i32,
    /// Whether this was an error
    pub is_error: bool,
}

/// Cache of tool results, keyed by tool_use_id
#[derive(Debug, Default)]
pub struct ToolResultCache {
    results: HashMap<String, ToolResultData>,
}

impl ToolResultCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cached result
    pub fn get(&self, tool_use_id: &str) -> Option<&ToolResultData> {
        self.results.get(tool_use_id)
    }

    /// Cache a tool result from raw output
    pub fn insert_raw(
        &mut self,
        tool_use_id: String,
        tool_name: &str,
        output: &str,
        is_error: bool,
    ) {
        // Parse JSON output for bash to extract actual output and exit code
        let (actual_output, exit_code) = if tool_name == "bash" {
            Self::parse_bash_output(output)
        } else {
            (output.to_string(), 0)
        };

        self.results.insert(
            tool_use_id,
            ToolResultData {
                output: actual_output,
                exit_code,
                is_error,
            },
        );
    }

    /// Parse bash JSON output from either legacy or structured tool envelopes.
    fn parse_bash_output(output: &str) -> (String, i32) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
            let actual_output = json
                .get("data")
                .and_then(|v| v.get("output"))
                .and_then(|v| v.as_str())
                .or_else(|| json.get("output").and_then(|v| v.as_str()))
                .or_else(|| {
                    json.get("error")
                        .and_then(|v| v.get("message"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or(output)
                .to_string();
            let exit_code = json
                .get("metadata")
                .and_then(|v| v.get("exit_code"))
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse::<i64>().ok())
                .or_else(|| {
                    json.get("metadata")
                        .and_then(|v| v.get("exit_code"))
                        .and_then(|v| v.as_i64())
                })
                .or_else(|| json.get("exitCode").and_then(|v| v.as_i64()))
                .unwrap_or(0) as i32;
            (actual_output, exit_code)
        } else {
            (output.to_string(), 0)
        }
    }

    /// Clear all cached results (for new session)
    pub fn clear(&mut self) {
        self.results.clear();
    }
}

/// Generate a stable ID from content (for thinking blocks without signatures)
pub fn hash_content(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("content_{:016x}", hasher.finish())
}
