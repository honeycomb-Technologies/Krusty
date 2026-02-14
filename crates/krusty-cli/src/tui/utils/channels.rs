//! Async Channels
//!
//! Groups all async channel receivers used by the App for background tasks.

use tokio::sync::{mpsc, oneshot};

use crate::agent::subagent::AgentProgress;
use crate::agent::SummarizationResult;
use crate::ai::models::ModelMetadata;
use crate::ai::types::Content;
use crate::tools::ToolOutputChunk;

/// AI-generated title update
pub struct TitleUpdate {
    pub session_id: String,
    pub title: String,
}

/// Result from /init codebase exploration
pub struct InitExplorationResult {
    /// Project architecture analysis
    pub architecture: String,
    /// Coding conventions found
    pub conventions: String,
    /// Key files and their purposes
    pub key_files: String,
    /// Build system analysis
    pub build_system: String,
    /// Whether exploration succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
}

/// AI-generated summarization result for pinch
pub struct SummarizationUpdate {
    pub result: Result<SummarizationResult, String>,
}

/// MCP server status update from background tasks
pub struct McpStatusUpdate {
    pub success: bool,
    pub message: String,
}

/// OAuth authentication status update from background tasks
pub struct OAuthStatusUpdate {
    /// Provider being authenticated
    pub provider: krusty_core::ai::providers::ProviderId,
    /// Whether authentication succeeded
    pub success: bool,
    /// Status message or error
    pub message: String,
    /// Device code info (for device flow)
    pub device_code: Option<DeviceCodeInfo>,
    /// OAuth token data (on success)
    pub token: Option<krusty_core::auth::OAuthTokenData>,
}

/// Device code information for OAuth device flow
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
}

/// Container for all async channel receivers
#[derive(Default)]
pub struct AsyncChannels {
    /// MCP status updates from background connection tasks
    pub mcp_status: Option<mpsc::UnboundedReceiver<McpStatusUpdate>>,
    /// Streaming bash output receiver
    pub bash_output: Option<mpsc::UnboundedReceiver<ToolOutputChunk>>,
    /// Pending tool execution results receiver
    pub tool_results: Option<oneshot::Receiver<Vec<Content>>>,
    /// AI-generated title update receiver
    pub title_update: Option<oneshot::Receiver<TitleUpdate>>,
    /// AI-generated summarization result for pinch
    pub summarization: Option<oneshot::Receiver<SummarizationUpdate>>,
    /// Explore tool sub-agent progress updates
    pub explore_progress: Option<mpsc::UnboundedReceiver<AgentProgress>>,
    /// Build tool builder agent progress updates
    pub build_progress: Option<mpsc::UnboundedReceiver<AgentProgress>>,
    /// OpenRouter model fetch result receiver
    pub openrouter_models: Option<oneshot::Receiver<Result<Vec<ModelMetadata>, String>>>,
    /// /init codebase exploration result receiver
    pub init_exploration: Option<oneshot::Receiver<InitExplorationResult>>,
    /// /init exploration progress updates
    pub init_progress: Option<mpsc::UnboundedReceiver<AgentProgress>>,
    /// Auto-updater status updates
    pub update_status: Option<mpsc::UnboundedReceiver<krusty_core::updater::UpdateStatus>>,
    /// OAuth authentication status updates
    pub oauth_status: Option<mpsc::UnboundedReceiver<OAuthStatusUpdate>>,
}

impl AsyncChannels {
    /// Create new empty channels container
    pub fn new() -> Self {
        Self::default()
    }
}
