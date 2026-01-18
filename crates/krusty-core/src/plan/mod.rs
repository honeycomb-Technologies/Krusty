//! Plan system for multi-phase task planning
//!
//! Provides a file-based planning system:
//! - Plans stored as markdown in ~/.krusty/plans/
//! - Phases → Tasks structure with checkboxes
//! - Plan mode restricts editing tools until approved
//! - Integrates with pinch for context preservation

mod file;
mod manager;

pub use file::{PlanFile, PlanStatus};
pub use manager::PlanManager;
