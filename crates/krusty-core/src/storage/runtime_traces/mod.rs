//! Structured runtime traces for replay, diagnostics, and regression gating.

mod mapping;
mod model;
mod store;
mod summary;
#[cfg(test)]
mod tests;

pub use model::{RuntimeTraceEvent, TraceFailureCategory};
pub use store::RuntimeTraceStore;
pub use summary::{ReplayExpectations, ReplayGateResult, RuntimeTraceSummary, TraceEventCount};
