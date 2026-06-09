//! AI provider layer
//!
//! Handles communication with AI providers (MiniMax, OpenRouter, ZAi, OpenAI, etc.)
//! Supports multiple API formats: Anthropic, OpenAI, and Google.

// Modular architecture
pub mod catalog;
pub mod client;
pub mod format;
pub mod format_detection;
pub mod grok;
pub mod retry;

// Provider-specific configuration
pub mod glm;
pub mod model_profile;
pub mod models;
pub mod openai;
pub mod openrouter;

// Shared infrastructure
pub mod parsers;
pub mod providers;
pub mod reasoning;
pub mod sse;
pub mod stream_buffer;
pub mod streaming;
pub mod title;
pub mod transform;
pub mod types;

// Re-export main types from new module
pub use client::{AiClient, AiClientConfig, CallOptions, KRUSTY_SYSTEM_PROMPT};

pub use title::{generate_pinch_title, generate_title};
