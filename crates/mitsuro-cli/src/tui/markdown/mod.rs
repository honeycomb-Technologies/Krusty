//! Markdown rendering for assistant messages
//!
//! Only affects assistant text - does not touch ThinkingBlock,
//! BashBlock, ToolResultBlock, or user/system messages.

use super::themes::Theme;

mod cache;
mod elements;
mod hyperlinks;
mod inline;
mod links;
mod parser;
mod renderer;

pub use cache::MarkdownCache;
pub use hyperlinks::{apply_hyperlinks, apply_link_hover_style};
pub use links::RenderedMarkdown;

/// Render markdown text to styled lines with link tracking
///
/// Returns both the rendered lines and metadata about hyperlink positions.
/// Use `apply_hyperlinks()` to apply OSC 8 sequences after rendering to buffer.
pub fn render_with_links(text: &str, width: usize, theme: &Theme) -> RenderedMarkdown {
    let elements = parser::parse(text);
    renderer::render_elements_with_links(&elements, width, theme)
}
