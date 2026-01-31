//! Agent module constants
//!
//! Centralized constants for agent configuration, avoiding magic numbers.

/// Token limits for different model tiers
pub mod token_limits {
    /// Small model token limit (Haiku)
    pub const SMALL: u32 = 4096;
    /// Medium model token limit (Sonnet)
    pub const MEDIUM: u32 = 8192;
    /// Large model token limit (Opus)
    pub const LARGE: u32 = 16384;
}

/// Concurrency limits
pub mod concurrency {
    /// Maximum parallel tool executions
    pub const MAX_PARALLEL_TOOLS: usize = 100;
}

/// Timeout configurations
pub mod timeouts {
    use std::time::Duration;

    /// Default tool execution timeout
    pub const TOOL_EXECUTION: Duration = Duration::from_secs(30);
    /// Default streaming timeout
    pub const STREAMING: Duration = Duration::from_secs(120);
    /// Explorer sub-agent per-turn API call timeout
    pub const EXPLORER_API_CALL: Duration = Duration::from_secs(90);
    /// Builder sub-agent per-turn API call timeout
    pub const BUILDER_API_CALL: Duration = Duration::from_secs(180);
}

/// Retry configuration
pub mod retry {
    use std::time::Duration;

    /// Retry delay progression in milliseconds
    pub const DELAYS_MS: &[u64] = &[50, 100, 200, 400, 800, 1000, 1000, 1000, 1000, 1000];

    /// Maximum retry attempts
    pub const MAX_ATTEMPTS: usize = 10;

    /// Minimum wait time before logging retry
    pub const LOG_THRESHOLD: Duration = Duration::from_millis(100);
}

/// Model identifiers
pub mod models {
    /// Claude Haiku 4.5 model ID
    pub const HAIKU_4_5: &str = "claude-haiku-4-5-20251001";
    /// Claude Sonnet 4.5 model ID
    pub const SONNET_4_5: &str = "claude-sonnet-4-5-20250929";
    /// Claude Opus 4.5 model ID
    pub const OPUS_4_5: &str = "claude-opus-4-5-20251101";
}
