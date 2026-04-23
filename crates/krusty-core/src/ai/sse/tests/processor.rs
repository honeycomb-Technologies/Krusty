use crate::ai::sse::{SseEvent, SseParser, SseStreamProcessor};
use crate::ai::streaming::StreamPart;
use crate::ai::types::FinishReason;
use serde_json::Value;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_sse_processor_done_marker() {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
    let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
    let mut processor = SseStreamProcessor::new(tx, buffer_tx);

    struct MockParser;
    #[async_trait::async_trait]
    impl SseParser for MockParser {
        async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
            Ok(SseEvent::Skip)
        }
    }

    processor
        .process_sse_data("[DONE]", &MockParser)
        .await
        .unwrap();
    let part = rx.recv().await.unwrap();
    assert!(matches!(
        part,
        StreamPart::Finish {
            reason: FinishReason::Stop
        }
    ));
}

#[tokio::test]
async fn test_sse_processor_text_delta() {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
    let (buffer_tx, mut buffer_rx) = mpsc::unbounded_channel::<String>();
    let mut processor = SseStreamProcessor::new(tx, buffer_tx);

    struct TextDeltaParser;
    #[async_trait::async_trait]
    impl SseParser for TextDeltaParser {
        async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
            Ok(SseEvent::TextDelta(
                "This is a longer text that exceeds the buffer chunk size of 64 characters to ensure immediate flushing.".to_string()
            ))
        }
    }

    processor
        .process_sse_data("{}", &TextDeltaParser)
        .await
        .unwrap();
    let text = buffer_rx.recv().await.unwrap();
    assert!(!text.is_empty());
    processor.finish().await;
    drop(processor);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_sse_processor_tool_call_start() {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
    let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
    let mut processor = SseStreamProcessor::new(tx, buffer_tx);

    struct ToolStartParser;
    #[async_trait::async_trait]
    impl SseParser for ToolStartParser {
        async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
            Ok(SseEvent::ToolCallStart {
                id: "tool_123".to_string(),
                name: "read".to_string(),
            })
        }
    }

    processor
        .process_sse_data("{}", &ToolStartParser)
        .await
        .unwrap();
    let part = rx.recv().await.unwrap();
    match part {
        StreamPart::ToolCallStart { id, name } => {
            assert_eq!(id, "tool_123");
            assert_eq!(name, "read");
        }
        _ => panic!("Expected ToolCallStart"),
    }
}

#[tokio::test]
async fn test_sse_processor_skip_empty_json() {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
    let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
    let mut processor = SseStreamProcessor::new(tx, buffer_tx);

    struct SkipParser;
    #[async_trait::async_trait]
    impl SseParser for SkipParser {
        async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
            Ok(SseEvent::Skip)
        }
    }

    processor.process_sse_data("", &SkipParser).await.unwrap();
    processor
        .process_sse_data("   ", &SkipParser)
        .await
        .unwrap();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_sse_processor_thinking_events() {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
    let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
    let mut processor = SseStreamProcessor::new(tx, buffer_tx);

    struct ThinkingStartParser;
    #[async_trait::async_trait]
    impl SseParser for ThinkingStartParser {
        async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
            Ok(SseEvent::ThinkingStart { index: 0 })
        }
    }

    processor
        .process_sse_data("{}", &ThinkingStartParser)
        .await
        .unwrap();
    let part = rx.recv().await.unwrap();
    assert!(matches!(part, StreamPart::ThinkingStart { index: 0 }));
}
