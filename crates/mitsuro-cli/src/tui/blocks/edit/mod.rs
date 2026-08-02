//! Edit block - diff display for file edits
//!
//! Shows file edits in GitHub-style diff format:
//! - Always expanded (not collapsible)
//! - Line numbers from original file
//! - Red gutter (-) for deletions, purple gutter (+) for additions
//! - Context lines grayed out
//! - Animated ±/∓ symbol while streaming
//! - Universal toggle for unified/side-by-side view

mod block;
mod render;

use std::time::Duration;

pub use block::EditBlock;

/// Animation frame interval for ±/∓ symbol toggle
pub(crate) const SYMBOL_TOGGLE_INTERVAL: Duration = Duration::from_millis(350);

/// Number of context lines to show around changes (for readability)
pub(crate) const CONTEXT_LINES: usize = 2;

/// Max visible lines before scrolling
pub(crate) const MAX_VISIBLE_LINES: u16 = 15;

/// Diff display mode - controlled globally
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffMode {
    #[default]
    Unified,
    SideBySide,
}

impl DiffMode {
    pub fn toggle(&mut self) {
        *self = match self {
            DiffMode::Unified => DiffMode::SideBySide,
            DiffMode::SideBySide => DiffMode::Unified,
        };
    }

    pub fn icon(&self) -> &'static str {
        match self {
            DiffMode::Unified => "≡",
            DiffMode::SideBySide => "║",
        }
    }
}

/// A single line in the diff
#[derive(Debug, Clone)]
pub enum DiffLine {
    /// Unchanged context line
    Context { line_num: usize, content: String },
    /// Removed line (old)
    Removed { line_num: usize, content: String },
    /// Added line (new)
    Added { line_num: usize, content: String },
}
