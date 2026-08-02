//! Canonical message display-list rendering.
//!
//! Text and stream blocks share one coordinate space. Rendering never paints
//! placeholder rows and then overlays blocks, which keeps scrolling, clipping,
//! selection, hyperlinks, and block widgets from drifting apart.

mod selection;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::blocks::{BlockType, ClipContext, StreamBlock};
use crate::tui::markdown::{apply_hyperlinks, apply_link_hover_style, RenderedMarkdown};
use crate::tui::state::SelectionArea;
use crate::tui::utils::wrap_line;

use super::display_list::{DisplayItem, DisplayItemKind, DisplayList};
use selection::{
    apply_selection_to_line, apply_selection_to_rendered_line, style_user_line_with_file_refs,
    UserLineFileRefOptions,
};

const USER_SYMBOL: &str = "⤷ ";
const ASSISTANT_SYMBOL: &str = "⬡ ";
pub const SYMBOL_WIDTH: usize = 2;
const SCROLLBAR_GAP: u16 = 4;

fn hash_content(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn clear_area(buffer: &mut ratatui::buffer::Buffer, area: Rect, background: Color) {
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_bg(background);
                cell.set_fg(Color::Reset);
            }
        }
    }
}

fn visible_item_area(
    item: &DisplayItem,
    viewport: Rect,
    scroll: usize,
) -> Option<(Rect, u16, u16)> {
    let viewport_end = scroll.saturating_add(viewport.height as usize);
    let item_end = item.line_start.saturating_add(item.height);
    if item_end <= scroll || item.line_start >= viewport_end || item.height == 0 {
        return None;
    }

    let clip_top = scroll.saturating_sub(item.line_start).min(item.height);
    let screen_y = viewport
        .y
        .saturating_add(item.line_start.saturating_sub(scroll) as u16);
    let available = viewport
        .height
        .saturating_sub(screen_y.saturating_sub(viewport.y));
    let visible_height = (item.height - clip_top).min(available as usize) as u16;
    if visible_height == 0 {
        return None;
    }

    let clip_bottom = item
        .height
        .saturating_sub(clip_top)
        .saturating_sub(visible_height as usize) as u16;
    Some((
        Rect::new(viewport.x, screen_y, viewport.width, visible_height),
        clip_top as u16,
        clip_bottom,
    ))
}

