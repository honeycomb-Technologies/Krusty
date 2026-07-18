//! Async Channels
//!
//! Groups all async channel receivers used by the App for background tasks.

use tokio::sync::{mpsc, oneshot};

use crate::agent::subagent::AgentProgress;
use crate::agent::{CompactionResult, DelegatedProgressEvent, LoopEvent, LoopInput};
use crate::ai::models::ModelMetadata;
use crate::ai::providers::ProviderId;
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

/// In-place compaction result for manual pinch
pub struct CompactionUpdate {
    pub result: Result<CompactionResult, String>,
    pub auto_continue: bool,
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

/// Result from fetching models for a dynamic provider.
pub struct DynamicModelUpdate {
    pub provider: ProviderId,
    pub result: Result<Vec<ModelMetadata>, String>,
}

/// Completion from an executable agent-extension slash command.
pub struct ExtensionCommandUpdate {
    pub command: String,
    pub result: Result<serde_json::Value, String>,
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
    /// Streaming bash output receiver (bounded for backpressure)
    pub bash_output: Option<mpsc::Receiver<ToolOutputChunk>>,
    /// AI-generated title update receiver
    pub title_update: Option<oneshot::Receiver<TitleUpdate>>,
    /// Manual pinch compaction apply result
    pub compaction: Option<oneshot::Receiver<CompactionUpdate>>,
    /// Explore tool sub-agent progress updates (bounded for backpressure)
    pub explore_progress: Option<mpsc::Receiver<AgentProgress>>,
    /// Build tool builder agent progress updates (bounded for backpressure)
    pub build_progress: Option<mpsc::Receiver<AgentProgress>>,
    /// Dynamic provider model fetch results (supports concurrent provider refreshes).
    /// Unbounded is intentional: refreshes are rare (startup/OAuth/model popup) and
    /// coalesced by `dynamic_model_fetches` so only one in-flight fetch per provider.
    pub dynamic_models: Option<mpsc::UnboundedReceiver<DynamicModelUpdate>>,
    /// Sender half for concurrent dynamic model fetches
    pub dynamic_models_tx: Option<mpsc::UnboundedSender<DynamicModelUpdate>>,
    /// /init codebase exploration result receiver
    pub init_exploration: Option<oneshot::Receiver<InitExplorationResult>>,
    /// /init exploration progress updates
    pub init_progress: Option<mpsc::UnboundedReceiver<AgentProgress>>,
    /// Auto-updater status updates
    pub update_status: Option<mpsc::UnboundedReceiver<krusty_core::updater::UpdateStatus>>,
    /// OAuth authentication status updates
    pub oauth_status: Option<mpsc::UnboundedReceiver<OAuthStatusUpdate>>,
    /// Anthropic PKCE verifier for paste-code flow
    pub anthropic_verifier: Option<oneshot::Receiver<krusty_core::auth::PkceVerifier>>,
    /// Core orchestrator event receiver (replaces StreamingManager when active)
    pub loop_events: Option<mpsc::UnboundedReceiver<LoopEvent>>,
    /// Core orchestrator input sender (for approvals, AskUser responses, cancellation)
    pub loop_input: Option<mpsc::UnboundedSender<LoopInput>>,
    /// Delegated agent progress emitted by the core orchestrator.
    pub delegated_progress: Option<mpsc::UnboundedReceiver<DelegatedProgressEvent>>,
    /// Agent-extension slash command completions.
    pub extension_commands: Option<mpsc::UnboundedReceiver<ExtensionCommandUpdate>>,
    /// Shared sender so concurrent extension commands cannot replace and lose
    /// an earlier command's completion receiver.
    pub extension_commands_tx: Option<mpsc::UnboundedSender<ExtensionCommandUpdate>>,
}

impl AsyncChannels {
    /// Create new empty channels container
    pub fn new() -> Self {
        Self::default()
    }

    pub fn extension_command_sender(&mut self) -> mpsc::UnboundedSender<ExtensionCommandUpdate> {
        if let Some(sender) = &self.extension_commands_tx {
            return sender.clone();
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        self.extension_commands = Some(receiver);
        self.extension_commands_tx = Some(sender.clone());
        sender
    }
}

#[cfg(test)]
mod tests {
    use super::{AsyncChannels, ExtensionCommandUpdate};

    #[test]
    fn extension_commands_share_one_completion_queue() {
        let mut channels = AsyncChannels::new();
        let first = channels.extension_command_sender();
        let second = channels.extension_command_sender();
        first
            .send(ExtensionCommandUpdate {
                command: "/one".to_string(),
                result: Ok(serde_json::Value::Null),
            })
            .expect("first completion");
        second
            .send(ExtensionCommandUpdate {
                command: "/two".to_string(),
                result: Ok(serde_json::Value::Null),
            })
            .expect("second completion");

        let receiver = channels
            .extension_commands
            .as_mut()
            .expect("shared receiver");
        assert_eq!(receiver.try_recv().expect("first").command, "/one");
        assert_eq!(receiver.try_recv().expect("second").command, "/two");
    }
}
