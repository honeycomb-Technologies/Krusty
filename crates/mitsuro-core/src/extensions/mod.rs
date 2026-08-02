//! Zed-compatible WASM Extension System
//!
//! This module provides a WASM-based extension system compatible with Zed's extensions.
//! Ported from Zed's crates/extension and crates/extension_host, adapted for tokio runtime.

pub mod agent;
pub mod bun_runtime;
pub mod github;
pub mod manifest;
pub mod types;
pub mod wasm_host;

pub use agent::{
    AgentExtensionCommand, AgentExtensionDiagnostic, AgentExtensionManager, AgentExtensionManifest,
    AgentExtensionPermissions, AgentExtensionRoot, AgentExtensionScope, AgentExtensionStatus,
    AgentExtensionToolIntercept, ExtensionCallContext, ProjectAgentExtensionTrustStatus,
};
pub use manifest::*;
pub use wasm_host::{WasmExtension, WasmHost};
