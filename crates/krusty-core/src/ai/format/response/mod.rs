//! Response normalization
//!
//! Converts responses from different API formats to a unified Anthropic-style format
//! for consistent downstream processing.

mod codex;
mod google;
mod openai;
mod shared;

pub use codex::normalize_codex_response;
pub use google::normalize_google_response;
pub use openai::normalize_openai_response;
pub use shared::extract_text_from_content;
