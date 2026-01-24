//! Popup system for Krusty TUI
//!
//! Follows krusty's popup patterns:
//! - Consistent sizing per popup type
//! - Title + separator + content + footer
//! - Rounded borders
//! - Scroll indicators
//! - Theme-aware colors

pub mod auth;
pub mod common;
pub mod file_preview;
pub mod help;
pub mod hooks;
pub mod lsp_browser;
pub mod lsp_install;
pub mod mcp_browser;
pub mod model_select;
pub mod pinch;
pub mod process_list;
pub mod scroll;
pub mod session_list;
pub mod skills_browser;
pub mod theme_select;