impl App {
    pub fn render_messages(&mut self, frame: &mut Frame, area: Rect) {
        let panel = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.ui.theme.border_color));
        let inner = panel.inner(area);
        frame.render_widget(panel, area);

        let content_width = inner.width.saturating_sub(SCROLLBAR_GAP);
        let wrap_width = (content_width as usize).saturating_sub(SYMBOL_WIDTH);
        let content_rect = Rect::new(inner.x, inner.y, content_width, inner.height);
        let selection = if self.ui.scroll_system.selection.area == SelectionArea::Messages {
            self.ui.scroll_system.selection.normalized()
        } else {
            None
        };
        let selection_background = self.ui.theme.selection_bg_color;
        let selection_foreground = self.ui.theme.selection_fg_color;

        self.ui.markdown_cache.check_width(wrap_width);
        let mut rendered_markdown: Vec<Option<Arc<RenderedMarkdown>>> =
            Vec::with_capacity(self.runtime.chat.messages.len());
        let mut message_heights = Vec::with_capacity(self.runtime.chat.messages.len());

        for (role, content) in &self.runtime.chat.messages {
            if role == "assistant" {
                let rendered = self.ui.markdown_cache.get_or_render_with_links(
                    content,
                    hash_content(content),
                    wrap_width,
                    &self.ui.theme,
                );
                message_heights.push(rendered.lines.len());
                rendered_markdown.push(Some(rendered));
            } else {
                let height = content
                    .lines()
                    .map(|line| {
                        if line.is_empty() {
                            1
                        } else {
                            wrap_line(line, wrap_width).len()
                        }
                    })
                    .sum();
                message_heights.push(height);
                rendered_markdown.push(None);
            }
        }

        let display_list = DisplayList::build(
            &self.runtime.chat.messages,
            |message_index, _, _| message_heights[message_index],
            |block_type, index| self.stream_block_height(block_type, index, content_width),
        );
        let scroll = self.ui.scroll_system.scroll.offset;
        let effective_scroll = scroll.min(u16::MAX as usize);
        let mut visible_assistant_offsets = Vec::new();

        // A single clear happens before any item draws. No later overlay can
        // expose stale cells from a previous frame.
        clear_area(frame.buffer_mut(), inner, self.ui.theme.bg_color);

        for item in &display_list.items {
            match item.kind {
                DisplayItemKind::Message { message_index } => self.render_message_item(
                    frame,
                    content_rect,
                    item,
                    message_index,
                    effective_scroll,
                    selection,
                    selection_background,
                    selection_foreground,
                    &rendered_markdown,
                    &mut visible_assistant_offsets,
                ),
                DisplayItemKind::Block { block_type, index } => self.render_block_item(
                    frame,
                    content_rect,
                    item,
                    block_type,
                    index,
                    effective_scroll,
                ),
            }
        }

        for (message_index, base_line) in visible_assistant_offsets {
            let Some(Some(rendered)) = rendered_markdown.get(message_index) else {
                continue;
            };
            if rendered.links.is_empty() {
                continue;
            }
            apply_hyperlinks(
                frame.buffer_mut(),
                content_rect,
                &rendered.links,
                effective_scroll,
                base_line,
            );
            if let Some(hovered) = &self.ui.scroll_system.hover.message_link {
                if hovered.msg_idx == message_index {
                    apply_link_hover_style(
                        frame.buffer_mut(),
                        content_rect,
                        &rendered.links,
                        Some(hovered),
                        effective_scroll,
                        base_line,
                        self.ui.theme.link_color,
                    );
                }
            }
        }

        let scrollbar_gap = Rect::new(
            inner.x.saturating_add(content_width),
            inner.y,
            SCROLLBAR_GAP,
            inner.height,
        );
        clear_area(frame.buffer_mut(), scrollbar_gap, self.ui.theme.bg_color);

        for terminal in &mut self.runtime.blocks.terminal {
            terminal.resize_to_width(content_width);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_message_item(
        &self,
        frame: &mut Frame,
        viewport: Rect,
        item: &DisplayItem,
        message_index: usize,
        scroll: usize,
        selection: Option<((usize, usize), (usize, usize))>,
        selection_background: Color,
        selection_foreground: Color,
        rendered_markdown: &[Option<Arc<RenderedMarkdown>>],
        visible_assistant_offsets: &mut Vec<(usize, usize)>,
    ) {
        let Some((visible_area, clip_top, _)) = visible_item_area(item, viewport, scroll) else {
            return;
        };
        let Some((role, content)) = self.runtime.chat.messages.get(message_index) else {
            return;
        };
        let mut lines = Vec::with_capacity(item.height);

        if role == "assistant" {
            let Some(Some(rendered)) = rendered_markdown.get(message_index) else {
                return;
            };
            visible_assistant_offsets.push((message_index, item.line_start));
            for (local_line, markdown_line) in rendered.lines.iter().enumerate() {
                let line = if local_line == 0 {
                    let mut spans = vec![Span::styled(
                        ASSISTANT_SYMBOL,
                        Style::default().fg(self.ui.theme.accent_color),
                    )];
                    spans.extend(markdown_line.spans.clone());
                    Line::from(spans)
                } else {
                    markdown_line.clone()
                };
                lines.push(apply_selection_to_rendered_line(
                    line,
                    item.line_start + local_line,
                    selection,
                    selection_background,
                    selection_foreground,
                ));
            }
        } else {
            let content_color = match role.as_str() {
                "user" => self.ui.theme.user_msg_color,
                "system" => self.ui.theme.system_msg_color,
                _ => self.ui.theme.text_color,
            };
            let hovered_file_ref = self.ui.scroll_system.hover.message_file_ref.as_ref();
            let mut first_user_line = role == "user";

            for source_line in content.lines() {
                if source_line.is_empty() {
                    lines.push(Line::from(""));
                    continue;
                }
                for wrapped in wrap_line(
                    source_line,
                    (viewport.width as usize).saturating_sub(SYMBOL_WIDTH),
                ) {
                    let global_line = item.line_start + lines.len();
                    let content_line = if role == "user" {
                        style_user_line_with_file_refs(
                            &wrapped,
                            UserLineFileRefOptions {
                                line_idx: global_line,
                                selection,
                                base_style: Style::default().fg(content_color),
                                link_color: self.ui.theme.link_color,
                                sel_bg: selection_background,
                                sel_fg: selection_foreground,
                                msg_idx: message_index,
                                hovered_file_ref,
                            },
                        )
                    } else {
                        apply_selection_to_line(
                            wrapped,
                            global_line,
                            selection,
                            Style::default().fg(content_color),
                            selection_background,
                            selection_foreground,
                        )
                    };

                    if first_user_line {
                        first_user_line = false;
                        let mut spans = vec![Span::styled(
                            USER_SYMBOL,
                            Style::default().fg(self.ui.theme.accent_color),
                        )];
                        spans.extend(content_line.spans);
                        lines.push(Line::from(spans));
                    } else {
                        lines.push(content_line);
                    }
                }
            }
        }

        frame.render_widget(Paragraph::new(lines).scroll((clip_top, 0)), visible_area);
    }

    fn render_block_item(
        &self,
        frame: &mut Frame,
        viewport: Rect,
        item: &DisplayItem,
        block_type: BlockType,
        index: usize,
        scroll: usize,
    ) {
        let Some((visible_area, clip_top, clip_bottom)) = visible_item_area(item, viewport, scroll)
        else {
            return;
        };
        let clip = if clip_top > 0 || clip_bottom > 0 {
            Some(ClipContext {
                clip_top,
                clip_bottom,
            })
        } else {
            None
        };
        let buffer = frame.buffer_mut();

        match block_type {
            BlockType::Thinking => {
                if let Some(block) = self.runtime.blocks.thinking.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::Pinch => {
                if let Some(block) = self.runtime.blocks.pinch.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::Bash => {
                if let Some(block) = self.runtime.blocks.bash.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::TerminalPane => {
                if let Some(block) = self.runtime.blocks.terminal.get(index) {
                    let focused = self.runtime.blocks.focused_terminal == Some(index);
                    block.render(visible_area, buffer, &self.ui.theme, focused, clip);
                }
            }
            BlockType::ToolResult => {
                if let Some(block) = self.runtime.blocks.tool_result.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::Read => {
                if let Some(block) = self.runtime.blocks.read.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::Edit => {
                if let Some(block) = self.runtime.blocks.edit.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::Write => {
                if let Some(block) = self.runtime.blocks.write.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::WebSearch => {
                if let Some(block) = self.runtime.blocks.web_search.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::Explore => {
                if let Some(block) = self.runtime.blocks.explore.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
            BlockType::Build => {
                if let Some(block) = self.runtime.blocks.build.get(index) {
                    block.render(visible_area, buffer, &self.ui.theme, false, clip);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_item_area_clips_in_the_same_coordinate_space() {
        let item = DisplayItem {
            line_start: 10,
            height: 8,
            kind: DisplayItemKind::Message { message_index: 0 },
        };
        let viewport = Rect::new(3, 5, 40, 6);

        let (area, clip_top, clip_bottom) =
            visible_item_area(&item, viewport, 13).expect("item should be visible");

        assert_eq!(area, Rect::new(3, 5, 40, 5));
        assert_eq!(clip_top, 3);
        assert_eq!(clip_bottom, 0);
    }

    #[test]
    fn visible_item_area_rejects_rows_outside_the_viewport() {
        let item = DisplayItem {
            line_start: 20,
            height: 4,
            kind: DisplayItemKind::Message { message_index: 0 },
        };

        assert!(visible_item_area(&item, Rect::new(0, 0, 80, 10), 0).is_none());
    }
}
