//! Background process management
//!
//! Tracks spawned background processes for visibility and control.

mod model;
mod registry;
pub(crate) mod signals;

pub use model::{ProcessCompletionEvent, ProcessId, ProcessInfo, ProcessStatus};
pub use registry::ProcessRegistry;
