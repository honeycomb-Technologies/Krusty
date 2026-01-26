//! Mouse event handling
//!
//! Handles mouse clicks, scrolling, drag operations, and text selection.
//! Scrollbar logic is in scrollbar.rs, selection logic is in selection.rs.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use crate::tui::app::{App, Popup};
use crate::tui::blocks::{BlockType, ClipContext, StreamBlock};
use crate::tui::state::{BlockScrollbarDrag, DragTarget, ScrollbarDrag, SelectionArea};

/// Extract clip values from optional ClipContext
fn extract_clip(clip: Option<ClipContext>) -> (u16, u16) {
    clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0))
}

/// Create a BlockScrollbarDrag for a block
fn make_block_scrollbar_drag(
    block_type: BlockType,
    index: usize,
    block_area: ratatui::layout::Rect,
    clip: Option<ClipContext>,
    total_lines: u16,
    visible_lines: u16,
) -> BlockScrollbarDrag {
    let (clip_top, clip_bottom) = extract_clip(clip);
    let header_lines = if clip_top == 0 { 1u16 } else { 0 };
    let footer_lines = if clip_bottom == 0 { 1u16 } else { 0 };
    let scrollbar_height = block_area
        .height
        .saturating_sub(header_lines + footer_lines);
    let scrollbar_y = block_area.y + header_lines;

    BlockScrollbarDrag {
        block_type,
        index,
        scrollbar_y,
        scrollbar_height,
        total_lines,
        visible_lines,
    }
}

/// Scroll direction for routing
#[derive(Clone, Copy)]
enum ScrollDirection {
    Up,
    Down,
}

