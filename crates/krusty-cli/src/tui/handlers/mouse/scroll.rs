use super::*;

impl App {
    /// Handle scroll in either direction
    pub(super) fn handle_scroll(&mut self, x: u16, y: u16, direction: ScrollDirection) {
        let mouse_event_kind = match direction {
            ScrollDirection::Up => MouseEventKind::ScrollUp,
            ScrollDirection::Down => MouseEventKind::ScrollDown,
        };

        // Check if over input area
        if let Some(area) = self.ui.scroll_system.layout.input_area {
            if area.contains(Position::new(x, y)) {
                self.ui.scroll_system.scroll.unlock_from_messages();
                match direction {
                    ScrollDirection::Up => self.ui.input.scroll_up(),
                    ScrollDirection::Down => self.ui.input.scroll_down(),
                }
                return;
            }
        }

        // Check if over plan sidebar
        if let Some(area) = self.ui.scroll_system.layout.plan_sidebar_area {
            if area.contains(Position::new(x, y)) {
                let visible_height = area.height.saturating_sub(2) as usize;
                match direction {
                    ScrollDirection::Up => self.ui.plan_sidebar.scroll_up(),
                    ScrollDirection::Down => self.ui.plan_sidebar.scroll_down(visible_height),
                }
                return;
            }
        }

        // Check if over plugin window
        if let Some(area) = self.ui.scroll_system.layout.plugin_window_area {
            if area.contains(Position::new(x, y)) {
                // First, pass scroll event to the active plugin (for volume control, etc.)
                if let Some(plugin) = self.ui.plugin_window.active_plugin_mut() {
                    let mouse_event_kind = match direction {
                        ScrollDirection::Up => MouseEventKind::ScrollUp,
                        ScrollDirection::Down => MouseEventKind::ScrollDown,
                    };
                    let event = crossterm::event::Event::Mouse(MouseEvent {
                        kind: mouse_event_kind,
                        column: x,
                        row: y,
                        modifiers: crossterm::event::KeyModifiers::NONE,
                    });
                    use crate::tui::plugins::PluginEventResult;
                    if matches!(
                        plugin.handle_event(&event, area),
                        PluginEventResult::Consumed
                    ) {
                        return;
                    }
                }

                // Default scroll behavior if plugin didn't handle it
                let visible_height = area.height.saturating_sub(2) as usize;
                match direction {
                    ScrollDirection::Up => self.ui.plugin_window.scroll_up(),
                    ScrollDirection::Down => self.ui.plugin_window.scroll_down(visible_height),
                }
                return;
            }
        }

        // Check if over pinned terminal at top
        if let (Some(pinned_idx), Some(pinned_area)) = (
            self.runtime.blocks.pinned_terminal,
            self.ui.scroll_system.layout.pinned_terminal_area,
        ) {
            if pinned_area.contains(Position::new(x, y)) {
                if let Some(tp) = self.runtime.blocks.terminal.get_mut(pinned_idx) {
                    if !tp.is_collapsed() {
                        let event = crossterm::event::Event::Mouse(MouseEvent {
                            kind: mouse_event_kind,
                            column: x,
                            row: y,
                            modifiers: crossterm::event::KeyModifiers::NONE,
                        });
                        tp.handle_event(&event, pinned_area, None);
                        return;
                    }
                }
            }
        }

        // Route scroll to block if not locked
        if !self.ui.scroll_system.scroll.is_locked_to_messages()
            && !self.ui.scroll_system.scroll.is_locked_for_selection()
            && self.route_scroll_to_block(x, y, direction)
        {
            return;
        }

        // Check if over messages area
        if let Some(area) = self.ui.scroll_system.layout.messages_area {
            if area.contains(Position::new(x, y)) {
                self.ui.scroll_system.scroll.lock_to_messages();
                let scroll_amount = (area.height as usize / 10).clamp(3, 10);
                match direction {
                    ScrollDirection::Up => self.ui.scroll_system.scroll.scroll_up(scroll_amount),
                    ScrollDirection::Down => {
                        self.ui.scroll_system.scroll.scroll_down(scroll_amount)
                    }
                }
            }
        }
    }

    /// Route scroll event to block under cursor
    pub(super) fn route_scroll_to_block(
        &mut self,
        x: u16,
        y: u16,
        direction: ScrollDirection,
    ) -> bool {
        use crate::tui::blocks::EventResult;

        let Some(hit) = self.hit_test_any_block(x, y) else {
            return false;
        };

        let kind = match direction {
            ScrollDirection::Down => MouseEventKind::ScrollDown,
            ScrollDirection::Up => MouseEventKind::ScrollUp,
        };

        let event = crossterm::event::Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });

        // Route to block and check if event was consumed
        // If block returns Ignored (e.g., mouse outside actual bounds), fall through to message scroll
        match hit.block_type {
            BlockType::Thinking => {
                if let Some(block) = self.runtime.blocks.thinking.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && block.has_scrollbar(hit.area.width)
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::ToolResult => {
                if let Some(block) = self.runtime.blocks.tool_result.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && block.has_scrollbar()
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::Bash => {
                if let Some(block) = self.runtime.blocks.bash.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && block.has_scrollbar(hit.area.width)
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::Read => {
                if let Some(block) = self.runtime.blocks.read.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && block.has_scrollbar(hit.area.width)
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::Edit => {
                if let Some(block) = self.runtime.blocks.edit.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && block.needs_scrollbar()
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::Write => {
                if let Some(block) = self.runtime.blocks.write.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::WebSearch => {
                if let Some(block) = self.runtime.blocks.web_search.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::TerminalPane => {
                if let Some(block) = self.runtime.blocks.terminal.get_mut(hit.index) {
                    if !block.is_collapsed()
                        && matches!(
                            block.handle_event(&event, hit.area, hit.clip),
                            EventResult::Consumed
                        )
                    {
                        return true;
                    }
                }
            }
            BlockType::Explore => {
                if let Some(block) = self.runtime.blocks.explore.get_mut(hit.index) {
                    if matches!(
                        block.handle_event(&event, hit.area, hit.clip),
                        EventResult::Consumed
                    ) {
                        return true;
                    }
                }
            }
            BlockType::Build => {
                if let Some(block) = self.runtime.blocks.build.get_mut(hit.index) {
                    if matches!(
                        block.handle_event(&event, hit.area, hit.clip),
                        EventResult::Consumed
                    ) {
                        return true;
                    }
                }
            }
        }

        false
    }
}
