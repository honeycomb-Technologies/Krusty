//! Mitsuro TUI v2.
//!
//! This module is intentionally isolated from the legacy TUI presentation
//! state. Shared runtime services will be introduced through typed adapters as
//! vertical slices reach parity.

#![allow(
    dead_code,
    reason = "phase-gated v2 contracts intentionally precede their route and service consumers"
)]

mod app;
mod components;
mod input;
mod layout;
mod model;
mod motion;
mod presentation;
mod projection;
mod render;
mod services;
mod terminal;

#[cfg(test)]
mod test_support;

use anyhow::Result;

/// How the TUI finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiOutcome {
    Quit,
    ApplyUpdate { version: String },
}

/// Run the opt-in TUI v2 developer preview.
pub async fn run() -> Result<TuiOutcome> {
    app::run().await
}
