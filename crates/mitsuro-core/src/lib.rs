//! Mitsuro Core - Shared library for AI, storage, tools, and extensions
//!
//! This crate provides the core functionality for the Mitsuro TUI:
//! - Multi-provider AI clients
//! - Tool execution framework
//! - Session and preference storage
//! - MCP (Model Context Protocol) support
//! - ACP (Agent Client Protocol) server for editor integration

pub mod acp;
pub mod agent;
pub mod ai;
pub mod auth;
pub mod constants;
pub mod extensions;
pub mod git;
pub mod hive;
pub mod identity;
pub mod mcp;
pub mod paths;
pub mod plan;
pub mod plugins;
pub mod ports;
pub mod process;
pub mod server_instance;
pub mod skills;
pub mod storage;
pub mod tailscale;
pub mod tools;
pub mod updater;
pub mod workflow;

// Re-exports for convenience
pub use ai::client::{
    AiClient, AiClientConfig, CallOptions, PromptCacheRetention, MITSURO_SYSTEM_PROMPT,
};
pub use ai::streaming::StreamPart;
pub use ai::types::{AiTool, AiToolCall, Content, ModelMessage, Role};
pub use mcp::McpManager;
pub use skills::SkillsManager;
pub use storage::{Database, SessionManager};
pub use tools::ToolRegistry;
