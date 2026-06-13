use super::config::ProviderId;

/// Features supported by a provider (used for feature negotiation).
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    /// Server-executed web search.
    pub web_search: bool,
    /// Server-executed web fetch.
    pub web_fetch: bool,
    /// Context management / auto-clearing.
    pub context_management: bool,
    /// Prompt caching support.
    pub prompt_caching: bool,
    /// Web search via plugins array (OpenRouter style).
    pub web_plugins: bool,
    /// Native image/document content block support.
    pub supports_vision: bool,
}

impl ProviderCapabilities {
    /// Get capabilities for a provider.
    pub fn for_provider(provider: ProviderId) -> Self {
        match provider {
            ProviderId::OpenRouter => Self {
                web_search: true,
                web_fetch: true,
                context_management: false,
                prompt_caching: true,
                web_plugins: false,
                supports_vision: true,
            },
            ProviderId::Anthropic => Self {
                web_search: true,
                web_fetch: true,
                context_management: false,
                prompt_caching: true,
                web_plugins: false,
                supports_vision: true,
            },
            ProviderId::OpenAI => Self {
                web_search: true,
                web_fetch: false,
                context_management: false,
                prompt_caching: false,
                web_plugins: false,
                supports_vision: true,
            },
            ProviderId::Grok => Self {
                web_search: false,
                web_fetch: false,
                context_management: false,
                prompt_caching: true,
                web_plugins: false,
                supports_vision: true,
            },
            ProviderId::ZAi | ProviderId::MiniMax => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderCapabilities;
    use crate::ai::providers::ProviderId;

    #[test]
    fn test_provider_capabilities() {
        let openrouter = ProviderCapabilities::for_provider(ProviderId::OpenRouter);
        assert!(openrouter.web_search);
        assert!(openrouter.web_fetch);
        assert!(!openrouter.web_plugins);
        assert!(openrouter.supports_vision);

        let zai = ProviderCapabilities::for_provider(ProviderId::ZAi);
        assert!(!zai.web_search);
        assert!(!zai.web_plugins);
        assert!(!zai.supports_vision);

        let anthropic = ProviderCapabilities::for_provider(ProviderId::Anthropic);
        assert!(anthropic.web_search);
        assert!(anthropic.web_fetch);
        assert!(anthropic.prompt_caching);
        assert!(!anthropic.web_plugins);
        assert!(anthropic.supports_vision);

        let openai = ProviderCapabilities::for_provider(ProviderId::OpenAI);
        assert!(openai.web_search);
        assert!(!openai.web_fetch);
        assert!(!openai.web_plugins);
        assert!(openai.supports_vision);

        let grok = ProviderCapabilities::for_provider(ProviderId::Grok);
        assert!(!grok.web_search);
        assert!(!grok.web_plugins);
        assert!(grok.prompt_caching);
        assert!(grok.supports_vision);

        let minimax = ProviderCapabilities::for_provider(ProviderId::MiniMax);
        assert!(!minimax.web_search);
        assert!(!minimax.web_plugins);
        assert!(!minimax.supports_vision);
    }
}
