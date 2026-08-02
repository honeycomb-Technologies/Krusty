//! Cross-transport provider usage normalization.
//!
//! Streaming and non-streaming requests must interpret provider usage with
//! identical cache and reasoning semantics. Keep the wire-shape quirks here so
//! callers only handle Mitsuro's normalized [`Usage`] buckets.

use serde_json::Value;

use super::types::Usage;

pub(crate) fn parse_anthropic_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage").unwrap_or(value);
    let input = token(usage, "input_tokens");
    let output = token(usage, "output_tokens");
    let cache_read = token(usage, "cache_read_input_tokens");
    let cache_write = token(usage, "cache_creation_input_tokens");

    usage_present(input, output, 0, cache_read, cache_write).then_some(Usage {
        // Anthropic reports uncached input separately from cache reads/writes.
        prompt_tokens: input,
        completion_tokens: output,
        reasoning_tokens: 0,
        total_tokens: input
            .saturating_add(cache_read)
            .saturating_add(cache_write)
            .saturating_add(output),
        cache_creation_input_tokens: cache_write,
        cache_read_input_tokens: cache_read,
    })
}

pub(crate) fn parse_google_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usageMetadata").unwrap_or(value);
    let input = token(usage, "promptTokenCount");
    let visible_output = token(usage, "candidatesTokenCount");
    let reasoning = token(usage, "thoughtsTokenCount");
    let output = visible_output.saturating_add(reasoning);
    let cache_read = token(usage, "cachedContentTokenCount");
    let total = token(usage, "totalTokenCount").max(input.saturating_add(output));

    usage_present(input, output, reasoning, cache_read, 0).then_some(Usage {
        // Google includes cached content in promptTokenCount.
        prompt_tokens: input.saturating_sub(cache_read),
        completion_tokens: output,
        reasoning_tokens: reasoning.min(output),
        total_tokens: total,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cache_read,
    })
}

pub(crate) fn parse_openai_chat_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage").unwrap_or(value);
    let input = token(usage, "prompt_tokens");
    let output = token(usage, "completion_tokens");
    let reasoning = usage
        .get("completion_tokens_details")
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| usage.get("reasoning_tokens").and_then(Value::as_u64))
        .unwrap_or(0) as usize;
    let cache_read = usage
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64))
        .unwrap_or(0) as usize;
    let cache_write = usage
        .get("cache_write_tokens")
        .or_else(|| usage.get("cache_creation_input_tokens"))
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|details| details.get("cache_write_tokens"))
        })
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let total = token(usage, "total_tokens").max(input.saturating_add(output));

    usage_present(input, output, reasoning, cache_read, cache_write).then_some(Usage {
        // OpenAI-compatible chat usage includes cached input in prompt_tokens.
        prompt_tokens: input.saturating_sub(cache_read).saturating_sub(cache_write),
        completion_tokens: output,
        reasoning_tokens: reasoning.min(output),
        total_tokens: total,
        cache_creation_input_tokens: cache_write,
        cache_read_input_tokens: cache_read,
    })
}

pub(crate) fn parse_openai_responses_usage(value: &Value) -> Option<Usage> {
    let usage = value
        .get("response")
        .and_then(|response| response.get("usage"))
        .or_else(|| value.get("usage"))
        .unwrap_or(value);
    let input = token_alias(usage, "input_tokens", "input");
    let output = token_alias(usage, "output_tokens", "output");
    let reasoning = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| usage.get("reasoning_tokens").and_then(Value::as_u64))
        .unwrap_or(0) as usize;
    let cache_read = usage
        .get("cached_input")
        .or_else(|| usage.get("cache_read_input_tokens"))
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
        })
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let cache_write = usage
        .get("cache_write_tokens")
        .or_else(|| usage.get("cache_creation_input_tokens"))
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cache_write_tokens"))
        })
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let total = token(usage, "total_tokens").max(input.saturating_add(output));

    usage_present(input, output, reasoning, cache_read, cache_write).then_some(Usage {
        // Responses input_tokens includes both cache buckets.
        prompt_tokens: input.saturating_sub(cache_read).saturating_sub(cache_write),
        completion_tokens: output,
        reasoning_tokens: reasoning.min(output),
        total_tokens: total,
        cache_creation_input_tokens: cache_write,
        cache_read_input_tokens: cache_read,
    })
}

fn token(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_u64).unwrap_or(0) as usize
}

fn token_alias(value: &Value, primary: &str, alias: &str) -> usize {
    value
        .get(primary)
        .or_else(|| value.get(alias))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

fn usage_present(
    input: usize,
    output: usize,
    reasoning: usize,
    cache_read: usize,
    cache_write: usize,
) -> bool {
    input > 0 || output > 0 || reasoning > 0 || cache_read > 0 || cache_write > 0
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        parse_anthropic_usage, parse_google_usage, parse_openai_chat_usage,
        parse_openai_responses_usage,
    };

    #[test]
    fn normalizes_anthropic_cache_buckets() {
        let usage = parse_anthropic_usage(&json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 200,
                "cache_read_input_tokens": 700
            }
        }))
        .expect("usage");

        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.input_tokens(), 1_000);
        assert_eq!(usage.logical_total_tokens(), 1_050);
    }

    #[test]
    fn normalizes_google_reasoning_and_cache() {
        let usage = parse_google_usage(&json!({
            "usageMetadata": {
                "promptTokenCount": 1000,
                "cachedContentTokenCount": 700,
                "candidatesTokenCount": 50,
                "thoughtsTokenCount": 500,
                "totalTokenCount": 1550
            }
        }))
        .expect("usage");

        assert_eq!(usage.prompt_tokens, 300);
        assert_eq!(usage.completion_tokens, 550);
        assert_eq!(usage.reasoning_tokens, 500);
        assert_eq!(usage.logical_total_tokens(), 1_550);
    }

    #[test]
    fn normalizes_openai_chat_cache_and_reasoning() {
        let usage = parse_openai_chat_usage(&json!({
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "total_tokens": 1050,
                "prompt_tokens_details": {"cached_tokens": 700},
                "completion_tokens_details": {"reasoning_tokens": 40}
            }
        }))
        .expect("usage");

        assert_eq!(usage.prompt_tokens, 300);
        assert_eq!(usage.cache_read_input_tokens, 700);
        assert_eq!(usage.reasoning_tokens, 40);
        assert_eq!(usage.logical_total_tokens(), 1_050);
    }

    #[test]
    fn normalizes_nested_responses_usage() {
        let usage = parse_openai_responses_usage(&json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 1000,
                    "output_tokens": 50,
                    "input_tokens_details": {"cached_tokens": 700},
                    "output_tokens_details": {"reasoning_tokens": 25}
                }
            }
        }))
        .expect("usage");

        assert_eq!(usage.prompt_tokens, 300);
        assert_eq!(usage.cache_read_input_tokens, 700);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 25);
        assert_eq!(usage.logical_total_tokens(), 1_050);
    }
}
