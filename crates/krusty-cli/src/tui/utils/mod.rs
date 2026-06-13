//! Utilities for the TUI
//!
//! Common helper functions and types used throughout the TUI.

mod channels;
pub mod clipboard;
pub mod edit_diff;
mod syntax;
mod text;
mod title;

pub use channels::{
    AsyncChannels, CompactionUpdate, DeviceCodeInfo, DynamicModelUpdate, InitExplorationResult,
    McpStatusUpdate, OAuthStatusUpdate, TitleUpdate,
};
pub use syntax::highlight_code;
pub use text::{count_wrapped_lines, truncate_ellipsis, wrap_line, wrap_text};
pub use title::{TitleAction, TitleEditor};
