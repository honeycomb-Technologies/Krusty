use super::*;

impl App {
    /// Handle left mouse click
    pub(super) fn handle_left_click(&mut self, mouse: MouseEvent) {
        let x = mouse.column;
        let y = mouse.row;

        // Check file search toggle button click first
        if self.ui.file_search.visible && self.ui.file_search.is_toggle_button_click(x, y) {
            self.ui.file_search.toggle_mode();
            return;
        }

        // Clear any existing selection first
        self.ui.scroll_system.selection.clear();
        self.ui.scroll_system.scroll.unlock_from_selection();

        // Check if clicking decision prompt options
        if self.ui.decision_prompt.visible {
            if let Some(area) = self.ui.scroll_system.layout.prompt_area {
                if area.contains(Position::new(x, y)) {
                    self.handle_prompt_click(x, y, area);
                    return;
                }
            }
        }

        // Check if clicking toolbar title area
        if let Some(area) = self.ui.scroll_system.layout.toolbar_title_area {
            if area.contains(Position::new(x, y)) {
                self.start_title_edit();
                return;
            }
        }

        // Check scrollbar clicks - jump to position and start drag for continued movement
        if let Some(area) = self.ui.scroll_system.layout.messages_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                // Jump to clicked position
                let clamped_y = y.clamp(area.y, area.y + area.height.saturating_sub(1));
                let relative_y = clamped_y.saturating_sub(area.y) as f32;
                let height = (area.height.saturating_sub(1)).max(1) as f32;
                let new_offset = ((relative_y / height)
                    * self.ui.scroll_system.scroll.max_scroll as f32)
                    .round() as usize;
                self.ui.scroll_system.scroll.scroll_to_line(new_offset);

                // Start drag from new position for continued movement
                let drag = ScrollbarDrag::new(
                    y,
                    new_offset,
                    area,
                    self.ui.scroll_system.scroll.max_scroll,
                );
                self.ui.scroll_system.layout.dragging_scrollbar = Some(DragTarget::Messages(drag));
                return;
            }
        }

        if let Some(area) = self.ui.scroll_system.layout.input_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                let total_lines = self.ui.input.get_wrapped_lines_count();
                let visible_lines = self.ui.input.get_max_visible_lines() as usize;
                let max_offset = total_lines.saturating_sub(visible_lines);

                // Jump to clicked position
                let clamped_y = y.clamp(area.y, area.y + area.height.saturating_sub(1));
                let relative_y = clamped_y.saturating_sub(area.y) as f32;
                let height = (area.height.saturating_sub(1)).max(1) as f32;
                let new_offset = ((relative_y / height) * max_offset as f32).round() as usize;
                self.ui
                    .input
                    .set_viewport_offset(new_offset.min(max_offset));

                // Start drag from new position for continued movement
                let drag = ScrollbarDrag::new(y, new_offset, area, max_offset);
                self.ui.scroll_system.layout.dragging_scrollbar = Some(DragTarget::Input(drag));
                return;
            }
        }

        if let Some(area) = self.ui.scroll_system.layout.plan_sidebar_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                self.ui.scroll_system.layout.dragging_scrollbar = Some(DragTarget::PlanSidebar);
                return;
            }
        }

        // Check plugin divider click (for resize dragging)
        if let Some(area) = self.ui.scroll_system.layout.plugin_divider_area {
            if area.contains(Position::new(x, y)) {
                self.ui.scroll_system.layout.dragging_scrollbar = Some(DragTarget::PluginDivider {
                    start_y: y,
                    start_position: self.ui.plugin_window.divider_position,
                });
                return;
            }
        }

        // Check plugin window scrollbar click
        if let Some(area) = self.ui.scroll_system.layout.plugin_window_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                self.ui.scroll_system.layout.dragging_scrollbar = Some(DragTarget::PluginWindow);
                return;
            }
        }

        // Check plugin window click (for plugin switcher or content interaction)
        if let Some(area) = self.ui.scroll_system.layout.plugin_window_area {
            if area.contains(Position::new(x, y)) {
                // Focus the plugin window on click
                self.ui.plugin_window.focus();

                // Check if clicking on switcher area (bottom line)
                let switcher_y = area.y + area.height - 2;
                if y == switcher_y {
                    // Check if clicking left arrow (prev) or right arrow (next)
                    let center_x = area.x + area.width / 2;
                    if x < center_x {
                        self.ui.plugin_window.prev_plugin();
                    } else {
                        self.ui.plugin_window.next_plugin();
                    }
                    // Save active plugin to preferences
                    if let (Some(prefs), Some(id)) = (
                        &self.services.preferences,
                        &self.ui.plugin_window.active_plugin_id,
                    ) {
                        let _ = prefs.set_active_plugin(id);
                    }
                    return;
                }

                // Pass event to plugin if it handles clicks
                if let Some(plugin) = self.ui.plugin_window.active_plugin_mut() {
                    use crate::tui::plugins::PluginEventResult;
                    let event = crossterm::event::Event::Mouse(mouse);
                    match plugin.handle_event(&event, area) {
                        PluginEventResult::Consumed => return,
                        PluginEventResult::Ignored => {}
                    }
                }
                return;
            }
        }

        // Clicking elsewhere unfocuses plugin window
        if self.ui.plugin_window.focused {
            self.ui.plugin_window.unfocus();
        }

        // Check block clicks
        if self.handle_block_click(mouse, x, y) {
            return;
        }

        // Clicking elsewhere clears terminal focus
        if self.runtime.blocks.focused_terminal.is_some() {
            self.runtime.blocks.clear_all_terminal_focus();
        }

        // Check for file reference click (before text selection)
        if self.try_open_file_preview(x, y) {
            return;
        }

        // Check for hyperlink click
        if self.try_open_link(x, y) {
            return;
        }

        // Start text selection
        if let Some(pos) = self.hit_test_messages(x, y) {
            self.ui.scroll_system.selection.start = Some(pos);
            self.ui.scroll_system.selection.end = Some(pos);
            self.ui.scroll_system.selection.is_selecting = true;
            self.ui.scroll_system.selection.area = SelectionArea::Messages;
            self.ui.scroll_system.scroll.lock_for_selection();
            return;
        }

        if let Some(pos) = self.hit_test_input(x, y) {
            // Check for input file reference click first
            if let Some(area) = self.ui.scroll_system.layout.input_area {
                let relative_x = x.saturating_sub(area.x);
                let relative_y = y.saturating_sub(area.y);
                if let Some((_start, _end, path)) =
                    self.ui.input.get_file_ref_at_click(relative_x, relative_y)
                {
                    self.ui.popups.file_preview.open(path);
                    self.ui.popup = Popup::FilePreview;
                    return;
                }
                self.ui.input.handle_click(relative_x, relative_y);
            }
            self.ui.scroll_system.selection.start = Some(pos);
            self.ui.scroll_system.selection.end = Some(pos);
            self.ui.scroll_system.selection.is_selecting = true;
            self.ui.scroll_system.selection.area = SelectionArea::Input;
            self.ui.scroll_system.scroll.lock_for_selection();
        }
    }

    /// Handle click in decision prompt area
    pub(super) fn handle_prompt_click(&mut self, _x: u16, y: u16, area: ratatui::layout::Rect) {
        // Layout: border (1) + question (1) + blank (1) + options...
        // Options start at area.y + 3
        let options_start_y = area.y + 3;

        // Get question info without holding borrow
        let (option_count, multi_select) = match self.ui.decision_prompt.current_question() {
            Some(q) => (q.options.len(), q.multi_select),
            None => return,
        };

        if option_count == 0 {
            return;
        }

        // Check if click is in options area
        if y >= options_start_y {
            let option_idx = (y - options_start_y) as usize;

            if option_idx < option_count {
                // Select this option
                self.ui.decision_prompt.selected_option = option_idx;

                // For multi-select, toggle; for single-select, confirm immediately
                if multi_select {
                    self.ui.decision_prompt.toggle_current();
                } else {
                    // Confirm selection and handle completion
                    let all_done = self.ui.decision_prompt.confirm_selection();
                    if all_done {
                        self.handle_decision_prompt_complete();
                    }
                }
            }
        }
    }
}