impl App {
    /// Handle mouse events for scrolling, clicking, and selection
    pub fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.handle_scroll(mouse.column, mouse.row, ScrollDirection::Down);
            }
            MouseEventKind::ScrollUp => {
                self.handle_scroll(mouse.column, mouse.row, ScrollDirection::Up);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_left_click(mouse);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.handle_drag(mouse.column, mouse.row);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.handle_mouse_up();
            }
            MouseEventKind::Moved => {
                self.scroll_system.scroll.unlock_from_messages();
                self.update_hover_state(mouse.column, mouse.row);
            }
            _ => {}
        }
    }

    /// Handle scroll in either direction
    fn handle_scroll(&mut self, x: u16, y: u16, direction: ScrollDirection) {
        let mouse_event_kind = match direction {
            ScrollDirection::Up => MouseEventKind::ScrollUp,
            ScrollDirection::Down => MouseEventKind::ScrollDown,
        };

        // Check if over input area
        if let Some(area) = self.scroll_system.layout.input_area {
            if area.contains(Position::new(x, y)) {
                self.scroll_system.scroll.unlock_from_messages();
                match direction {
                    ScrollDirection::Up => self.input.scroll_up(),
                    ScrollDirection::Down => self.input.scroll_down(),
                }
                return;
            }
        }

        // Check if over plan sidebar
        if let Some(area) = self.scroll_system.layout.plan_sidebar_area {
            if area.contains(Position::new(x, y)) {
                let visible_height = area.height.saturating_sub(2) as usize;
                match direction {
                    ScrollDirection::Up => self.plan_sidebar.scroll_up(),
                    ScrollDirection::Down => self.plan_sidebar.scroll_down(visible_height),
                }
                return;
            }
        }

        // Check if over plugin window
        if let Some(area) = self.scroll_system.layout.plugin_window_area {
            if area.contains(Position::new(x, y)) {
                // First, pass scroll event to the active plugin (for volume control, etc.)
                if let Some(plugin) = self.plugin_window.active_plugin_mut() {
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
                    ScrollDirection::Up => self.plugin_window.scroll_up(),
                    ScrollDirection::Down => self.plugin_window.scroll_down(visible_height),
                }
                return;
            }
        }

        // Check if over pinned terminal at top
        if let (Some(pinned_idx), Some(pinned_area)) = (
            self.blocks.pinned_terminal,
            self.scroll_system.layout.pinned_terminal_area,
        ) {
            if pinned_area.contains(Position::new(x, y)) {
                if let Some(tp) = self.blocks.terminal.get_mut(pinned_idx) {
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
        if !self.scroll_system.scroll.is_locked_to_messages()
            && !self.scroll_system.scroll.is_locked_for_selection()
            && self.route_scroll_to_block(x, y, direction)
        {
            return;
        }

        // Check if over messages area
        if let Some(area) = self.scroll_system.layout.messages_area {
            if area.contains(Position::new(x, y)) {
                self.scroll_system.scroll.lock_to_messages();
                let scroll_amount = (area.height as usize / 10).clamp(3, 10);
                match direction {
                    ScrollDirection::Up => self.scroll_system.scroll.scroll_up(scroll_amount),
                    ScrollDirection::Down => self.scroll_system.scroll.scroll_down(scroll_amount),
                }
            }
        }
    }

    /// Handle left mouse click
    fn handle_left_click(&mut self, mouse: MouseEvent) {
        let x = mouse.column;
        let y = mouse.row;

        // Check file search toggle button click first
        if self.file_search.visible && self.file_search.is_toggle_button_click(x, y) {
            self.file_search.toggle_mode();
            return;
        }

        // Clear any existing selection first
        self.scroll_system.selection.clear();
        self.scroll_system.scroll.unlock_from_selection();

        // Check if clicking decision prompt options
        if self.decision_prompt.visible {
            if let Some(area) = self.scroll_system.layout.prompt_area {
                if area.contains(Position::new(x, y)) {
                    self.handle_prompt_click(x, y, area);
                    return;
                }
            }
        }

        // Check if clicking toolbar title area
        if let Some(area) = self.scroll_system.layout.toolbar_title_area {
            if area.contains(Position::new(x, y)) {
                self.start_title_edit();
                return;
            }
        }

        // Check scrollbar clicks - jump to position and start drag for continued movement
        if let Some(area) = self.scroll_system.layout.messages_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                // Jump to clicked position
                let clamped_y = y.clamp(area.y, area.y + area.height.saturating_sub(1));
                let relative_y = clamped_y.saturating_sub(area.y) as f32;
                let height = (area.height.saturating_sub(1)).max(1) as f32;
                let new_offset = ((relative_y / height)
                    * self.scroll_system.scroll.max_scroll as f32)
                    .round() as usize;
                self.scroll_system.scroll.scroll_to_line(new_offset);

                // Start drag from new position for continued movement
                let drag =
                    ScrollbarDrag::new(y, new_offset, area, self.scroll_system.scroll.max_scroll);
                self.scroll_system.layout.dragging_scrollbar = Some(DragTarget::Messages(drag));
                return;
            }
        }

        if let Some(area) = self.scroll_system.layout.input_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                let total_lines = self.input.get_wrapped_lines_count();
                let visible_lines = self.input.get_max_visible_lines() as usize;
                let max_offset = total_lines.saturating_sub(visible_lines);

                // Jump to clicked position
                let clamped_y = y.clamp(area.y, area.y + area.height.saturating_sub(1));
                let relative_y = clamped_y.saturating_sub(area.y) as f32;
                let height = (area.height.saturating_sub(1)).max(1) as f32;
                let new_offset = ((relative_y / height) * max_offset as f32).round() as usize;
                self.input.set_viewport_offset(new_offset.min(max_offset));

                // Start drag from new position for continued movement
                let drag = ScrollbarDrag::new(y, new_offset, area, max_offset);
                self.scroll_system.layout.dragging_scrollbar = Some(DragTarget::Input(drag));
                return;
            }
        }

        if let Some(area) = self.scroll_system.layout.plan_sidebar_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                self.scroll_system.layout.dragging_scrollbar = Some(DragTarget::PlanSidebar);
                return;
            }
        }

        // Check plugin divider click (for resize dragging)
        if let Some(area) = self.scroll_system.layout.plugin_divider_area {
            if area.contains(Position::new(x, y)) {
                self.scroll_system.layout.dragging_scrollbar = Some(DragTarget::PluginDivider {
                    start_y: y,
                    start_position: self.plugin_window.divider_position,
                });
                return;
            }
        }

        // Check plugin window scrollbar click
        if let Some(area) = self.scroll_system.layout.plugin_window_scrollbar_area {
            if area.contains(Position::new(x, y)) {
                self.scroll_system.layout.dragging_scrollbar = Some(DragTarget::PluginWindow);
                return;
            }
        }

        // Check plugin window click (for plugin switcher or content interaction)
        if let Some(area) = self.scroll_system.layout.plugin_window_area {
            if area.contains(Position::new(x, y)) {
                // Focus the plugin window on click
                self.plugin_window.focus();

                // Check if clicking on switcher area (bottom line)
                let switcher_y = area.y + area.height - 2;
                if y == switcher_y {
                    // Check if clicking left arrow (prev) or right arrow (next)
                    let center_x = area.x + area.width / 2;
                    if x < center_x {
                        self.plugin_window.prev_plugin();
                    } else {
                        self.plugin_window.next_plugin();
                    }
                    // Save active plugin to preferences
                    if let (Some(prefs), Some(id)) = (
                        &self.services.preferences,
                        &self.plugin_window.active_plugin_id,
                    ) {
                        let _ = prefs.set_active_plugin(id);
                    }
                    return;
                }

                // Pass event to plugin if it handles clicks
                if let Some(plugin) = self.plugin_window.active_plugin_mut() {
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
        if self.plugin_window.focused {
            self.plugin_window.unfocus();
        }

        // Check block clicks
        if self.handle_block_click(mouse, x, y) {
            return;
        }

        // Clicking elsewhere clears terminal focus
        if self.blocks.focused_terminal.is_some() {
            self.blocks.clear_all_terminal_focus();
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
            self.scroll_system.selection.start = Some(pos);
            self.scroll_system.selection.end = Some(pos);
            self.scroll_system.selection.is_selecting = true;
            self.scroll_system.selection.area = SelectionArea::Messages;
            self.scroll_system.scroll.lock_for_selection();
            return;
        }

        if let Some(pos) = self.hit_test_input(x, y) {
            // Check for input file reference click first
            if let Some(area) = self.scroll_system.layout.input_area {
                let relative_x = x.saturating_sub(area.x);
                let relative_y = y.saturating_sub(area.y);
                if let Some((_start, _end, path)) =
                    self.input.get_file_ref_at_click(relative_x, relative_y)
                {
                    self.popups.file_preview.open(path);
                    self.ui.popup = Popup::FilePreview;
                    return;
                }
                self.input.handle_click(relative_x, relative_y);
            }
            self.scroll_system.selection.start = Some(pos);
            self.scroll_system.selection.end = Some(pos);
            self.scroll_system.selection.is_selecting = true;
            self.scroll_system.selection.area = SelectionArea::Input;
            self.scroll_system.scroll.lock_for_selection();
        }
    }

    /// Handle block click events - returns true if a block was clicked
    /// Uses single hit_test_any_block call for performance
    fn handle_block_click(&mut self, mouse: MouseEvent, x: u16, y: u16) -> bool {
        use crate::tui::blocks::{BlockEvent, EventResult};

        // Check pinned terminal first (separate area, not in messages)
        if let (Some(pinned_idx), Some(pinned_area)) = (
            self.blocks.pinned_terminal,
            self.scroll_system.layout.pinned_terminal_area,
        ) {
            if pinned_area.contains(Position::new(x, y)) {
                self.blocks.clear_all_terminal_focus();
                if let Some(tp) = self.blocks.terminal.get_mut(pinned_idx) {
                    let event = crossterm::event::Event::Mouse(mouse);
                    let result = tp.handle_event(&event, pinned_area, None);
                    match result {
                        EventResult::Action(BlockEvent::Close) => {
                            self.close_terminal(pinned_idx);
                        }
                        EventResult::Action(BlockEvent::RequestFocus) => {
                            self.blocks.focus_terminal(pinned_idx);
                        }
                        EventResult::Action(BlockEvent::Pinned(is_pinned)) => {
                            if is_pinned {
                                self.blocks.pinned_terminal = Some(pinned_idx);
                            } else {
                                self.blocks.pinned_terminal = None;
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

        match hit.block_type {
            BlockType::Thinking => {
                if let Some(block) = self.blocks.thinking.get_mut(idx) {
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
                        self.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                    }
                    block.handle_event(&event, block_area, clip);
                }
            }
            BlockType::ToolResult => {
                if let Some(block) = self.blocks.tool_result.get_mut(idx) {
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
                        self.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                    }
                    block.handle_event(&event, block_area, clip);
                }
            }
            BlockType::Read => {
                if let Some(block) = self.blocks.read.get_mut(idx) {
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
                        self.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                    }
                    block.handle_event(&event, block_area, clip);
                }
            }
            BlockType::Edit => {
                if let Some(block) = self.blocks.edit.get_mut(idx) {
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
                        self.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                    }
                    let result = block.handle_event(&event, block_area, clip);
                    if let EventResult::Action(BlockEvent::ToggleDiffMode) = result {
                        self.blocks.diff_mode.toggle();
                        let new_mode = self.blocks.diff_mode;
                        for eb in &mut self.blocks.edit {
                            eb.set_diff_mode(new_mode);
                        }
                    }
                }
            }
            BlockType::Write => {
                if let Some(block) = self.blocks.write.get_mut(idx) {
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
                            self.scroll_system.layout.dragging_scrollbar =
                                Some(DragTarget::Block(drag));
                        }
                    }
                    block.handle_event(&event, block_area, clip);
                }
            }
            BlockType::WebSearch => {
                if let Some(block) = self.blocks.web_search.get_mut(idx) {
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
                            self.scroll_system.layout.dragging_scrollbar =
                                Some(DragTarget::Block(drag));
                        }
                    }
                    block.handle_event(&event, block_area, clip);
                }
            }
            BlockType::Bash => {
                self.blocks.clear_all_terminal_focus();
                if let Some(block) = self.blocks.bash.get_mut(idx) {
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
                        self.scroll_system.layout.dragging_scrollbar =
                            Some(DragTarget::Block(drag));
                    }
                    block.handle_event(&event, block_area, clip);
                }
            }
            BlockType::TerminalPane => {
                self.blocks.clear_all_terminal_focus();
                if let Some(tp) = self.blocks.terminal.get_mut(idx) {
                    let result = tp.handle_event(&event, block_area, clip);
                    match result {
                        EventResult::Action(BlockEvent::Close) => {
                            self.close_terminal(idx);
                        }
                        EventResult::Action(BlockEvent::RequestFocus) => {
                            self.blocks.focus_terminal(idx);
                        }
                        EventResult::Action(BlockEvent::Pinned(is_pinned)) => {
                            if is_pinned {
                                if let Some(prev_pinned) = self.blocks.pinned_terminal {
                                    if prev_pinned != idx {
                                        if let Some(prev_tp) =
                                            self.blocks.terminal.get_mut(prev_pinned)
                                        {
                                            prev_tp.set_pinned(false);
                                        }
                                    }
                                }
                                self.blocks.pinned_terminal = Some(idx);
                            } else {
                                self.blocks.pinned_terminal = None;
                            }
                        }
                        _ => {}
                    }
                }
            }
            BlockType::Explore | BlockType::Build => {
                // These blocks don't have click interaction yet
            }
        }

        true
    }

    /// Handle mouse drag (for scrollbar dragging and text selection)
    fn handle_drag(&mut self, x: u16, y: u16) {
        // Handle scrollbar dragging first (from scrollbar.rs)
        if self.handle_scrollbar_drag(y) {
            return;
        }

        // Handle text selection dragging (from selection.rs)
        self.handle_selection_drag(x, y);
    }

    /// Handle mouse button release
    fn handle_mouse_up(&mut self) {
        self.scroll_system.layout.dragging_scrollbar = None;
        self.scroll_system.edge_scroll.direction = None;

        if self.scroll_system.selection.is_selecting && self.scroll_system.selection.has_selection()
        {
            // Copy to clipboard then clear selection
            self.copy_selection_to_clipboard();
            self.scroll_system.selection.clear();
        } else {
            self.scroll_system.selection.is_selecting = false;
        }

        self.scroll_system.scroll.unlock_from_selection();
    }

    /// Route scroll event to block under cursor
    fn route_scroll_to_block(&mut self, x: u16, y: u16, direction: ScrollDirection) -> bool {
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
                if let Some(block) = self.blocks.thinking.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.tool_result.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.bash.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.read.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.edit.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.write.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.web_search.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.terminal.get_mut(hit.index) {
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
                if let Some(block) = self.blocks.explore.get_mut(hit.index) {
                    if matches!(
                        block.handle_event(&event, hit.area, hit.clip),
                        EventResult::Consumed
                    ) {
                        return true;
                    }
                }
            }
            BlockType::Build => {
                if let Some(block) = self.blocks.build.get_mut(hit.index) {
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

    /// Handle click in decision prompt area
    fn handle_prompt_click(&mut self, _x: u16, y: u16, area: ratatui::layout::Rect) {
        // Layout: border (1) + question (1) + blank (1) + options...
        // Options start at area.y + 3
        let options_start_y = area.y + 3;

        // Get question info without holding borrow
        let (option_count, multi_select) = match self.decision_prompt.current_question() {
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
                self.decision_prompt.selected_option = option_idx;

                // For multi-select, toggle; for single-select, confirm immediately
                if multi_select {
                    self.decision_prompt.toggle_current();
                } else {
                    // Confirm selection and handle completion
                    let all_done = self.decision_prompt.confirm_selection();
                    if all_done {
                        self.handle_decision_prompt_complete();
                    }
                }
            }
        }
    }

    /// Update hover state based on mouse position
    fn update_hover_state(&mut self, x: u16, y: u16) {
        // Always update mouse position (cheap)
        self.scroll_system.hover.mouse_pos = Some((x, y));

        // Check if hovering over plugin divider (cheap check, no throttle needed)
        self.scroll_system.layout.plugin_divider_hovered =
            if let Some(area) = self.scroll_system.layout.plugin_divider_area {
                area.contains(Position::new(x, y))
            } else {
                false
            };

        // Throttle expensive detection operations
        if !self.scroll_system.hover.should_detect() {
            return;
        }

        // Check messages area for file references
        self.scroll_system.hover.message_file_ref = self.detect_message_file_ref(x, y);

        // Check messages area for hyperlinks
        self.scroll_system.hover.message_link = self.detect_message_link(x, y);

        // Check input area for file references
        self.scroll_system.hover.input_file_ref = self.detect_input_file_ref(x, y);
    }

    /// Detect file reference in messages at position
    fn detect_message_file_ref(&self, x: u16, y: u16) -> Option<(usize, String)> {
        use regex::Regex;
        use std::sync::LazyLock;

        static FILE_REF_PATTERN: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\[(Image|PDF): ([^\]]+)\]").unwrap());

        let area = self.scroll_system.layout.messages_area?;
        if !area.contains(Position::new(x, y)) {
            return None;
        }

        let (line_idx, _col) = self.hit_test_messages(x, y)?;

        let wrap_width = area.width.saturating_sub(6) as usize;
        let mut current_line = 0usize;

        for (msg_idx, (role, content)) in self.chat.messages.iter().enumerate() {
            if role == "user" || role == "system" {
                let mut msg_lines = 0usize;
                for line in content.lines() {
                    if line.is_empty() {
                        msg_lines += 1;
                    } else {
                        msg_lines += crate::tui::utils::count_wrapped_lines(line, wrap_width);
                    }
                }
                msg_lines += 1;

                if line_idx >= current_line && line_idx < current_line + msg_lines {
                    if let Some(caps) = FILE_REF_PATTERN.captures(content) {
                        let display_name = caps.get(2).map(|m| m.as_str().to_string())?;
                        return Some((msg_idx, display_name));
                    }
                }
                current_line += msg_lines;
            } else if role == "assistant" {
                let line_count = self.get_markdown_line_count(content, wrap_width);
                current_line += line_count + 1;
            } else {
                current_line += 1;
            }
        }

        None
    }

    /// Detect file reference in input at position
    fn detect_input_file_ref(&self, x: u16, y: u16) -> Option<(usize, usize, std::path::PathBuf)> {
        let area = self.scroll_system.layout.input_area?;
        if !area.contains(Position::new(x, y)) {
            return None;
        }

        // Get byte position from click
        let (_line, _col) = self.hit_test_input(x, y)?;

        // Check if there's a file segment at this position
        self.input
            .get_file_ref_at_click(x.saturating_sub(area.x), y.saturating_sub(area.y))
    }

    /// Detect hyperlink in messages at position
    fn detect_message_link(&self, x: u16, y: u16) -> Option<crate::tui::state::HoveredLink> {
        use crate::tui::state::HoveredLink;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Local hash function matching markdown cache key format
        fn hash_content(s: &str) -> u64 {
            let mut hasher = DefaultHasher::new();
            s.hash(&mut hasher);
            hasher.finish()
        }

        let area = self.scroll_system.layout.messages_area?;
        if !area.contains(Position::new(x, y)) {
            return None;
        }

        let (line_idx, col) = self.hit_test_messages(x, y)?;

        let wrap_width = area.width.saturating_sub(6) as usize;
        let mut current_line = 0usize;

        for (msg_idx, (role, content)) in self.chat.messages.iter().enumerate() {
            if role == "assistant" {
                // Get rendered markdown from cache
                let content_hash = hash_content(content);
                if let Some(rendered) = self.markdown_cache.get_rendered(content_hash, wrap_width) {
                    let msg_line_count = rendered.lines.len();

                    // Check if click is within this message's lines
                    if line_idx >= current_line && line_idx < current_line + msg_line_count {
                        let relative_line = line_idx - current_line;

                        // Check if any link spans contain this position
                        for link in &rendered.links {
                            if link.line == relative_line
                                && col >= link.start_col
                                && col < link.end_col
                            {
                                return Some(HoveredLink {
                                    msg_idx,
                                    line: relative_line,
                                    start_col: link.start_col,
                                    end_col: link.end_col,
                                    url: link.url.clone(),
                                });
                            }
                        }
                    }
                    current_line += msg_line_count + 1; // +1 for blank line
                } else {
                    // Fallback line count
                    let line_count = self.get_markdown_line_count(content, wrap_width);
                    current_line += line_count + 1;
                }
            } else {
                // User/system messages
                let mut msg_lines = 0usize;
                for line in content.lines() {
                    if line.is_empty() {
                        msg_lines += 1;
                    } else {
                        msg_lines += crate::tui::utils::count_wrapped_lines(line, wrap_width);
                    }
                }
                msg_lines += 1;
                current_line += msg_lines;
            }
        }

        None
    }

    /// Try to open a hyperlink at the click position
    /// Returns true if a link was clicked and opened
    fn try_open_link(&mut self, x: u16, y: u16) -> bool {
        if let Some(link) = self.detect_message_link(x, y) {
            // Open the URL in the default browser
            if let Err(e) = webbrowser::open(&link.url) {
                tracing::warn!("Failed to open URL {}: {}", link.url, e);
            }
            return true;
        }
        false
    }

    /// Try to detect and open a file preview from a click position
    /// Returns true if a file reference was clicked and preview opened
    fn try_open_file_preview(&mut self, x: u16, y: u16) -> bool {
        use regex::Regex;
        use std::sync::LazyLock;

        static FILE_REF_PATTERN: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\[(Image|PDF): ([^\]]+)\]").unwrap());

        // Check if click is in messages area
        let Some(area) = self.scroll_system.layout.messages_area else {
            return false;
        };

        if !area.contains(Position::new(x, y)) {
            return false;
        }

        // Get the clicked line position
        let Some((line_idx, _col)) = self.hit_test_messages(x, y) else {
            return false;
        };

        // Find the message content at this line
        let wrap_width = area.width.saturating_sub(6) as usize;
        let mut current_line = 0usize;

        for (role, content) in &self.chat.messages {
            if role == "user" || role == "system" {
                // Calculate lines in this message
                let mut msg_lines = 0usize;
                for line in content.lines() {
                    if line.is_empty() {
                        msg_lines += 1;
                    } else {
                        msg_lines += crate::tui::utils::count_wrapped_lines(line, wrap_width);
                    }
                }
                msg_lines += 1; // blank line

                // Check if clicked line is within this message
                if line_idx >= current_line && line_idx < current_line + msg_lines {
                    // Check for file reference in the content
                    if let Some(caps) = FILE_REF_PATTERN.captures(content) {
                        let display_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");

                        // Look up the file path
                        if let Some(path) = self.attached_files.get(display_name) {
                            // Open the preview popup
                            self.popups.file_preview.open(path.clone());
                            self.ui.popup = Popup::FilePreview;
                            return true;
                        }
                    }
                }

                current_line += msg_lines;
            } else if role == "assistant" {
                let line_count = self.get_markdown_line_count(content, wrap_width);
                current_line += line_count + 1;
            } else {
                // Skip blocks (thinking, bash, etc.)
                current_line += 1;
            }
        }

        false
    }
}
