use krusty_core::ai::providers::ProviderId;

pub fn parse_provider(s: &str) -> Option<ProviderId> {
    match s.to_ascii_lowercase().as_str() {
        "minimax" => Some(ProviderId::MiniMax),
        "openrouter" => Some(ProviderId::OpenRouter),
        "z_ai" | "zai" => Some(ProviderId::ZAi),
        "openai" => Some(ProviderId::OpenAI),
        "anthropic" => Some(ProviderId::Anthropic),
        "grok" | "xai" | "x_ai" => Some(ProviderId::Grok),
        _ => None,
    }
}

pub fn provider_display_name(provider_id: ProviderId) -> &'static str {
    match provider_id {
        ProviderId::MiniMax => "MiniMax",
        ProviderId::OpenRouter => "OpenRouter",
        ProviderId::ZAi => "Z.ai",
        ProviderId::Anthropic => "Anthropic",
        ProviderId::OpenAI => "OpenAI",
        ProviderId::Grok => "xAI",
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_provider, provider_display_name};
    use krusty_core::ai::providers::ProviderId;

    #[test]
    fn parse_provider_accepts_xai_aliases() {
        assert_eq!(parse_provider("grok"), Some(ProviderId::Grok));
        assert_eq!(parse_provider("xai"), Some(ProviderId::Grok));
        assert_eq!(parse_provider("x_ai"), Some(ProviderId::Grok));
    }

    #[test]
    fn provider_display_name_uses_product_casing() {
        assert_eq!(provider_display_name(ProviderId::MiniMax), "MiniMax");
        assert_eq!(provider_display_name(ProviderId::OpenAI), "OpenAI");
        assert_eq!(provider_display_name(ProviderId::Grok), "xAI");
    }
}
