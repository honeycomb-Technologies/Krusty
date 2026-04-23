//! Plan storage with strict session linkage
//!
//! Provides SQLite-backed plan storage with:
//! - 1:1 session-plan relationship (enforced by UNIQUE constraint)
//! - Automatic plan deletion on session delete (CASCADE)
//! - CRUD operations for plans

mod model;
mod store;

pub use model::PlanSummary;
pub use store::PlanStore;
