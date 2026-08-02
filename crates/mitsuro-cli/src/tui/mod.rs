//! Terminal User Interface for Mitsuro

pub mod animation;
pub mod app;
pub(crate) mod app_builder;
pub(crate) mod auth;
pub mod blocks;
pub mod components;
pub mod graphics;
pub mod handlers;
pub mod input;
pub mod markdown;
pub mod plugins;
pub mod polling;
pub mod popups;
pub mod state;
pub mod streaming;
pub mod themes;
pub(crate) mod tool_presentation;
pub mod utils;

// Re-exports
pub use app::App;
