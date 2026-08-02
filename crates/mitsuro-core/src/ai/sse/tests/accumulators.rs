use crate::ai::sse::{ServerToolAccumulator, ThinkingAccumulator, ToolCallAccumulator};

#[test]
fn test_tool_call_accumulator_new() {
    let acc = ToolCallAccumulator::new("id_123".to_string(), "my_tool".to_string());
    assert_eq!(acc.id, "id_123");
    assert_eq!(acc.name, "my_tool");
    assert!(acc.arguments.is_empty());
    assert!(!acc.is_complete);
}

#[test]
fn test_tool_call_accumulator_add_arguments() {
    let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
    acc.add_arguments("{\"key\":");
    acc.add_arguments("\"value\"}");
    assert_eq!(acc.arguments, "{\"key\":\"value\"}");
}

#[test]
fn test_tool_call_accumulator_try_complete_incomplete_json() {
    let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
    acc.add_arguments("{\"incomplete\":");
    assert!(acc.try_complete().is_none());
    assert!(!acc.is_complete);
}

#[test]
fn test_tool_call_accumulator_try_complete_valid_json() {
    let mut acc = ToolCallAccumulator::new("id_1".to_string(), "read".to_string());
    acc.add_arguments("{\"path\": \"/tmp/test.txt\"}");
    let result = acc.try_complete();
    assert!(result.is_some());
    let tool_call = result.unwrap();
    assert_eq!(tool_call.id, "id_1");
    assert_eq!(tool_call.name, "read");
    assert!(acc.is_complete);
}

#[test]
fn test_tool_call_accumulator_force_complete_valid_json() {
    let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
    acc.add_arguments("{\"a\": 1}");
    let result = acc.force_complete();
    assert_eq!(result.arguments["a"], 1);
    assert!(acc.is_complete);
}

#[test]
fn test_tool_call_accumulator_force_complete_invalid_json() {
    let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
    acc.add_arguments("not valid json");
    let result = acc.force_complete();
    assert_eq!(result.arguments["raw"], "not valid json");
}

#[test]
fn test_tool_call_accumulator_force_complete_empty() {
    let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
    let result = acc.force_complete();
    assert_eq!(result.arguments, serde_json::json!({}));
}

#[test]
fn test_tool_call_accumulator_try_complete_empty_returns_none() {
    let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
    assert!(acc.try_complete().is_none());
}

#[test]
fn test_server_tool_accumulator_new() {
    let acc = ServerToolAccumulator::new("st_123".to_string(), "web_search".to_string());
    assert_eq!(acc.id, "st_123");
    assert_eq!(acc.name, "web_search");
    assert!(acc.input_json.is_empty());
    assert!(!acc.is_complete);
}

#[test]
fn test_server_tool_accumulator_add_input() {
    let mut acc = ServerToolAccumulator::new("id".to_string(), "web_search".to_string());
    acc.add_input("{\"query\":");
    acc.add_input("\"rust async\"}");
    assert_eq!(acc.input_json, "{\"query\":\"rust async\"}");
}

#[test]
fn test_server_tool_accumulator_complete_valid_json() {
    let mut acc = ServerToolAccumulator::new("id".to_string(), "web_search".to_string());
    acc.add_input("{\"query\": \"test\"}");
    let result = acc.complete();
    assert_eq!(result["query"], "test");
    assert!(acc.is_complete);
}

#[test]
fn test_server_tool_accumulator_complete_invalid_json() {
    let mut acc = ServerToolAccumulator::new("id".to_string(), "tool".to_string());
    acc.add_input("malformed {json");
    let result = acc.complete();
    assert_eq!(result["raw"], "malformed {json");
}

#[test]
fn test_server_tool_accumulator_complete_empty() {
    let mut acc = ServerToolAccumulator::new("id".to_string(), "tool".to_string());
    let result = acc.complete();
    assert_eq!(result, serde_json::json!({}));
}

#[test]
fn test_thinking_accumulator_new() {
    let acc = ThinkingAccumulator::new();
    assert!(acc.thinking.is_empty());
    assert!(acc.signature.is_empty());
    assert!(!acc.is_complete);
}

#[test]
fn test_thinking_accumulator_default() {
    let acc = ThinkingAccumulator::default();
    assert!(acc.thinking.is_empty());
}

#[test]
fn test_thinking_accumulator_add_thinking() {
    let mut acc = ThinkingAccumulator::new();
    acc.add_thinking("Let me think about ");
    acc.add_thinking("this problem...");
    assert_eq!(acc.thinking, "Let me think about this problem...");
}

#[test]
fn test_thinking_accumulator_add_signature() {
    let mut acc = ThinkingAccumulator::new();
    acc.add_signature("sig_part1");
    acc.add_signature("_part2");
    assert_eq!(acc.signature, "sig_part1_part2");
}

#[test]
fn test_thinking_accumulator_complete() {
    let mut acc = ThinkingAccumulator::new();
    acc.add_thinking("My analysis is...");
    acc.add_signature("abcdef123");
    let (thinking, signature) = acc.complete();
    assert_eq!(thinking, "My analysis is...");
    assert_eq!(signature, "abcdef123");
    assert!(acc.is_complete);
}
