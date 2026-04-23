//! WASM Extension Host
//!
//! Ported from Zed's crates/extension_host/src/wasm_host.rs
//! Replaces gpui runtime with tokio

pub mod wit;

mod engine;
mod runtime;
mod state;

pub use engine::wasm_engine;
pub use runtime::{WasmExtension, WasmHost};
pub use state::{WasmState, IS_WASM_THREAD};

// Re-export WIT types for use by consumers
pub use wit::Command;
