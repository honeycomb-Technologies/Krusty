use std::sync::Arc;

use agent_client_protocol::StopReason;

use super::PromptProcessor;
use crate::acp::processor::loop_impl::convert_finish_reason;
use crate::ai::providers::ProviderId;
use crate::ai::types::FinishReason;
use crate::tools::registry::ToolRegistry;

#[test]
fn test_convert_finish_reason() {
    assert!(matches!(
        convert_finish_reason(FinishReason::Stop),
        StopReason::EndTurn
    ));
    assert!(matches!(
        convert_finish_reason(FinishReason::ToolCalls),
        StopReason::EndTurn
    ));
    assert!(matches!(
        convert_finish_reason(FinishReason::Length),
        StopReason::MaxTokens
    ));
}

#[test]
fn test_init_ai_client_requires_selected_model() {
    let tools = Arc::new(ToolRegistry::new());
    let mut processor = PromptProcessor::new(tools);

    let initialized = processor.init_ai_client("test-key".to_string(), ProviderId::MiniMax, None);

    assert!(!initialized);
    assert!(processor.ai_client.is_none());
}
