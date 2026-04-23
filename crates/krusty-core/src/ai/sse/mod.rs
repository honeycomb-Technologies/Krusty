//! SSE (Server-Sent Events) stream processing utilities
//!
//! Handles parsing of SSE streams from AI providers.

mod accumulators;
mod channels;
mod events;
mod processor;

pub use accumulators::{ServerToolAccumulator, ThinkingAccumulator, ToolCallAccumulator};
pub use channels::{create_streaming_channels, spawn_buffer_processor};
pub use events::{parse_finish_reason, SseEvent, SseParser};
pub use processor::SseStreamProcessor;

#[cfg(test)]
mod tests;
