//! ACP-specific error types

use thiserror::Error;

/// ACP error type
#[derive(Debug, Error)]
pub enum AcpError {
    /// Session not found
    #[error("session not found: {0}")]
    SessionNotFound(String),

    /// Session already exists
    #[error("session already exists: {0}")]
    SessionExists(String),

    /// Authentication required
    #[error("authentication required")]
    AuthenticationRequired,

    /// Authentication failed
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Invalid request
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// Protocol error
    #[error("protocol error: {0}")]
    ProtocolError(String),

    /// Internal error
    #[error("internal error: {0}")]
    InternalError(String),

    /// IO error
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    /// Serialization error
    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Request cancelled by client
    #[error("request cancelled")]
    Cancelled,

    /// Tool execution error
    #[error("tool error: {0}")]
    ToolError(String),

    /// AI client error
    #[error("ai error: {0}")]
    AiError(String),

    /// AI client error (alias for backwards compatibility)
    #[error("ai client error: {0}")]
    AiClientError(String),

    /// Not authenticated (need to call authenticate first)
    #[error("not authenticated: {0}")]
    NotAuthenticated(String),

    /// Capability not supported
    #[error("capability not supported: {0}")]
    CapabilityNotSupported(String),
}

impl AcpError {
    /// Convert to JSON-RPC error code
    pub fn error_code(&self) -> i32 {
        match self {
            AcpError::SessionNotFound(_) => -32002,
            AcpError::SessionExists(_) => -32001,
            AcpError::AuthenticationRequired => -32000,
            AcpError::AuthenticationFailed(_) => -32000,
            AcpError::NotAuthenticated(_) => -32000,
            AcpError::InvalidRequest(_) => -32600,
            AcpError::ProtocolError(_) => -32600,
            AcpError::InternalError(_) => -32603,
            AcpError::IoError(_) => -32603,
            AcpError::SerializationError(_) => -32700,
            AcpError::Cancelled => -32001,
            AcpError::ToolError(_) => -32603,
            AcpError::AiError(_) => -32603,
            AcpError::AiClientError(_) => -32603,
            AcpError::CapabilityNotSupported(_) => -32601,
        }
    }
}

impl From<anyhow::Error> for AcpError {
    fn from(err: anyhow::Error) -> Self {
        AcpError::InternalError(err.to_string())
    }
}
