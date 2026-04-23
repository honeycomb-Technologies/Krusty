//! Autonomous/Mako-specific agent runtime.
//!
//! This subtree isolates the always-on coordination surfaces from the default
//! interactive agent loop.

pub mod auto_classifier;
pub mod coordinator_prompt;
pub mod team;
pub mod tick_engine;
