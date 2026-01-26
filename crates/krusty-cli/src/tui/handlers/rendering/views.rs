//! View rendering
//!
//! Renders the main views: start menu and chat.

use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::blocks::StreamBlock;
use crate::tui::components::{
    render_input_scrollbar, render_messages_scrollbar, render_plan_sidebar, render_plugin_window,
    render_status_bar, render_toolbar, MIN_TERMINAL_WIDTH,
};
use crate::tui::state::SelectionArea;

impl App {
    /// Render the start menu view
    pub fn render_start_menu(&mut self, f: &mut Frame) {
        let area = f.area();

        // Calculate input height (max 8 rows of content + 2 for borders)
        let input_lines = self.input.get_wrapped_lines_count().max(1);
        let input_height = (input_lines + 2).min(10) as u16; // 8 content rows + 2 border

        // Layout: toolbar, logo+crab, quick actions, input, status bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),            // Toolbar
                Constraint::Length(15),           // Logo + crab area
                Constraint::Min(8),               // Quick actions
                Constraint::Length(input_height), // Input
                Constraint::Length(1),            // Status bar
            ])
            .split(area);

        // Render toolbar (no title in start menu)
        render_toolbar(
            f,
            chunks[0],
            &self.ui.theme,
            self.ui.work_mode,
            None,
            false,
            "",
            self.is_busy(),
            self.get_plan_info(),
        );

        // Logo area with border
        let logo_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.ui.theme.border_color))
            .style(Style::default().bg(self.ui.theme.bg_color));

        let inner_area = logo_block.inner(chunks[1]);
        f.render_widget(logo_block, chunks[1]);

        // Render bubbles inside logo area
        let bubbles = self
            .menu_animator
            .render_bubbles(inner_area.width, inner_area.height);
        for (x, y, ch, color) in bubbles {
            if x < inner_area.width && y < inner_area.height {
                let abs_x = inner_area.x + x;
                let abs_y = inner_area.y + y;
                if let Some(cell) = f.buffer_mut().cell_mut(Position::new(abs_x, abs_y)) {
                    cell.set_char(ch);
                    cell.set_fg(color);
                }
            }
        }

        // ASCII logo with two colors
        let title_padding = " ".repeat(((inner_area.width as usize).saturating_sub(33)) / 2);
        let logo_text = vec![
            Line::from(vec![
                Span::raw(&title_padding),
                Span::styled(
                    "▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌",
                    Style::default()
                        .fg(self.ui.theme.logo_primary_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw(&title_padding),
                Span::styled(
                    "█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌",
                    Style::default().fg(self.ui.theme.logo_primary_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&title_padding),
                Span::styled(
                    "▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪",
                    Style::default().fg(self.ui.theme.logo_secondary_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&title_padding),
                Span::styled(
                    "▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.",
                    Style::default().fg(self.ui.theme.logo_secondary_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&title_padding),
                Span::styled(
                    "·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ • ",
                    Style::default().fg(self.ui.theme.logo_primary_color),
                ),
            ]),
        ];
        f.render_widget(Paragraph::new(logo_text), inner_area);

        // Render crab at bottom of logo area
        let (crab_frames, crab_x, _crab_y) = self.menu_animator.render_crab();
        let crab_height = crab_frames.len() as u16;
        let crab_y = inner_area.y + inner_area.height.saturating_sub(crab_height);

        let crab_color = self.ui.theme.accent_color;
        let eye_color = self.ui.theme.mode_chat_color;

        for (i, line) in crab_frames.into_iter().enumerate() {
            let y = crab_y + i as u16;
            if y < inner_area.y + inner_area.height {
                // Color eyes pink on the second line
                if i == 1 && (line.contains(" o ") || line.contains("-")) {
                    let mut x_pos = inner_area.x + crab_x.round() as u16;
                    for ch in line.chars() {
                        if x_pos < inner_area.x + inner_area.width {
                            if let Some(cell) = f.buffer_mut().cell_mut(Position::new(x_pos, y)) {
                                if ch == 'o' || ch == '-' {
                                    cell.set_char(ch);
                                    cell.set_fg(eye_color);
                                } else {
                                    cell.set_char(ch);
                                    cell.set_fg(crab_color);
                                }
                            }
                        }
                        x_pos += 1;
                    }
                } else {
                    let mut x_pos = inner_area.x + crab_x.round() as u16;
                    for ch in line.chars() {
                        if x_pos < inner_area.x + inner_area.width {
                            if let Some(cell) = f.buffer_mut().cell_mut(Position::new(x_pos, y)) {
                                cell.set_char(ch);
                                cell.set_fg(crab_color);
                            }
                        }
                        x_pos += 1;
                    }
                }
            }
        }

        // Quick Actions section
        let padding = " ".repeat(((area.width as usize / 2).saturating_sub(25)).max(0));
        let commands_text = vec![
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "Quick Actions:",
                    Style::default()
                        .fg(self.ui.theme.title_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /init   ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Initialize project (KRAB.md)",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /load   ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Load previous session",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /model  ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Select AI model",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /auth   ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Manage API providers",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /theme  ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Change color theme",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /skills ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Browse skills",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /lsp    ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Browse LSP extensions",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /hooks  ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Configure tool hooks",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
            Line::from(vec![
                Span::raw(&padding),
                Span::styled(
                    "  /cmd    ",
                    Style::default().fg(self.ui.theme.accent_color),
                ),
                Span::styled(
                    "Show all controls",
                    Style::default().fg(self.ui.theme.text_color),
                ),
            ]),
        ];

        let commands = Paragraph::new(commands_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(self.ui.theme.border_color))
                .style(Style::default().bg(self.ui.theme.bg_color)),
        );
        f.render_widget(commands, chunks[2]);

        // Input area (full width)
        let input_area = chunks[3];
        self.scroll_system.layout.input_area = Some(input_area);
        self.input
            .set_max_visible_lines(input_area.height.saturating_sub(2));

        // Get input selection if selecting in input area
        let input_selection = if self.scroll_system.selection.area == SelectionArea::Input {
            self.scroll_system.selection.normalized()
        } else {
            None
        };

        // Border color changes to accent when thinking mode enabled (Tab toggle)
        let input_border_color = if self.thinking_enabled {
            self.ui.theme.accent_color
        } else {
            self.ui.theme.border_color
        };
        // Get hover range for file ref highlighting
        let hover_range = self
            .scroll_system
            .hover
            .input_file_ref
            .as_ref()
            .map(|(s, e, _)| (*s, *e));
        let input_widget = self.input.render_styled_with_file_refs(
            input_area,
            self.ui.theme.bg_color,
            input_border_color,
            self.ui.theme.text_color,
            input_selection,
            self.ui.theme.selection_bg_color,
            self.ui.theme.selection_fg_color,
            Some(self.ui.theme.link_color),
            hover_range,
        );
        f.render_widget(input_widget, input_area);

        // Render input scrollbar (always shows track, thumb when content overflows)
        let total_lines = self.input.get_wrapped_lines_count();
        let visible_lines = self.input.get_max_visible_lines() as usize;
        // Scrollbar area is 1 column wide on the right side of input, inside the border
        let scrollbar_area = Rect::new(
            input_area.x + input_area.width - 2,
            input_area.y + 1,
            1,
            input_area.height.saturating_sub(2),
        );
        self.scroll_system.layout.input_scrollbar_area = Some(scrollbar_area);
        render_input_scrollbar(
            f,
            scrollbar_area,
            total_lines,
            visible_lines,
            self.input.get_viewport_offset(),
            &self.ui.theme,
        );

        // Autocomplete popup above input
        if self.autocomplete.visible && self.autocomplete.has_suggestions() {
            let ac_height = 9.min(chunks[2].height.saturating_sub(2));
            let ac_area = Rect::new(
                input_area.x,
                input_area.y.saturating_sub(ac_height),
                input_area.width,
                ac_height,
            );
            self.autocomplete.render(f, ac_area, &self.ui.theme);
        }

        // File search popup above input (mutually exclusive with autocomplete)
        if self.file_search.visible && self.file_search.has_results() && !self.autocomplete.visible
        {
            let fs_height = 12.min(chunks[2].height.saturating_sub(2));
            let fs_area = Rect::new(
                input_area.x,
                input_area.y.saturating_sub(fs_height),
                input_area.width,
                fs_height,
            );
            self.file_search.render(f, fs_area, &self.ui.theme);
        }

        // Status bar (no context tokens in start menu)
        render_status_bar(
            f,
            chunks[4],
            &self.ui.theme,
            &self.current_model,
            &self.working_dir,
            None,
            self.running_process_count,
            self.running_process_elapsed,
        );
    }

    /// Render the chat view
    pub fn render_chat(&mut self, f: &mut Frame) {
        let full_area = f.area();

        // Check if sidebar should be shown (has width and terminal is wide enough)
        // Sidebar is shown if either plan or plugin window is visible
        let sidebar_width = if full_area.width >= MIN_TERMINAL_WIDTH {
            let plan_width = self.plan_sidebar.width();
            let plugin_visible = self.plugin_window.visible;

            if plan_width > 0 || plugin_visible {
                // Use plan sidebar width as the standard, or fallback to constant if only plugin
                if plan_width > 0 {
                    plan_width
                } else {
                    crate::tui::components::plan_sidebar::SIDEBAR_WIDTH
                }
            } else {
                0
            }
        } else {
            0
        };

        // Split horizontally if sidebar is showing
        let (area, sidebar_area) = if sidebar_width > 0 {
            let h_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(40),               // Main content
                    Constraint::Length(sidebar_width), // Sidebar
                ])
                .split(full_area);
            (h_chunks[0], Some(h_chunks[1]))
        } else {
            (full_area, None)
        };

        // Input height: 8 content rows max + 2 for borders
        let input_height = (self.input.get_wrapped_lines_count() as u16 + 2).clamp(3, 10);

        // Decision prompt height (0 if not visible)
        let prompt_height = self.decision_prompt.calculate_height();

        // Check if we have a pinned terminal (height() is O(1) - just returns constant)
        // Use the same width for height calculation as we use for rendering to prevent mismatch
        let pinned_render_width = area.width.saturating_sub(2); // Must match pinned_area.width below
        let pinned_height = self
            .blocks
            .pinned_terminal
            .and_then(|idx| self.blocks.terminal.get(idx))
            .map(|tp| tp.height(pinned_render_width, &self.ui.theme))
            .unwrap_or(0);

        // Layout: toolbar, pinned (0 if none), messages, prompt (0 if none), input, status
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),             // Toolbar
                Constraint::Length(pinned_height), // Pinned terminal (0 if none)
                Constraint::Min(5),                // Messages
                Constraint::Length(prompt_height), // Decision prompt (0 if none)
                Constraint::Length(input_height),  // Input
                Constraint::Length(1),             // Status bar
            ])
            .split(area);

        // Render toolbar with session title (clickable to edit)
        self.scroll_system.layout.toolbar_title_area = render_toolbar(
            f,
            chunks[0],
            &self.ui.theme,
            self.ui.work_mode,
            self.session_title.as_deref(),
            self.title_editor.is_editing,
            &self.title_editor.buffer,
            self.is_busy(),
            self.get_plan_info(),
        );

        // Render pinned terminal if present
        if let Some(pinned_idx) = self.blocks.pinned_terminal {
            if let Some(tp) = self.blocks.terminal.get(pinned_idx) {
                let pinned_area = Rect {
                    x: chunks[1].x + 1,
                    y: chunks[1].y,
                    width: chunks[1].width.saturating_sub(2),
                    height: chunks[1].height,
                };
                self.scroll_system.layout.pinned_terminal_area = Some(pinned_area);
                let is_focused = self.blocks.focused_terminal == Some(pinned_idx);
                tp.render(
                    pinned_area,
                    f.buffer_mut(),
                    &self.ui.theme,
                    is_focused,
                    None,
                );
            }
        } else {
            self.scroll_system.layout.pinned_terminal_area = None;
        }

        // Messages area (chunks[2] now, was chunks[1])
        let messages_chunk = chunks[2];

        // Calculate scroll position BEFORE rendering (fixes 1-frame lag on streaming)
        self.scroll_system.layout.messages_area = Some(messages_chunk);

        // Debounce expensive line calculation during sidebar animation
        // Only recalculate when NOT animating or when width changed significantly
        let msg_total_lines = if self.plan_sidebar.is_animating()
            && self.scroll_system.layout_cache.message_lines > 0
        {
            // During animation: use cached value
            self.scroll_system.layout_cache.message_lines
        } else {
            // Animation complete or no animation: recalculate and cache
            let lines = self.calculate_message_lines(messages_chunk.width);
            self.scroll_system.layout_cache.message_lines = lines;
            self.scroll_system.layout_cache.cached_width = messages_chunk.width;
            lines
        };
        let msg_visible_height = messages_chunk.height.saturating_sub(2);

        // Update max scroll, clamp offset, and handle auto-scroll
        self.scroll_system
            .scroll
            .update_max_scroll(msg_total_lines, msg_visible_height);
        self.scroll_system.scroll.apply_scroll_to_bottom();

        // NOW render messages with correct scroll position
        self.render_messages(f, messages_chunk);

        // Render messages scrollbar (1 column wide)
        let msg_scrollbar_area = Rect::new(
            messages_chunk.x + messages_chunk.width - 2,
            messages_chunk.y + 1,
            1,
            messages_chunk.height.saturating_sub(2),
        );
        self.scroll_system.layout.messages_scrollbar_area = Some(msg_scrollbar_area);
        render_messages_scrollbar(
            f,
            msg_scrollbar_area,
            self.scroll_system.scroll.offset,
            msg_total_lines,
            msg_visible_height as usize,
            &self.ui.theme,
        );

        // Decision prompt (chunks[3])
        if self.decision_prompt.visible {
            let prompt_area = chunks[3];
            self.scroll_system.layout.prompt_area = Some(prompt_area);
            self.decision_prompt
                .render(f.buffer_mut(), prompt_area, &self.ui.theme);
        } else {
            self.scroll_system.layout.prompt_area = None;
        }

        // Input area (chunks[4] with prompt, was chunks[3])
        let input_area = chunks[4];
        self.scroll_system.layout.input_area = Some(input_area);
        self.input
            .set_max_visible_lines(input_area.height.saturating_sub(2));

        // Get input selection if selecting in input area
        let input_selection = if self.scroll_system.selection.area == SelectionArea::Input {
            self.scroll_system.selection.normalized()
        } else {
            None
        };

        // Border color changes to accent when thinking mode enabled (Tab toggle)
        let input_border_color = if self.thinking_enabled {
            self.ui.theme.accent_color
        } else {
            self.ui.theme.border_color
        };
        // Get hover range for file ref highlighting
        let hover_range = self
            .scroll_system
            .hover
            .input_file_ref
            .as_ref()
            .map(|(s, e, _)| (*s, *e));
        let input_widget = self.input.render_styled_with_file_refs(
            input_area,
            self.ui.theme.bg_color,
            input_border_color,
            self.ui.theme.text_color,
            input_selection,
            self.ui.theme.selection_bg_color,
            self.ui.theme.selection_fg_color,
            Some(self.ui.theme.link_color),
            hover_range,
        );
        f.render_widget(input_widget, input_area);

        // Render input scrollbar (1 column wide, always shows track, thumb when content overflows)
        let total_lines = self.input.get_wrapped_lines_count();
        let visible_lines = self.input.get_max_visible_lines() as usize;
        let scrollbar_area = Rect::new(
            input_area.x + input_area.width - 2,
            input_area.y + 1,
            1,
            input_area.height.saturating_sub(2),
        );
        self.scroll_system.layout.input_scrollbar_area = Some(scrollbar_area);
        render_input_scrollbar(
            f,
            scrollbar_area,
            total_lines,
            visible_lines,
            self.input.get_viewport_offset(),
            &self.ui.theme,
        );

        // Autocomplete popup above input
        if self.autocomplete.visible && self.autocomplete.has_suggestions() {
            let ac_height = 9.min(chunks[2].height.saturating_sub(2));
            let ac_area = Rect::new(
                input_area.x,
                input_area.y.saturating_sub(ac_height),
                input_area.width,
                ac_height,
            );
            self.autocomplete.render(f, ac_area, &self.ui.theme);
        }

        // File search popup above input (mutually exclusive with autocomplete)
        if self.file_search.visible && self.file_search.has_results() && !self.autocomplete.visible
        {
            let fs_height = 12.min(chunks[2].height.saturating_sub(2));
            let fs_area = Rect::new(
                input_area.x,
                input_area.y.saturating_sub(fs_height),
                input_area.width,
                fs_height,
            );
            self.file_search.render(f, fs_area, &self.ui.theme);
        }

        // Status bar (with context tokens in chat mode)
        let context_tokens = if self.context_tokens_used > 0 {
            Some((self.context_tokens_used, self.max_context_tokens()))
        } else {
            None
        };
        render_status_bar(
            f,
            chunks[5],
            &self.ui.theme,
            &self.current_model,
            &self.working_dir,
            context_tokens,
            self.running_process_count,
            self.running_process_elapsed,
        );

        // Render sidebar content (plan and/or plugin window)
        if let Some(sidebar_rect) = sidebar_area {
            // Determine what should be shown in the sidebar
            let plan_visible = self.active_plan.is_some() && self.plan_sidebar.width() > 0;
            let plugin_visible = self.plugin_window.visible && self.plugin_window.height() > 0;

            if plan_visible && plugin_visible {
                // Both visible - split the sidebar
                let divider_position = self.plugin_window.divider_position;
                let plan_height = ((sidebar_rect.height as f32) * divider_position).round() as u16;
                let plugin_height = sidebar_rect.height.saturating_sub(plan_height + 1); // 1 for divider

                // Plan area (top)
                let plan_rect = Rect {
                    x: sidebar_rect.x,
                    y: sidebar_rect.y,
                    width: sidebar_rect.width,
                    height: plan_height,
                };

                // Divider area (1 line between plan and plugin)
                let divider_rect = Rect {
                    x: sidebar_rect.x,
                    y: sidebar_rect.y + plan_height,
                    width: sidebar_rect.width,
                    height: 1,
                };

                // Plugin area (bottom)
                let plugin_rect = Rect {
                    x: sidebar_rect.x,
                    y: sidebar_rect.y + plan_height + 1,
                    width: sidebar_rect.width,
                    height: plugin_height,
                };

                // Render plan
                if let Some(plan) = self.active_plan.clone() {
                    self.scroll_system.layout.plan_sidebar_area = Some(plan_rect);
                    let result = render_plan_sidebar(
                        f.buffer_mut(),
                        plan_rect,
                        &plan,
                        &self.ui.theme,
                        &mut self.plan_sidebar,
                    );
                    self.scroll_system.layout.plan_sidebar_scrollbar_area = result.scrollbar_area;
                }

                // Render divider (draggable)
                self.scroll_system.layout.plugin_divider_area = Some(divider_rect);
                let divider_hovered = self.scroll_system.layout.plugin_divider_hovered;
                render_divider(
                    f.buffer_mut(),
                    divider_rect,
                    &self.ui.theme,
                    divider_hovered,
                );

                // Render plugin window
                self.scroll_system.layout.plugin_window_area = Some(plugin_rect);
                let result = render_plugin_window(
                    f.buffer_mut(),
                    plugin_rect,
                    &self.ui.theme,
                    &mut self.plugin_window,
                );
                self.scroll_system.layout.plugin_window_scrollbar_area = result.scrollbar_area;
            } else if plan_visible {
                // Only plan visible
                self.scroll_system.layout.plan_sidebar_area = Some(sidebar_rect);
                if let Some(plan) = self.active_plan.clone() {
                    let result = render_plan_sidebar(
                        f.buffer_mut(),
                        sidebar_rect,
                        &plan,
                        &self.ui.theme,
                        &mut self.plan_sidebar,
                    );
                    self.scroll_system.layout.plan_sidebar_scrollbar_area = result.scrollbar_area;
                }
                self.scroll_system.layout.plugin_window_area = None;
                self.scroll_system.layout.plugin_window_scrollbar_area = None;
                self.scroll_system.layout.plugin_divider_area = None;
            } else if plugin_visible {
                // Only plugin visible
                self.scroll_system.layout.plugin_window_area = Some(sidebar_rect);
                let result = render_plugin_window(
                    f.buffer_mut(),
                    sidebar_rect,
                    &self.ui.theme,
                    &mut self.plugin_window,
                );
                self.scroll_system.layout.plugin_window_scrollbar_area = result.scrollbar_area;
                self.scroll_system.layout.plan_sidebar_area = None;
                self.scroll_system.layout.plan_sidebar_scrollbar_area = None;
                self.scroll_system.layout.plugin_divider_area = None;
            } else {
                // Nothing visible in sidebar
                self.scroll_system.layout.plan_sidebar_area = None;
                self.scroll_system.layout.plan_sidebar_scrollbar_area = None;
                self.scroll_system.layout.plugin_window_area = None;
                self.scroll_system.layout.plugin_window_scrollbar_area = None;
                self.scroll_system.layout.plugin_divider_area = None;
            }
        } else {
            self.scroll_system.layout.plan_sidebar_area = None;
            self.scroll_system.layout.plan_sidebar_scrollbar_area = None;
            self.scroll_system.layout.plugin_window_area = None;
            self.scroll_system.layout.plugin_window_scrollbar_area = None;
            self.scroll_system.layout.plugin_divider_area = None;
        }
    }
}

/// Render the draggable divider between plan and plugin windows
fn render_divider(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    theme: &crate::tui::themes::Theme,
    hovered: bool,
) {
    use ratatui::style::Style;

    // Use different style when hovered to indicate interactivity
    let color = if hovered {
        theme.accent_color
    } else {
        theme.border_color
    };

    // Draw horizontal line with drag handles
    let divider_char = '─';
    let handle_char = if hovered { '━' } else { '┄' }; // Bold when hovered
    let style = Style::default().fg(color);

    for x in area.x..area.x + area.width {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            // Use handle chars in the middle to indicate draggability
            let is_handle_zone = x > area.x + 2 && x < area.x + area.width - 3;
            cell.set_char(if is_handle_zone {
                handle_char
            } else {
                divider_char
            });
            cell.set_style(style);
        }
    }
}
