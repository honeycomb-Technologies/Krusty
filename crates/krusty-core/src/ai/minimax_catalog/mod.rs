//! MiniMax's Anthropic-compatible model catalog.
//!
//! MiniMax currently returns a sparse `/models` shape, so live IDs are
//! combined with a small capability overlay for the supported M-series.

mod api;
mod mapping;
mod types;

pub use api::{fetch_models, fetch_models_with_client};
