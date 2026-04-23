//! Built-in provider registry.

mod builtins;
#[cfg(test)]
mod tests;

use std::sync::LazyLock;

use super::config::{ProviderConfig, ProviderId};

/// Lazily initialized built-in provider configurations.
static BUILTIN_PROVIDERS: LazyLock<Vec<ProviderConfig>> =
    LazyLock::new(builtins::curated_providers);

/// Get all built-in provider configurations (cached, no allocation).
pub fn builtin_providers() -> &'static [ProviderConfig] {
    &BUILTIN_PROVIDERS
}

/// Get a specific provider configuration by ID.
pub fn get_provider(id: ProviderId) -> Option<&'static ProviderConfig> {
    BUILTIN_PROVIDERS.iter().find(|p| p.id == id)
}
