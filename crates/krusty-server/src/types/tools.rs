use krusty_core::storage::WorkMode;
use serde::{Deserialize, Serialize};

// ============================================================================
// Tool Types
// ============================================================================

#[derive(Deserialize)]
pub struct ToolExecuteRequest {
    pub tool_name: String,
    pub params: serde_json::Value,
    /// Optional working directory override
    pub working_dir: Option<String>,
    /// Optional mode override for one-off tool execution context
    pub mode: Option<WorkMode>,
}

#[derive(Serialize)]
pub struct ToolExecuteResponse {
    pub output: String,
    pub is_error: bool,
}
