use crate::ai::sse::parse_finish_reason;
use crate::ai::types::FinishReason;

#[test]
fn test_parse_finish_reason_stop() {
    assert!(matches!(parse_finish_reason("stop"), FinishReason::Stop));
}

#[test]
fn test_parse_finish_reason_end_turn() {
    assert!(matches!(
        parse_finish_reason("end_turn"),
        FinishReason::Stop
    ));
}

#[test]
fn test_parse_finish_reason_max_tokens() {
    assert!(matches!(
        parse_finish_reason("max_tokens"),
        FinishReason::Length
    ));
}

#[test]
fn test_parse_finish_reason_tool_use() {
    assert!(matches!(
        parse_finish_reason("tool_use"),
        FinishReason::ToolCalls
    ));
}

#[test]
fn test_parse_finish_reason_unknown() {
    match parse_finish_reason("something_else") {
        FinishReason::Other(s) => assert_eq!(s, "something_else"),
        _ => panic!("Expected FinishReason::Other"),
    }
}
