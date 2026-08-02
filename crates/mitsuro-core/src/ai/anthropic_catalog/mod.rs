//! Anthropic's account-scoped model catalog.
//!
//! The `/v1/models` response includes richer capability metadata than the
//! inference transport. Keep that provider-specific shape isolated here and
//! normalize it into [`ModelMetadata`](crate::ai::models::ModelMetadata).

mod api;
mod mapping;
mod types;

pub use api::{fetch_models, fetch_models_with_client};
