//! Application constants and configuration defaults
//!
//! Centralized location for magic numbers and default values

use std::time::Duration;

/// HTTP client configuration
pub mod http {
    use super::*;

    /// Connection timeout for HTTP requests
    pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Streaming timeout - must be long for extended thinking + large tool outputs
    /// SSE streams can run 5+ minutes for complex tasks
    pub const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
}

/// AI/LLM configuration
pub mod ai {
    /// Maximum context window size in tokens
    pub const CONTEXT_WINDOW_TOKENS: usize = 200_000;

    /// Default maximum output tokens (16K for large file writes)
    pub const MAX_OUTPUT_TOKENS: usize = 16384;

    /// Default model ID
    pub const DEFAULT_MODEL: &str = "MiniMax-M2.1";
}

/// UI configuration
pub mod ui {
    /// Config directory name
    pub const CONFIG_DIR_NAME: &str = ".krusty";

    /// Extensions subdirectory name
    pub const EXTENSIONS_DIR_NAME: &str = "extensions";
}
