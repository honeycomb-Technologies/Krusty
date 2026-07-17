use std::sync::Arc;

use super::PromptProcessor;
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
