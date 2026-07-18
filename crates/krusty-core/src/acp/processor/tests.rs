use std::sync::Arc;

use super::PromptProcessor;
use crate::ai::models::{ApiFormat, ModelAuthScope};
use crate::ai::providers::ProviderId;
use crate::tools::registry::ToolRegistry;

#[test]
fn test_init_ai_client_requires_selected_model() {
    let tools = Arc::new(ToolRegistry::new());
    let mut processor = PromptProcessor::new(tools);

    let initialized = processor.init_ai_client("test-key".to_string(), ProviderId::MiniMax, None);

    assert!(!initialized);
    assert!(processor.ai_client.is_none());
}

#[test]
fn openai_catalog_scope_selects_the_advertising_transport() {
    let tools = Arc::new(ToolRegistry::new());
    let processor = PromptProcessor::new(tools);

    let oauth = processor
        .build_ai_client_with_auth_scope(
            "oauth-token".to_string(),
            ProviderId::OpenAI,
            Some("gpt-5.6-sol".to_string()),
            Some(ModelAuthScope::OAuth),
            Some("account-1".to_string()),
        )
        .expect("OAuth client");
    assert!(oauth.config().uses_chatgpt_codex_format());
    assert_eq!(oauth.config().api_format, ApiFormat::OpenAIResponses);
    assert_eq!(
        oauth
            .config()
            .custom_headers
            .get("ChatGPT-Account-Id")
            .map(String::as_str),
        Some("account-1")
    );

    let api_key = processor
        .build_ai_client_with_auth_scope(
            "sk-api".to_string(),
            ProviderId::OpenAI,
            Some("gpt-5.6-sol".to_string()),
            Some(ModelAuthScope::ApiKey),
            None,
        )
        .expect("API-key client");
    assert!(!api_key.config().uses_chatgpt_codex_format());
    assert_eq!(api_key.config().api_format, ApiFormat::OpenAIResponses);
    assert_eq!(
        api_key.config().base_url.as_deref(),
        Some("https://api.openai.com/v1/responses")
    );
    assert!(!api_key
        .config()
        .custom_headers
        .contains_key("ChatGPT-Account-Id"));
}

#[test]
fn acp_workspace_root_is_canonicalized_for_sandboxing() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let nested = dir.path().join("nested");
    std::fs::create_dir(&nested)?;
    let session = crate::acp::session::SessionState::new(
        agent_client_protocol::SessionId::from("sandbox-test"),
        Some(nested.join("..")),
        None,
    );

    let root = super::loop_impl::canonical_acp_workspace_root(&session)?;

    assert_eq!(root, dir.path().canonicalize()?);
    Ok(())
}
