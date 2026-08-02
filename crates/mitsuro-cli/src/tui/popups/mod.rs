//! Popup system for Mitsuro TUI
//!
//! Follows mitsuro's popup patterns:
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
pub mod mcp_browser;
pub mod model_select;
pub mod plugins;
pub mod process_list;
pub mod scroll;
pub mod session_list;
pub mod skills_browser;
pub mod theme_select;
