//! Plugin Window Component
//!
//! Manages the plugin window state, rendering, and interaction.
//! The plugin window lives in the right sidebar area, stacked below the plan sidebar.
//!
//! Supports two rendering modes:
//! - Text: Standard ratatui widgets rendered to the buffer
//! - KittyGraphics: Pixel rendering via Kitty graphics protocol (60fps capable)

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders, Widget},
};
use std::io::Write;

use crate::tui::plugins::{
    kitty_graphics, GamepadHandler, KittyGraphics, Plugin, PluginContext, PluginRenderMode,
};
use crate::tui::themes::Theme;

/// Minimum height for plugin window when visible (in lines)
pub const PLUGIN_MIN_HEIGHT: u16 = 8;

/// Default divider position (percentage of sidebar height for plan vs plugin)
/// 0.5 means 50% plan, 50% plugin
pub const DEFAULT_DIVIDER_POSITION: f32 = 0.5;

/// Result of rendering the plugin window
pub struct PluginWindowRenderResult {
    /// Scrollbar area (if scrolling is needed)
    pub scrollbar_area: Option<Rect>,
}

/// Graphics frame waiting to be rendered after buffer flush
pub struct PendingGraphics {
    /// Pixel data to render
    pub frame: crate::tui::plugins::PluginFrame,
    /// Cell position (column)
    pub col: u16,
    /// Cell position (row)
    pub row: u16,
    /// Display width in cells
    pub cols: u16,
    /// Display height in cells
    pub rows: u16,
}

/// Plugin window state
pub struct PluginWindowState {
    /// Whether plugin window is visible
    pub visible: bool,
    /// Whether plugin window has keyboard focus (click to focus, Delete to unfocus)
    pub focused: bool,
    /// Current animated height (0 to target)
    pub current_height: u16,
    /// Target height
    pub target_height: u16,
    /// Minimum height when visible
    pub min_height: u16,
    /// Divider position (0.0-1.0, percentage of sidebar for plan vs plugin)
    pub divider_position: f32,
    /// Active plugin (boxed for trait object)
    active_plugin: Option<Box<dyn Plugin>>,
    /// Active plugin ID (for persistence)
    pub active_plugin_id: Option<String>,
    /// Scroll offset for plugin content (if needed)
    pub scroll_offset: usize,
    /// Total content lines (for scrolling)
    pub total_lines: usize,
    /// Kitty graphics handler
    pub graphics: KittyGraphics,
    /// Cell width in pixels
    pub cell_width: u16,
    /// Cell height in pixels
    pub cell_height: u16,
    /// Pending graphics to render after buffer flush
    pending_graphics: Option<PendingGraphics>,
    /// Gamepad handler for controller input
    pub gamepad: GamepadHandler,
    /// Last known area for click detection
    pub last_area: Option<Rect>,
}

impl Default for PluginWindowState {
    fn default() -> Self {
        Self {
            visible: false,
            focused: false,
            current_height: 0,
            target_height: 0,
            min_height: PLUGIN_MIN_HEIGHT,
            divider_position: DEFAULT_DIVIDER_POSITION,
            active_plugin: None,
            active_plugin_id: None,
            scroll_offset: 0,
            total_lines: 0,
            graphics: KittyGraphics::new(),
            cell_width: 0,
            cell_height: 0,
            pending_graphics: None,
            gamepad: GamepadHandler::new(),
            last_area: None,
        }
    }
}

impl PluginWindowState {
    /// Toggle plugin window visibility
    /// If preferred_plugin_id is provided and no plugin is active, load that plugin
    pub fn toggle(&mut self, preferred_plugin_id: Option<&str>) {
        self.visible = !self.visible;
        if self.visible {
            // If no plugin is active, load preferred or first available
            if self.active_plugin.is_none() {
                let plugin_id = preferred_plugin_id.map(String::from).or_else(|| {
                    crate::tui::plugins::builtin_plugins()
                        .into_iter()
                        .next()
                        .map(|p| p.id().to_string())
                });

                if let Some(id) = plugin_id {
                    self.set_plugin(crate::tui::plugins::get_plugin_by_id(&id));
                }
            }
            // Set target height to minimum - will be expanded during layout
            self.target_height = self.min_height;
        } else {
            self.target_height = 0;
            self.scroll_offset = 0;
            self.focused = false;
            // Clear any displayed graphics when hiding
            let _ = self.graphics.clear_all(&mut std::io::stdout());
        }
    }

