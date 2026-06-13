use super::*;

impl App {
    /// Handle block click events - returns true if a block was clicked
    /// Uses single hit_test_any_block call for performance
    pub(super) fn handle_block_click(&mut self, mouse: MouseEvent, x: u16, y: u16) -> bool {
        use crate::tui::blocks::{BlockEvent, EventResult};

        // Check pinned terminal first (separate area, not in messages)
        if let (Some(pinned_idx), Some(pinned_area)) = (
            self.runtime.blocks.pinned_terminal,
            self.ui.scroll_system.layout.pinned_terminal_area,
        ) {
            if pinned_area.contains(Position::new(x, y)) {
                self.runtime.blocks.clear_all_terminal_focus();
                if let Some(tp) = self.runtime.blocks.terminal.get_mut(pinned_idx) {
                    let event = crossterm::event::Event::Mouse(mouse);
                    let result = tp.handle_event(&event, pinned_area, None);
                    match result {
                        EventResult::Action(BlockEvent::Close) => {
                            self.close_terminal(pinned_idx);
                        }
                        EventResult::Action(BlockEvent::RequestFocus) => {
                            self.runtime.blocks.focus_terminal(pinned_idx);
                        }
                        EventResult::Action(BlockEvent::Pinned(is_pinned)) => {
                            if is_pinned {
                                self.runtime.blocks.pinned_terminal = Some(pinned_idx);
                            } else {
                                self.runtime.blocks.pinned_terminal = None;
                            }
                        }
                        _ => {}
                    }
                }
                return true;
            }
        }

        // Single hit test for all block types in messages area
        let Some(hit) = self.hit_test_any_block(x, y) else {
            return false;
        };

        let event = crossterm::event::Event::Mouse(mouse);
        let idx = hit.index;
        let block_area = hit.area;
        let clip = hit.clip;

        // Track if we handled something specific (scrollbar drag, button click) vs plain content click
        let mut handled_specific = false;
        let mut event_consumed = false;

        match hit.block_type {
            BlockType::Pinch => {}
            BlockType::Thinking => {
                if let Some(block) = self.runtime.blocks.thinking.get_mut(idx) {
                    let actual_width = block.box_width(block_area.width);
                    let scrollbar_x = block_area.x + actual_width.saturating_sub(2);
                    if !block.is_collapsed()
                        && block.has_scrollbar(block_area.width)
                        && x >= scrollbar_x
                    {
                        let (total_lines, visible_lines, _) =
                            block.get_scroll_info(block_area.width);
                        let drag = make_block_scrollbar_drag(
                            BlockType::Thinking,
                            idx,
                            block_area,
                            clip,
                            total_lines,
                            visible_lines,
                        );
                        self.ui.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                        handled_specific = true;
                    }
                    if let EventResult::Consumed = block.handle_event(&event, block_area, clip) {
                        event_consumed = true;
                    }
                }
            }
            BlockType::ToolResult => {
                if let Some(block) = self.runtime.blocks.tool_result.get_mut(idx) {
                    let actual_width = block.box_width(block_area.width);
                    let scrollbar_x = block_area.x + actual_width.saturating_sub(2);
                    if !block.is_collapsed() && block.has_scrollbar() && x >= scrollbar_x {
                        let (total_lines, visible_lines, _) = block.get_scroll_info();
                        let drag = make_block_scrollbar_drag(
                            BlockType::ToolResult,
                            idx,
                            block_area,
                            clip,
                            total_lines,
                            visible_lines,
                        );
                        self.ui.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                        handled_specific = true;
                    }
                    if let EventResult::Consumed = block.handle_event(&event, block_area, clip) {
                        event_consumed = true;
                    }
                }
            }
            BlockType::Read => {
                if let Some(block) = self.runtime.blocks.read.get_mut(idx) {
                    let actual_width = block.box_width(block_area.width);
                    let scrollbar_x = block_area.x + actual_width.saturating_sub(2);
                    if !block.is_collapsed()
                        && block.has_scrollbar(block_area.width)
                        && x >= scrollbar_x
                    {
                        let (total_lines, visible_lines, _) =
                            block.get_scroll_info(block_area.width);
                        let drag = make_block_scrollbar_drag(
                            BlockType::Read,
                            idx,
                            block_area,
                            clip,
                            total_lines,
                            visible_lines,
                        );
                        self.ui.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                        handled_specific = true;
                    }
                    if let EventResult::Consumed = block.handle_event(&event, block_area, clip) {
                        event_consumed = true;
                    }
                }
            }
            BlockType::Edit => {
                if let Some(block) = self.runtime.blocks.edit.get_mut(idx) {
                    if block.needs_scrollbar()
                        && x >= block_area.x + block_area.width.saturating_sub(3)
                    {
                        let (total_lines, visible_lines, _) = block.get_scroll_info();
                        let drag = make_block_scrollbar_drag(
                            BlockType::Edit,
                            idx,
                            block_area,
                            clip,
                            total_lines,
                            visible_lines,
                        );
                        self.ui.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                        handled_specific = true;
                    }
                    let result = block.handle_event(&event, block_area, clip);
                    if let EventResult::Consumed = result {
                        event_consumed = true;
                    }
                    if let EventResult::Action(BlockEvent::ToggleDiffMode) = result {
                        self.runtime.blocks.diff_mode.toggle();
                        let new_mode = self.runtime.blocks.diff_mode;
                        for eb in &mut self.runtime.blocks.edit {
                            eb.set_diff_mode(new_mode);
                        }
                    }
                }
            }
            BlockType::Write => {
                if let Some(block) = self.runtime.blocks.write.get_mut(idx) {
                    let actual_width = block.box_width(block_area.width);
                    let scrollbar_x = block_area.x + actual_width.saturating_sub(2);
                    if !block.is_collapsed() && x >= scrollbar_x {
                        let (total_lines, visible_lines, _) =
                            block.get_scroll_info(block_area.width);
                        if total_lines > visible_lines {
                            let drag = make_block_scrollbar_drag(
                                BlockType::Write,
                                idx,
                                block_area,
                                clip,
                                total_lines,
                                visible_lines,
                            );
                            self.ui.scroll_system.layout.dragging_scrollbar =
                                Some(DragTarget::Block(drag));
                            handled_specific = true;
                        }
                    }
                    if let EventResult::Consumed = block.handle_event(&event, block_area, clip) {
                        event_consumed = true;
                    }
                }
            }
            BlockType::WebSearch => {
                if let Some(block) = self.runtime.blocks.web_search.get_mut(idx) {
                    let actual_width = block.box_width(block_area.width);
                    let scrollbar_x = block_area.x + actual_width.saturating_sub(2);
                    if !block.is_collapsed() && x >= scrollbar_x {
                        let (total_lines, visible_lines, _) = block.get_scroll_info();
                        if total_lines > visible_lines {
                            let drag = make_block_scrollbar_drag(
                                BlockType::WebSearch,
                                idx,
                                block_area,
                                clip,
                                total_lines,
                                visible_lines,
                            );
                            self.ui.scroll_system.layout.dragging_scrollbar =
                                Some(DragTarget::Block(drag));
                            handled_specific = true;
                        }
                    }
                    if let EventResult::Consumed = block.handle_event(&event, block_area, clip) {
                        event_consumed = true;
                    }
                }
            }
            BlockType::Bash => {
                self.runtime.blocks.clear_all_terminal_focus();
                if let Some(block) = self.runtime.blocks.bash.get_mut(idx) {
                    if !block.is_collapsed()
                        && x >= block_area.x + block_area.width.saturating_sub(3)
                    {
                        let (total_lines, visible_lines, _) =
                            block.get_scroll_info(block_area.width);
                        let drag = make_block_scrollbar_drag(
                            BlockType::Bash,
                            idx,
                            block_area,
                            clip,
                            total_lines,
                            visible_lines,
                        );
                        self.ui.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                        handled_specific = true;
                    }
                    if let EventResult::Consumed = block.handle_event(&event, block_area, clip) {
                        event_consumed = true;
                    }
                }
            }
            BlockType::TerminalPane => {
                self.runtime.blocks.clear_all_terminal_focus();
                if let Some(tp) = self.runtime.blocks.terminal.get_mut(idx) {
                    let result = tp.handle_event(&event, block_area, clip);
                    if let EventResult::Consumed = result {
                        event_consumed = true;
                    }
                    match result {
                        EventResult::Action(BlockEvent::Close) => {
                            self.close_terminal(idx);
                            handled_specific = true;
                        }
                        EventResult::Action(BlockEvent::RequestFocus) => {
                            self.runtime.blocks.focus_terminal(idx);
                            handled_specific = true;
                        }
                        EventResult::Action(BlockEvent::Pinned(is_pinned)) => {
                            if is_pinned {
                                if let Some(prev_pinned) = self.runtime.blocks.pinned_terminal {
                                    if prev_pinned != idx {
                                        if let Some(prev_tp) =
                                            self.runtime.blocks.terminal.get_mut(prev_pinned)
                                        {
                                            prev_tp.set_pinned(false);
                                        }
                                    }
                                }
                                self.runtime.blocks.pinned_terminal = Some(idx);
                            } else {
                                self.runtime.blocks.pinned_terminal = None;
                            }
                            handled_specific = true;
                        }
                        _ => {}
                    }
                }
            }
            BlockType::Explore | BlockType::Build => {
                // These blocks don't have click interaction yet
            }
        }

        // Return true only if we handled something specific (scrollbar drag, button click)
        // Return false for plain content clicks to allow text selection
        handled_specific || event_consumed
    }
}
