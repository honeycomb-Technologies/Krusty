//! Tool implementations for Krusty
//!
//! Provides the tool registry and all built-in tool implementations.

pub mod git_identity;
pub mod image;
pub mod implementations;
pub mod path_utils;
pub mod registry;

pub use git_identity::{GitIdentity, GitIdentityMode};
pub use image::{
    is_image_extension, is_supported_file, load_from_clipboard_rgba, load_from_path, load_from_url,
};
pub use implementations::{
    register_acp_tools, register_all_tools, register_build_tool, register_explore_tool,
};
pub use registry::{parse_params, ToolContext, ToolOutputChunk, ToolRegistry, ToolResult};
