//! Core AI Client
//!
//! The main AiClient struct that handles API communication with multiple providers.
//! Routes requests through appropriate format handlers based on API format.

mod client;
mod system_prompt;
mod transport;

pub use client::{AiClient, PreparedRequestDiagnostics};
pub use system_prompt::MITSURO_SYSTEM_PROMPT;
