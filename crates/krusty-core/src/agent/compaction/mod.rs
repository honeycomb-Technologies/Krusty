//! In-place context compaction for long-running agent sessions.
//!
//! Replaces session-forking pinch with a layered pipeline:
//! microcompact → archived segment + LLM summary + preserved tail → post-compact restore.

mod apply;
mod budget;
pub(crate) mod cut_point;
pub(crate) mod microcompact;
mod overflow;
mod pipeline;
mod summarize;

pub use overflow::is_context_overflow_error;

pub use budget::{
    effective_context_window_for_runtime, estimate_rendered_request_tokens, CompactionManager,
    CompactionRequestBudget, RenderedRequestTokenEstimate,
};

pub(crate) use budget::estimate_with_usage;
pub(crate) use pipeline::run_compaction_pipeline_observed;
pub use pipeline::{
    run_compaction_pipeline, CompactionRequest, CompactionResult, CompactionTrigger,
};

pub(crate) const COMPACTION_BOUNDARY_PREFIX: &str = "[COMPACTION_BOUNDARY]";
pub(crate) const COMPACTION_SUMMARY_PREFIX: &str = "# Conversation Compacted\n\n";

pub(crate) const DEFAULT_KEEP_RECENT_TOKENS: usize = 20_000;
pub(crate) const DEFAULT_RESERVE_TOKENS: usize = 16_384;

#[cfg(test)]
mod tests;
