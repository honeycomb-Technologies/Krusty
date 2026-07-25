//! Autonomous/Mako-specific agent runtime.
//!
//! This subtree isolates the always-on coordination surfaces from the default
//! interactive agent loop.

pub mod auto_classifier;
pub mod coordinator_prompt;
// The unused rigid TeamManager/TeammateRole prototype is intentionally not
// compiled. Dynamic delegated work is owned by AgentSpec plus the unified
// agent lifecycle control plane.
pub mod tick_engine;
