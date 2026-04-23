//! Background process management
//!
//! Tracks spawned background processes for visibility and control.

mod model;
mod registry;
mod signals;

pub use model::{ProcessId, ProcessInfo, ProcessStatus};
pub use registry::ProcessRegistry;
