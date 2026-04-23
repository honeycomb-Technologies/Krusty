//! OpenRouter API integration
//!
//! Fetches available models from OpenRouter's API.

mod api;
mod mapping;
mod types;

pub use api::{fetch_models, fetch_models_with_client};
