//! Shared terminal support used by tui_v2 (not a product UI surface).
//!
//! Legacy product TUI (handlers/blocks/popups) is archived on
//! `archive/tui-v1-20260802`.

#![allow(dead_code, unused_imports, unused_variables)]

pub mod app_builder;
pub mod auth;
pub mod graphics;
pub mod image_parser;
pub mod markdown;
pub mod plugins;
pub mod services;
pub mod themes;
pub mod utils;

pub use image_parser::{has_image_references, parse_input, InputSegment};
pub use services::AppServices;