    /// Focus the plugin window (keyboard input goes to plugin)
    pub fn focus(&mut self) {
        if self.visible {
            self.focused = true;
        }
    }

    /// Unfocus the plugin window (keyboard input goes back to main input)
    pub fn unfocus(&mut self) {
        self.focused = false;
    }

    /// Check if a point is within the plugin window area
    #[allow(dead_code)]
    pub fn contains_point(&self, x: u16, y: u16) -> bool {
        if let Some(area) = self.last_area {
            x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
        } else {
            false
        }
    }

    /// Set the active plugin
    pub fn set_plugin(&mut self, plugin: Option<Box<dyn Plugin>>) {
        // Clear graphics from old plugin
        let _ = self.graphics.clear_all(&mut std::io::stdout());

        // Deactivate old plugin
        if let Some(ref mut old) = self.active_plugin {
            old.on_deactivate();
        }

        // Activate new plugin
        if let Some(mut new_plugin) = plugin {
            self.active_plugin_id = Some(new_plugin.id().to_string());
            new_plugin.on_activate();
            self.active_plugin = Some(new_plugin);
        } else {
            self.active_plugin_id = None;
            self.active_plugin = None;
        }
    }

    /// Get active plugin reference
    pub fn active_plugin(&self) -> Option<&dyn Plugin> {
        self.active_plugin.as_ref().map(|p| p.as_ref())
    }

    /// Get active plugin mutable reference
    pub fn active_plugin_mut(&mut self) -> Option<&mut Box<dyn Plugin>> {
        self.active_plugin.as_mut()
    }

    /// Switch to next available plugin
    pub fn next_plugin(&mut self) {
        let plugins = crate::tui::plugins::builtin_plugins();
        if plugins.is_empty() {
            return;
        }

        let current_idx = self
            .active_plugin_id
            .as_ref()
            .and_then(|id| plugins.iter().position(|p| p.id() == id))
            .unwrap_or(0);

        let next_idx = (current_idx + 1) % plugins.len();
        let next_id = plugins[next_idx].id().to_string();

        self.set_plugin(crate::tui::plugins::get_plugin_by_id(&next_id));
    }

    /// Switch to previous available plugin
    pub fn prev_plugin(&mut self) {
        let plugins = crate::tui::plugins::builtin_plugins();
        if plugins.is_empty() {
            return;
        }

        let current_idx = self
            .active_plugin_id
            .as_ref()
            .and_then(|id| plugins.iter().position(|p| p.id() == id))
            .unwrap_or(0);

        let prev_idx = if current_idx == 0 {
            plugins.len() - 1
        } else {
            current_idx - 1
        };
        let prev_id = plugins[prev_idx].id().to_string();

        self.set_plugin(crate::tui::plugins::get_plugin_by_id(&prev_id));
    }

    /// Animate height towards target
    /// Returns true if animation is still in progress or plugin needs redraw
    pub fn tick(&mut self) -> bool {
        let mut needs_redraw = false;

        // Animate height
        if self.current_height != self.target_height {
            let remaining = (self.target_height as i16 - self.current_height as i16).unsigned_abs();
            let step = (remaining / 5).clamp(2, 8);

            if self.current_height < self.target_height {
                self.current_height = (self.current_height + step).min(self.target_height);
            } else {
                self.current_height = self.current_height.saturating_sub(step);
                if self.current_height < step {
                    self.current_height = self.target_height;
                }
            }
            needs_redraw = true;
        }

        // Poll gamepad and pass to active plugin
        if self.gamepad.poll() {
            needs_redraw = true;
        }

        // Pass gamepad button states to RetroArch plugin
        if let Some(ref mut plugin) = self.active_plugin {
            if plugin.id() == "retroarch" {
                if let Some(retroarch) = plugin
                    .as_any_mut()
                    .downcast_mut::<crate::tui::plugins::retroarch::RetroArchPlugin>(
                ) {
                    // Pass each pressed button to the plugin
                    for button_id in self.gamepad.pressed_buttons() {
                        retroarch.press_button(button_id);
                    }
                }
            }
        }

        // Tick the active plugin
        if let Some(ref mut plugin) = self.active_plugin {
            if plugin.tick() {
                needs_redraw = true;
            }
        }

        needs_redraw
    }

