use std::pin::Pin;

use anyhow::{Context as _, Result};
use futures_core::Stream;
use futures_util::StreamExt as _;
use reqwest::Response;
use serde_json::Value;

use crate::ChatStreamEvent;

pub type ChatEventStream = Pin<Box<dyn Stream<Item = Result<ChatStreamEvent>> + Send + 'static>>;

pub fn chat_stream_from_response(response: Response) -> ChatEventStream {
    let mut chunks = response.bytes_stream();
    Box::pin(async_stream::try_stream! {
        let mut line_buffer = Vec::<u8>::new();
        let mut decoder = SseDecoder::default();

        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.context("reading SSE response chunk")?;
            line_buffer.extend_from_slice(&chunk);

            while let Some(line_end) = line_buffer.iter().position(|byte| *byte == b'\n') {
                let line = line_buffer.drain(..=line_end).collect::<Vec<_>>();
                let line = decode_sse_line(&line)?;
                if let Some(event) = decoder.push_line(&line)? {
                    yield event;
                }
            }
        }

        if !line_buffer.is_empty() {
            let line = decode_sse_line(&line_buffer)?;
            if let Some(event) = decoder.push_line(&line)? {
                yield event;
            }
        }

        if let Some(event) = decoder.finish()? {
            yield event;
        }
    })
}

pub fn parse_sse_data_line(line: &str) -> Result<Option<ChatStreamEvent>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return Ok(None);
    }

    let Some(data) = trimmed.strip_prefix("data:") else {
        return Ok(None);
    };
    parse_sse_payload(data.trim())
}

#[derive(Default)]
struct SseDecoder {
    data: String,
}

impl SseDecoder {
    fn push_line(&mut self, line: &str) -> Result<Option<ChatStreamEvent>> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(None);
        };
        let data = data.strip_prefix(' ').unwrap_or(data);
        if !self.data.is_empty() {
            self.data.push('\n');
        }
        self.data.push_str(data);
        Ok(None)
    }

    fn finish(&mut self) -> Result<Option<ChatStreamEvent>> {
        self.dispatch()
    }

    fn dispatch(&mut self) -> Result<Option<ChatStreamEvent>> {
        let data = std::mem::take(&mut self.data);
        parse_sse_payload(data.trim())
    }
}

fn parse_sse_payload(data: &str) -> Result<Option<ChatStreamEvent>> {
    if data.is_empty() || data == "[DONE]" {
        return Ok(None);
    }

    let value = serde_json::from_str::<Value>(data)
        .with_context(|| format!("parsing SSE data payload: {data}"))?;
    Ok(Some(ChatStreamEvent::from_json_value(value)))
}

fn decode_sse_line(line: &[u8]) -> Result<String> {
    let without_lf = line.strip_suffix(b"\n").unwrap_or(line);
    Ok(std::str::from_utf8(without_lf)
        .context("decoding SSE line as UTF-8")?
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_delta_line() {
        let event = parse_sse_data_line(r#"data: {"type":"text_delta","delta":"hi"}"#)
            .expect("valid line")
            .expect("event");
        assert_eq!(
            event,
            ChatStreamEvent::TextDelta {
                delta: "hi".to_owned()
            }
        );
    }

    #[test]
    fn ignores_comments_and_empty_lines() {
        assert!(parse_sse_data_line(": keepalive").expect("line").is_none());
        assert!(parse_sse_data_line("   ").expect("line").is_none());
    }

    #[test]
    fn keeps_unknown_events_for_forward_compatibility() {
        let event = parse_sse_data_line(r#"data: {"type":"future_event","value":1}"#)
            .expect("valid line")
            .expect("event");
        match event {
            ChatStreamEvent::Other {
                event_type,
                payload,
            } => {
                assert_eq!(event_type, "future_event");
                assert_eq!(payload["value"], 1);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn decodes_event_after_blank_line() {
        let mut decoder = SseDecoder::default();
        assert!(decoder
            .push_line(r#"data: {"type":"text_delta","delta":"hi"}"#)
            .expect("line")
            .is_none());
        let event = decoder.push_line("").expect("dispatch").expect("event");
        assert_eq!(
            event,
            ChatStreamEvent::TextDelta {
                delta: "hi".to_owned()
            }
        );
    }

    #[test]
    fn flushes_tail_without_blank_line() {
        let mut decoder = SseDecoder::default();
        assert!(decoder
            .push_line(r#"data: {"type":"finish","session_id":"s1","stop_reason":"done"}"#)
            .expect("line")
            .is_none());
        let event = decoder.finish().expect("finish").expect("event");
        assert_eq!(
            event,
            ChatStreamEvent::Finish {
                session_id: "s1".to_owned(),
                stop_reason: "done".to_owned(),
            }
        );
    }

    #[test]
    fn decodes_utf8_line_after_split_chunks() {
        let line = decode_sse_line("data: café\n".as_bytes()).expect("utf8");
        assert_eq!(line, "data: café");
    }
}
