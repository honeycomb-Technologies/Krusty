//! Sub-agent types and data structures.
//!
//! This keeps the `subagent` surface stable while separating:
//! - delegated API/retry errors
//! - task/progress models
//! - explore-report parsing and synthesis
//! - result/evidence shaping

mod api_error;
mod models;
mod report;
mod result;

pub use self::api_error::SubAgentApiError;
pub use self::models::{AgentProgress, AgentProgressStatus, SubAgentTask};
pub(crate) use self::report::{
    parse_explore_report, render_explore_report, summary_looks_non_substantive,
    synthesize_explore_report, synthesize_explore_report_from_paths,
};
pub use self::result::{DelegatedProcessArtifact, SubAgentResult};

/// Parsed tool call from a sub-agent API response.
#[derive(Debug)]
pub(crate) struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}