    /// Get current height for layout calculations
    pub fn height(&self) -> u16 {
        self.current_height
    }

    /// Scroll up
    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Scroll down
    pub fn scroll_down(&mut self, visible_height: usize) {
        let max_offset = self.total_lines.saturating_sub(visible_height);
        if self.scroll_offset < max_offset {
            self.scroll_offset += 1;
        }
    }

    /// Update cell dimensions (call when terminal size changes)
    pub fn update_cell_size(&mut self) {
        if let Some((w, h)) = kitty_graphics::query_cell_size() {
            self.cell_width = w;
            self.cell_height = h;
            self.graphics.set_cell_size(w, h);
        }
    }

    /// Detect Kitty graphics support
    pub fn detect_graphics_support(&mut self) -> bool {
        let supported = self.graphics.detect_support();
        tracing::info!("Kitty graphics support detected: {}", supported);
        supported
    }

    /// Flush any pending graphics (call after buffer flush)
    pub fn flush_pending_graphics(&mut self) {
        if let Some(pending) = self.pending_graphics.take() {
            tracing::trace!(
                "Flushing graphics: frame={}x{} at ({},{}) cells={}x{}",
                pending.frame.width,
                pending.frame.height,
                pending.col,
                pending.row,
                pending.cols,
                pending.rows
            );
            let mut stdout = std::io::stdout();
            if let Err(e) = self.graphics.display_frame(
                &mut stdout,
                &pending.frame,
                pending.col,
                pending.row,
                pending.cols,
                pending.rows,
            ) {
                tracing::error!("Failed to display frame: {}", e);
            }
            if let Err(e) = stdout.flush() {
                tracing::error!("Failed to flush stdout: {}", e);
            }
        }
    }

    /// Store pending graphics for later flush
    fn set_pending_graphics(&mut self, pending: Option<PendingGraphics>) {
        self.pending_graphics = pending;
    }
}

/// Render the plugin window
pub fn render_plugin_window(
    buf: &mut Buffer,
    area: Rect,
    theme: &Theme,
    state: &mut PluginWindowState,
) -> PluginWindowRenderResult {
    // Store area for click detection
    state.last_area = Some(area);

    // Clear any pending graphics from previous frame
    state.set_pending_graphics(None);

    if area.width < 10 || area.height < 5 {
        return PluginWindowRenderResult {
            scrollbar_area: None,
        };
    }

    // Draw border with plugin name as title
    // Show focus indicator in title
    let focus_indicator = if state.focused { " [FOCUSED] " } else { "" };
    let title = state
        .active_plugin()
        .map(|p| format!(" {}{}", p.display_name(), focus_indicator))
        .unwrap_or_else(|| format!(" Plugin{}", focus_indicator));

    // Use accent color for border when focused
    let border_color = if state.focused {
        theme.accent_color
    } else {
        theme.border_color
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent_color)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(theme.bg_color));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width < 5 || inner.height < 3 {
        return PluginWindowRenderResult {
            scrollbar_area: None,
        };
    }

    // Render plugin switcher indicator at bottom
    render_plugin_switcher(buf, area, theme, state);

    // Adjust inner area for switcher
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(1), // Reserve 1 line for switcher
    };

    // Calculate pixel dimensions for graphics mode
    let (pixel_width, pixel_height) = state
        .graphics
        .pixels_for_cells(content_area.width, content_area.height);

    // Create context for plugin rendering
    let ctx = PluginContext { theme };

    // Check plugin render mode and render accordingly
    if let Some(ref mut plugin) = state.active_plugin {
        match plugin.render_mode() {
            PluginRenderMode::Text => {
                // Standard ratatui rendering
                plugin.render(content_area, buf, &ctx);
            }
            PluginRenderMode::KittyGraphics => {
                tracing::trace!(
                    "Plugin wants KittyGraphics mode, supported={}",
                    state.graphics.is_supported()
                );
                // Check if graphics is supported
                if state.graphics.is_supported() {
                    // Request frame from plugin
                    if let Some(frame) = plugin.render_frame(pixel_width, pixel_height) {
                        tracing::trace!("Got frame {}x{}", frame.width, frame.height);
                        // Clear the content area in buffer (graphics will overlay)
                        // Use bulk set_style instead of cell-by-cell iteration for performance
                        buf.set_style(content_area, Style::default().bg(theme.bg_color));

                        // Store pending graphics for flush after buffer draw
                        state.set_pending_graphics(Some(PendingGraphics {
                            frame,
                            col: content_area.x,
                            row: content_area.y,
                            cols: content_area.width,
                            rows: content_area.height,
                        }));
                    } else {
                        // Plugin returned no frame, render fallback
                        render_graphics_placeholder(buf, content_area, theme, "No frame");
                    }
                } else {
                    // Graphics not supported, show message
                    render_graphics_placeholder(
                        buf,
                        content_area,
                        theme,
                        "Kitty graphics not supported",
                    );
                }
            }
        }
    } else {
        // No plugin active - show placeholder
        render_no_plugin(buf, content_area, theme);
    }

    PluginWindowRenderResult {
        scrollbar_area: None,
    }
}

