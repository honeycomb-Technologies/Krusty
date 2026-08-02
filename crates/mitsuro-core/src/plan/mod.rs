//! Plan system for multi-phase task planning
//!
//! Provides a database-backed planning system with 1:1 session linkage:
//! - Plans stored in SQLite with strict session-plan relationship
//! - Phases → Tasks structure with checkboxes
//! - Plan mode restricts editing tools until approved
//! - Integrates with pinch for context preservation
//! - Automatic cleanup on session deletion (CASCADE)
//!
//! ## Migration
//!
//! Legacy file-based plans (~/.mitsuro/plans/) are automatically migrated
//! to the database on first access. The file-based format is still supported
//! for export/import.

mod file;
mod lifecycle;
mod manager;

pub use file::{PlanFile, PlanPhase, PlanStatus, PlanTask, TaskStatus};
pub use lifecycle::{
    has_active_workflow_or_plan, is_active_plan, resolve_effective_work_mode, PlanLifecycleState,
};
pub use manager::PlanManager;
