//! Multi-provider credential storage
//!
//! Stores API keys for each provider in a JSON file.
//! Also provides unified auth resolution that checks both API keys and OAuth tokens.

mod active_provider;
mod credential_store;

pub use active_provider::ActiveProviderStore;
pub use credential_store::CredentialStore;
