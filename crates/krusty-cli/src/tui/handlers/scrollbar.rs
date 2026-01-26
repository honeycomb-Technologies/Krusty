//! Scrollbar handling
//!
//! Handles scrollbar click and drag operations for messages, input, and block scrollbars.

use crate::tui::app::App;
use crate::tui::blocks::{BlockType, SimpleScrollable, WidthScrollable};
use crate::tui::state::DragTarget;

impl App {
    /// Handle plan sidebar scrollbar click - jump to position
    pub fn handle_plan_sidebar_scrollbar_click(
        &mut self,
        click_y: u16,
        area: ratatui::layout::Rect,
    ) {
        self.plan_sidebar.handle_scrollbar_click(click_y, area);
    }

    /// Handle scrollbar drag - routes to appropriate scrollbar based on drag target
    ///
    /// Returns true if a scrollbar drag was handled.
    pub fn handle_scrollbar_drag(&mut self, y: u16) -> bool {
        match self.scroll_system.layout.dragging_scrollbar {
            Some(DragTarget::Messages(drag)) => {
                let new_offset = drag.calculate_offset(y);
                self.scroll_system.scroll.scroll_to_line(new_offset);
                true
            }
            Some(DragTarget::Input(drag)) => {
                let new_offset = drag.calculate_offset(y);
                self.input.set_viewport_offset(new_offset);
                true
            }
            Some(DragTarget::PlanSidebar) => {
                if let Some(area) = self.scroll_system.layout.plan_sidebar_scrollbar_area {
                    self.handle_plan_sidebar_scrollbar_click(y, area);
                }
                true
            }
            Some(DragTarget::PluginWindow) => {
                if let Some(area) = self.scroll_system.layout.plugin_window_area {
                    let visible_height = area.height.saturating_sub(2) as usize;
                    // Jump to position based on y
                    let relative_y = y.saturating_sub(area.y) as f32;
                    let height = area.height as f32;
                    let max_offset = self
                        .plugin_window
                        .total_lines
                        .saturating_sub(visible_height);
                    let new_offset = ((relative_y / height) * max_offset as f32).round() as usize;
                    self.plugin_window.scroll_offset = new_offset.min(max_offset);
                }
                true
            }
            Some(DragTarget::PluginDivider {
                start_y,
                start_position,
            }) => {
                // Calculate new divider position based on drag
                // Use combined height of plan + divider + plugin for accurate 1:1 drag feel
                let total_height = match (
                    self.scroll_system.layout.plan_sidebar_area,
                    self.scroll_system.layout.plugin_window_area,
                ) {
                    (Some(plan), Some(plugin)) => plan.height + 1 + plugin.height, // +1 for divider
                    (Some(plan), None) => plan.height,
                    (None, Some(plugin)) => plugin.height,
                    (None, None) => 1, // Avoid division by zero
                };

                let delta_y = y as i16 - start_y as i16;
                let position_delta = delta_y as f32 / total_height as f32;
                self.plugin_window.divider_position =
                    (start_position + position_delta).clamp(0.2, 0.8);
                true
            }
            Some(DragTarget::Block(drag)) => {
                if let Some(offset) = drag.calculate_offset(y) {
                    match drag.block_type {
                        BlockType::Thinking => {
                            if let Some(block) = self.blocks.thinking.get_mut(drag.index) {
                                block.set_scroll_offset(offset);
                            }
                        }
                        BlockType::ToolResult => {
                            if let Some(block) = self.blocks.tool_result.get_mut(drag.index) {
                                block.set_scroll_offset(offset);
                            }
                        }
                        BlockType::Bash => {
                            if let Some(block) = self.blocks.bash.get_mut(drag.index) {
                                block.set_scroll_offset(offset);
                            }
                        }
                        BlockType::Read => {
                            if let Some(block) = self.blocks.read.get_mut(drag.index) {
                                block.set_scroll_offset(offset);
                            }
                        }
                        BlockType::Edit => {
                            if let Some(block) = self.blocks.edit.get_mut(drag.index) {
                                block.set_scroll_offset(offset);
                            }
                        }
                        BlockType::Write => {
                            if let Some(block) = self.blocks.write.get_mut(drag.index) {
                                block.set_scroll_offset(offset);
                            }
                        }
                        BlockType::WebSearch => {
                            if let Some(block) = self.blocks.web_search.get_mut(drag.index) {
                                block.set_scroll_offset(offset);
                            }
                        }
                        BlockType::TerminalPane => {
                            // Terminal panes don't use this scrollbar system
                        }
                        BlockType::Explore => {
                            // Explore blocks don't use this scrollbar system
                        }
                        BlockType::Build => {
                            // Build blocks don't use this scrollbar system
                        }
                    }
                }
                true
            }
            None => false,
        }
    }
}
