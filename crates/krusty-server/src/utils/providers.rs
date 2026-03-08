use krusty_core::ai::providers::ProviderId;

pub fn parse_provider(s: &str) -> Option<ProviderId> {
    match s.to_ascii_lowercase().as_str() {
        "minimax" => Some(ProviderId::MiniMax),
        "openrouter" => Some(ProviderId::OpenRouter),
        "z_ai" | "zai" => Some(ProviderId::ZAi),
        "openai" => Some(ProviderId::OpenAI),
        "anthropic" => Some(ProviderId::Anthropic),
        _ => None,
    }
}
