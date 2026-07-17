//! OpenAI model catalog integration.
//!
//! Fetches available text-generation models from OpenAI's `/v1/models` API.

mod api;
mod mapping;
mod types;

pub use api::{
    fetch_chatgpt_models, fetch_chatgpt_models_with_client, fetch_models, fetch_models_with_client,
};