/// Render placeholder when no plugin is active
fn render_no_plugin(buf: &mut Buffer, area: Rect, theme: &Theme) {
    let message = "No plugin active";
    let hint = "Click ◀ ▶ to switch plugins";

    let msg_x = area.x + (area.width.saturating_sub(message.len() as u16)) / 2;
    let msg_y = area.y + area.height / 2 - 1;

    for (i, ch) in message.chars().enumerate() {
        if let Some(cell) = buf.cell_mut((msg_x + i as u16, msg_y)) {
            cell.set_char(ch);
            cell.set_fg(theme.dim_color);
        }
    }

    let hint_x = area.x + (area.width.saturating_sub(hint.len() as u16)) / 2;
    let hint_y = msg_y + 1;

    for (i, ch) in hint.chars().enumerate() {
        if let Some(cell) = buf.cell_mut((hint_x + i as u16, hint_y)) {
            cell.set_char(ch);
            cell.set_fg(theme.dim_color);
        }
    }
}

/// Render placeholder when graphics mode isn't available
fn render_graphics_placeholder(buf: &mut Buffer, area: Rect, theme: &Theme, message: &str) {
    let msg_x = area.x + (area.width.saturating_sub(message.len() as u16)) / 2;
    let msg_y = area.y + area.height / 2;

    for (i, ch) in message.chars().enumerate() {
        if let Some(cell) = buf.cell_mut((msg_x + i as u16, msg_y)) {
            cell.set_char(ch);
            cell.set_fg(theme.dim_color);
        }
    }
}

/// Render plugin switcher indicator at bottom of window
fn render_plugin_switcher(buf: &mut Buffer, area: Rect, theme: &Theme, state: &PluginWindowState) {
    let plugins = crate::tui::plugins::builtin_plugins();
    if plugins.is_empty() {
        return;
    }

    // Find current plugin index
    let current_idx = state
        .active_plugin_id
        .as_ref()
        .and_then(|id| plugins.iter().position(|p| p.id() == id))
        .unwrap_or(0);

    // Build indicator: "◀ ● ○ ▶" for dots, or "◀ 1/5 ▶" for many plugins
    let indicator = if plugins.len() <= 3 {
        // Show dots
        let mut dots = String::new();
        for (i, _) in plugins.iter().enumerate() {
            if i == current_idx {
                dots.push('●');
            } else {
                dots.push('○');
            }
            if i < plugins.len() - 1 {
                dots.push(' ');
            }
        }
        format!("◀ {} ▶", dots)
    } else {
        format!("◀ {}/{} ▶", current_idx + 1, plugins.len())
    };

    // Position at bottom center, inside border
    // Use chars().count() for display width (not byte length) since we have unicode
    let display_width = indicator.chars().count() as u16;
    let ind_x = area.x + (area.width.saturating_sub(display_width)) / 2;
    let ind_y = area.y + area.height - 2; // Inside bottom border

    for (i, ch) in indicator.chars().enumerate() {
        if let Some(cell) = buf.cell_mut((ind_x + i as u16, ind_y)) {
            // Active dot gets accent color, everything else (arrows, inactive dots) gets dim
            let color = if ch == '●' {
                theme.accent_color
            } else {
                theme.dim_color
            };
            cell.set_char(ch);
            cell.set_fg(color);
        }
    }
}
