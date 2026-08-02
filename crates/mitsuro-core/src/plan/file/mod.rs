//! Plan file structure and markdown parser/serializer.
//!
//! Public facade for the plan model, markdown parsing/rendering, response
//! extraction, and task graph operations.

mod markdown;
mod model;
mod response;
mod task_ops;
#[cfg(test)]
mod tests;

pub use self::model::{PlanFile, PlanPhase, PlanStatus, PlanTask, TaskStatus};
