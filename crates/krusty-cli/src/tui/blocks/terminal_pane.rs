//! Terminal Pane - A fully interactive PTY terminal widget
//!
//! Spawns a real pseudo-terminal and renders it inside Krusty.
//! Supports full ANSI/VT100 escape sequences, colors, and interactive programs.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{BlockEvent, ClipContext, EventResult, StreamBlock};
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::themes::Theme;

/// Default scrollback lines (reduced from 10000)
const DEFAULT_SCROLLBACK: usize = 2000;

/// Max visible terminal rows
const MAX_VISIBLE_ROWS: u16 = 22;

/// Cursor blink interval in ms
const CURSOR_BLINK_MS: u128 = 530;

/// Terminal pane - an interactive PTY terminal widget
pub struct TerminalPane {
    /// Title shown in header
    title: String,
    /// Terminal emulator state (vt100 parser) - shared with reader thread
    parser: Arc<Mutex<vt100::Parser>>,
    /// Writer to send input to PTY (Mutex for Sync bound, only accessed from main thread)
    pty_writer: Mutex<Box<dyn Write + Send>>,
    /// PTY master for resize operations (Mutex for Sync bound, only accessed from main thread)
    pty_master: Mutex<Box<dyn MasterPty + Send>>,
    /// Child process (Mutex for Sync bound, only accessed from main thread)
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// Shutdown signal for reader thread
    shutdown: Arc<AtomicBool>,
    /// Reader thread handle for cleanup
    reader_handle: Option<std::thread::JoinHandle<()>>,
    /// Notification: new data available (replaces channel)
    has_new_data: Arc<AtomicBool>,
    /// Whether the terminal is focused
    focused: bool,
    /// Auto-scroll to bottom on new content (disabled when user scrolls up)
    auto_scroll: bool,
    /// Whether terminal is running
    running: bool,
    /// Exit code when complete
    exit_code: Option<u32>,
    /// Start time
    start_time: Instant,
    /// Terminal size (rows, cols)
    size: (u16, u16),
    /// Cursor blink state
    cursor_visible: bool,
    /// Last cursor toggle
    last_cursor_toggle: Instant,
    /// Whether the pane is collapsed
    collapsed: bool,
    /// Last rendered width for resize detection
    last_render_width: u16,
    /// Process ID for registry tracking
    process_id: Option<String>,
    /// Whether the terminal is pinned to the top
    pinned: bool,
}

impl TerminalPane {
    /// Spawn a command in a new PTY
    pub fn spawn(cmd: &str, rows: u16, cols: u16) -> anyhow::Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // Build command
        let mut cmd_builder = CommandBuilder::new(cmd);
        cmd_builder.env("TERM", "xterm-256color");

        // Spawn child process
        let child = pair.slave.spawn_command(cmd_builder)?;

        // Get reader and writer from master
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        // Create vt100 parser
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            rows,
            cols,
            DEFAULT_SCROLLBACK,
        )));

        // Shutdown coordination
        let shutdown = Arc::new(AtomicBool::new(false));
        let has_new_data = Arc::new(AtomicBool::new(false));

        // Spawn reader thread with shutdown signal
        let parser_clone = Arc::clone(&parser);
        let shutdown_clone = Arc::clone(&shutdown);
        let has_new_data_clone = Arc::clone(&has_new_data);

        let reader_handle = std::thread::spawn(move || {
            Self::reader_thread(reader, parser_clone, shutdown_clone, has_new_data_clone);
        });

        let now = Instant::now();
        Ok(Self {
            title: cmd.to_string(),
            parser,
            pty_writer: Mutex::new(writer),
            pty_master: Mutex::new(pair.master),
            child: Mutex::new(child),
            shutdown,
            reader_handle: Some(reader_handle),
            has_new_data,
            focused: false,
            auto_scroll: true,
            running: true,
            exit_code: None,
            start_time: now,
            size: (rows, cols),
            cursor_visible: true,
            last_cursor_toggle: now,
            collapsed: false,
            last_render_width: cols,
            process_id: None,
            pinned: false,
        })
    }

    /// Reader thread - reads from PTY and updates parser
    fn reader_thread(
        mut reader: Box<dyn Read + Send>,
        parser: Arc<Mutex<vt100::Parser>>,
        shutdown: Arc<AtomicBool>,
        has_new_data: Arc<AtomicBool>,
    ) {
        let mut buf = [0u8; 4096];

        loop {
            // Check shutdown before blocking read
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            match reader.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    let data = &buf[..n];
                    // Process into parser (vt100 handles scrollback internally)
                    if let Ok(mut p) = parser.lock() {
                        p.process(data);
                    }
                    // Signal new data available
                    has_new_data.store(true, Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
    }

    /// Poll for updates - call in event loop
    pub fn poll(&mut self) -> bool {
        // Check for new data (fast atomic check)
        let updated = self.has_new_data.swap(false, Ordering::Relaxed);

        // Auto-scroll to bottom when new content arrives
        if updated && self.auto_scroll {
            if let Ok(mut parser) = self.parser.lock() {
                parser.set_scrollback(0); // 0 = viewing live screen (bottom)
            }
        }

        // Check if child has exited
        if self.running {
            if let Ok(mut child) = self.child.lock() {
                if let Ok(Some(status)) = child.try_wait() {
                    self.running = false;
                    self.exit_code = Some(status.exit_code());
                }
            }
        }

        updated
    }

    /// Write bytes to the PTY
    pub fn write(&mut self, data: &[u8]) -> anyhow::Result<()> {
        if let Ok(mut writer) = self.pty_writer.lock() {
            writer.write_all(data)?;
            writer.flush()?;
        }
        Ok(())
    }

    /// Resize the terminal
    pub fn resize(&mut self, rows: u16, cols: u16) -> anyhow::Result<()> {
        // Only resize if actually changed
        if self.size == (rows, cols) {
            return Ok(());
        }

        if let Ok(master) = self.pty_master.lock() {
            master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        }

        if let Ok(mut parser) = self.parser.lock() {
            parser.set_size(rows, cols);
        }
        self.size = (rows, cols);
        Ok(())
    }

    /// Set focus state
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Handle a key event - convert to terminal escape sequences
    pub fn handle_key(&mut self, key: KeyEvent) -> anyhow::Result<bool> {
        if !self.focused || !self.running {
            return Ok(false);
        }

        let bytes: Vec<u8> = match key.code {
            // Basic characters
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    let ctrl_char = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a' - 1);
                    vec![ctrl_char]
                } else if key.modifiers.contains(KeyModifiers::ALT) {
                    vec![0x1b, c as u8]
                } else {
                    c.to_string().into_bytes()
                }
            }

            // Special keys
            KeyCode::Enter => vec![b'\r'],
            KeyCode::Backspace => vec![0x7f],
            KeyCode::Tab => vec![b'\t'],
            KeyCode::Esc => vec![0x1b],

            // Arrow keys
            KeyCode::Up => b"\x1b[A".to_vec(),
            KeyCode::Down => b"\x1b[B".to_vec(),
            KeyCode::Right => b"\x1b[C".to_vec(),
            KeyCode::Left => b"\x1b[D".to_vec(),

            // Navigation
            KeyCode::Home => b"\x1b[H".to_vec(),
            KeyCode::End => b"\x1b[F".to_vec(),
            KeyCode::PageUp => b"\x1b[5~".to_vec(),
            KeyCode::PageDown => b"\x1b[6~".to_vec(),
            KeyCode::Insert => b"\x1b[2~".to_vec(),
            KeyCode::Delete => b"\x1b[3~".to_vec(),

            // Function keys
            KeyCode::F(1) => b"\x1bOP".to_vec(),
            KeyCode::F(2) => b"\x1bOQ".to_vec(),
            KeyCode::F(3) => b"\x1bOR".to_vec(),
            KeyCode::F(4) => b"\x1bOS".to_vec(),
            KeyCode::F(5) => b"\x1b[15~".to_vec(),
            KeyCode::F(6) => b"\x1b[17~".to_vec(),
            KeyCode::F(7) => b"\x1b[18~".to_vec(),
            KeyCode::F(8) => b"\x1b[19~".to_vec(),
            KeyCode::F(9) => b"\x1b[20~".to_vec(),
            KeyCode::F(10) => b"\x1b[21~".to_vec(),
            KeyCode::F(11) => b"\x1b[23~".to_vec(),
            KeyCode::F(12) => b"\x1b[24~".to_vec(),
            KeyCode::F(_) => return Ok(false),

            _ => return Ok(false),
        };

        self.write(&bytes)?;
        Ok(true)
    }

    /// Calculate content width consistently
    fn content_width(&self, area_width: u16, has_scrollbar: bool) -> u16 {
        // area_width - left_border(1) - right_border(1) - scrollbar(1 if present)
        if has_scrollbar {
            area_width.saturating_sub(3)
        } else {
            area_width.saturating_sub(2)
        }
    }

    /// Get scroll state: (max_scrollback, current_scrollback) in a single lock
    /// This avoids race conditions between separate lock acquisitions
    fn scroll_state(&self) -> (usize, usize) {
        if let Ok(mut parser) = self.parser.lock() {
            // Save current position
            let current = parser.screen().scrollback();
            // Probe max by setting to MAX (vt100 clamps to valid range)
            parser.set_scrollback(usize::MAX);
            let max = parser.screen().scrollback();
            // Restore position immediately
            parser.set_scrollback(current);
            (max, current)
        } else {
            (0, 0)
        }
    }

    /// Get status indicator
    fn status_indicator(&self, theme: &Theme) -> (&'static str, Color) {
        match (self.running, self.exit_code) {
            (true, _) => ("●", theme.running_color),
            (false, Some(0)) => ("✓", theme.success_color),
            (false, _) => ("✗", theme.error_color),
        }
    }

    /// Get duration string
    fn duration_string(&self) -> String {
        let dur = self.start_time.elapsed();
        let secs = dur.as_secs_f32();
        if secs < 60.0 {
            format!("{:.1}s", secs)
        } else {
            let mins = secs / 60.0;
            format!("{:.1}m", mins)
        }
    }

    /// Convert vt100 color to ratatui color
    fn convert_color(color: vt100::Color) -> Color {
        match color {
            vt100::Color::Default => Color::Reset,
            vt100::Color::Idx(idx) => Color::Indexed(idx),
            vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }

    /// Convert vt100 cell to ratatui style
    fn cell_to_style(cell: &vt100::Cell) -> Style {
        let style = Style::default()
            .fg(Self::convert_color(cell.fgcolor()))
            .bg(Self::convert_color(cell.bgcolor()));

        let mut modifiers = Modifier::empty();
        if cell.bold() {
            modifiers |= Modifier::BOLD;
        }
        if cell.italic() {
            modifiers |= Modifier::ITALIC;
        }
        if cell.underline() {
            modifiers |= Modifier::UNDERLINED;
        }
        if cell.inverse() {
            modifiers |= Modifier::REVERSED;
        }

        style.add_modifier(modifiers)
    }

    /// Resize PTY to match render width (with debouncing)
    /// Uses conservative width estimate to avoid mutex lock on hot path
    pub fn resize_to_width(&mut self, width: u16) {
        // Assume scrollbar present for conservative width estimate (avoids mutex lock)
        // The 1-column difference is negligible and debouncing handles minor changes
        let content_width = self.content_width(width, true);

        // Only resize if width changed by more than 2 columns (debounce)
        if content_width > 10 && (content_width as i16 - self.size.1 as i16).abs() > 2 {
            let _ = self.resize(MAX_VISIBLE_ROWS, content_width);
            self.last_render_width = width;
        }
    }

    /// Check if scrollbar is needed
    pub fn has_scrollbar(&self) -> bool {
        let (max, _) = self.scroll_state();
        max > 0
    }

    /// Scroll up (into history)
    fn scroll_up(&mut self, amount: usize) {
        if let Ok(mut parser) = self.parser.lock() {
            let current = parser.screen().scrollback();
            parser.set_scrollback(current + amount);
        }
        // Disable auto-scroll when user scrolls up
        self.auto_scroll = false;
    }

    /// Scroll down (toward live content)
    fn scroll_down(&mut self, amount: usize) {
        if let Ok(mut parser) = self.parser.lock() {
            let current = parser.screen().scrollback();
            parser.set_scrollback(current.saturating_sub(amount));
            // Re-enable auto-scroll if at bottom
            if parser.screen().scrollback() == 0 {
                self.auto_scroll = true;
            }
        }
    }

    /// Update cursor blink
    fn update_cursor(&mut self) {
        if self.last_cursor_toggle.elapsed().as_millis() > CURSOR_BLINK_MS {
            self.cursor_visible = !self.cursor_visible;
            self.last_cursor_toggle = Instant::now();
        }
    }

    /// Get child process PID
    pub fn get_child_pid(&self) -> Option<u32> {
        self.child.lock().ok().and_then(|c| c.process_id())
    }

    /// Set process ID for registry tracking
    pub fn set_process_id(&mut self, id: String) {
        self.process_id = Some(id);
    }

    /// Get process ID
    pub fn get_process_id(&self) -> Option<&str> {
        self.process_id.as_deref()
    }

    /// Render collapsed view
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let y = area.y;
        let (status, status_color) = self.status_indicator(theme);
        let duration = self.duration_string();
        let text_color = theme.text_color;

        // Truncate title if needed
        let max_len = area.width.saturating_sub(20) as usize;
        let title_display = if self.title.len() > max_len {
            format!("{}...", &self.title[..max_len.saturating_sub(3)])
        } else {
            self.title.clone()
        };

        let prefix = format!("▶ $ {}", title_display);

        // Draw prefix
        let mut x = area.x;
        for ch in prefix.chars() {
            let char_width = ch.width().unwrap_or(0) as u16;
            if x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if ch == '▶' || ch == '$' {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_fg(text_color);
                    }
                }
            }
            x += char_width;
        }
        let prefix_end_x = x;

        // Status and duration on right
        let suffix = format!(" {} {}", status, duration);
        let suffix_width = suffix.width() as u16;
        let suffix_start = (area.x + area.width).saturating_sub(suffix_width);
        let mut x = suffix_start;
        for ch in suffix.chars() {
            let char_width = ch.width().unwrap_or(0) as u16;
            if x >= area.x && x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if ch == '●' || ch == '✓' || ch == '✗' {
                        cell.set_fg(status_color);
                    } else {
                        cell.set_fg(text_color);
                    }
                }
            }
            x += char_width;
        }

        // Fill middle with dots
        let dots_start = prefix_end_x + 1;
        let dots_end = suffix_start.saturating_sub(1);
        for x in dots_start..dots_end {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char('·');
                cell.set_fg(theme.accent_color);
            }
        }
    }

    /// Render header
    fn render_header(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        content_end_x: u16,
        right_x: u16,
        needs_scrollbar: bool,
    ) {
        let y = area.y;
        let border_color = theme.accent_color;
        let text_color = theme.text_color;

        // Left corner
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            cell.set_char('┏');
            cell.set_fg(border_color);
        }

        // Build header content
        let (status, status_color) = self.status_indicator(theme);
        let duration = self.duration_string();
        let focus_indicator = if self.focused { "◉" } else { "○" };
        // Use ASCII for reliable hit detection (3 chars each)
        let pin_btn = if self.pinned { "[^]" } else { "[_]" };
        let close_btn = "[x]";

        let header = format!(" {} {} $ {} ", focus_indicator, status, self.title);
        let status_suffix = format!(" {} {} {}", duration, pin_btn, close_btn);
        let status_suffix_width = status_suffix.width() as u16;

        // Draw header text
        let mut x = area.x + 1;
        for ch in header.chars() {
            let char_width = ch.width().unwrap_or(0) as u16;
            if x < content_end_x.saturating_sub(status_suffix_width) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if ch == '●' || ch == '✓' || ch == '✗' {
                        cell.set_fg(status_color);
                    } else if ch == '◉' || ch == '○' {
                        cell.set_fg(if self.focused {
                            theme.accent_color
                        } else {
                            text_color
                        });
                    } else if ch == '$' {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_fg(text_color);
                    }
                }
                x += char_width;
            }
        }

        // Fill with horizontal line
        let status_start = content_end_x.saturating_sub(status_suffix_width);
        while x < status_start {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
            x += 1;
        }

        // Duration, pin button, and close button
        // Color based on character content rather than position
        for ch in status_suffix.chars() {
            let char_width = ch.width().unwrap_or(0) as u16;
            if x < content_end_x {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if ch == 'x' {
                        cell.set_fg(theme.error_color);
                    } else if ch == '^' {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_fg(theme.dim_color);
                    }
                }
                x += char_width;
            }
        }

        // Scrollbar connector or right corner
        if needs_scrollbar {
            if let Some(cell) = buf.cell_mut((content_end_x, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
        }
        if let Some(cell) = buf.cell_mut((right_x, y)) {
            cell.set_char('┓');
            cell.set_fg(border_color);
        }
    }

    /// Render content area
    fn render_content(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        content_end_x: u16,
        start_y: u16,
        lines_to_show: u16,
        clip_top: u16,
    ) {
        let parser = match self.parser.lock() {
            Ok(p) => p,
            Err(_) => return,
        };
        let screen = parser.screen();

        let border_color = theme.accent_color;
        let (screen_rows, screen_cols) = screen.size();
        let screen_cols = screen_cols as usize;
        let screen_rows = screen_rows as usize;

        // vt100 handles scrollback internally via set_scrollback()
        // screen.cell(row, col) returns the cell at the current view position
        // Row 0 is the top of the visible area (which may be in scrollback)
        let content_width = content_end_x.saturating_sub(area.x).saturating_sub(1) as usize;

        // Calculate which terminal rows to show (accounting for clipping)
        let term_start_row = if clip_top > 0 { clip_top as usize } else { 0 };

        for display_idx in 0..lines_to_show as usize {
            let term_row = term_start_row + display_idx;
            let screen_y = start_y + display_idx as u16;

            if screen_y >= area.y + area.height || term_row >= screen_rows {
                break;
            }

            // Left border
            if let Some(cell) = buf.cell_mut((area.x, screen_y)) {
                cell.set_char('┃');
                cell.set_fg(border_color);
            }

            // Render each column
            for col in 0..content_width {
                let screen_x = area.x + 1 + col as u16;
                if screen_x >= content_end_x {
                    break;
                }

                // Get terminal cell or empty
                if col < screen_cols {
                    if let Some(term_cell) = screen.cell(term_row as u16, col as u16) {
                        let contents = term_cell.contents();
                        let style = Self::cell_to_style(term_cell);

                        if let Some(buf_cell) = buf.cell_mut((screen_x, screen_y)) {
                            let ch = contents.chars().next().unwrap_or(' ');
                            buf_cell.set_char(ch);
                            buf_cell.set_style(style);
                        }
                    }
                } else {
                    // Empty cell beyond terminal width
                    if let Some(buf_cell) = buf.cell_mut((screen_x, screen_y)) {
                        buf_cell.set_char(' ');
                    }
                }
            }

            // Right border - use content_end_x to respect scrollbar gap
            if let Some(cell) = buf.cell_mut((content_end_x, screen_y)) {
                cell.set_char('┃');
                cell.set_fg(border_color);
            }
        }

        // Render cursor only when at live view (scrollback=0), focused, and app hasn't hidden cursor
        let current_scrollback = screen.scrollback();
        let app_hides_cursor = screen.hide_cursor();
        if self.focused
            && self.running
            && self.cursor_visible
            && current_scrollback == 0
            && !app_hides_cursor
        {
            let cursor = screen.cursor_position();
            let cursor_row = cursor.0 as usize;
            let cursor_col = cursor.1 as usize;

            // Cursor is relative to visible screen
            if cursor_row >= term_start_row && cursor_row < term_start_row + lines_to_show as usize
            {
                let display_row = cursor_row - term_start_row;
                let screen_y = start_y + display_row as u16;
                let screen_x = area.x + 1 + cursor_col as u16;

                if screen_x < content_end_x && screen_y < area.y + area.height {
                    if let Some(cell) = buf.cell_mut((screen_x, screen_y)) {
                        cell.set_bg(theme.accent_color);
                        cell.set_fg(Color::Black);
                    }
                }
            }
        }
    }

    /// Render footer
    fn render_footer(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        content_end_x: u16,
        right_x: u16,
        needs_scrollbar: bool,
    ) {
        let y = area.y + area.height - 1;
        let border_color = theme.accent_color;

        // Left corner
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            cell.set_char('┗');
            cell.set_fg(border_color);
        }

        // Horizontal line
        for x in (area.x + 1)..content_end_x {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
        }

        // Scrollbar connector or right corner
        if needs_scrollbar {
            if let Some(cell) = buf.cell_mut((content_end_x, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
        }
        if let Some(cell) = buf.cell_mut((right_x, y)) {
            cell.set_char('┛');
            cell.set_fg(border_color);
        }
    }

    /// Set pinned state
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    /// Check if collapsed
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl StreamBlock for TerminalPane {
    fn height(&self, _width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else {
            // Header + content + footer
            MAX_VISIBLE_ROWS + 2
        }
    }

    fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        _focused: bool,
        clip: Option<ClipContext>,
    ) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        if self.collapsed {
            self.render_collapsed(area, buf, theme);
            return;
        }

        let (clip_top, clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        // Get scroll state once - this is the ONLY place we acquire the parser lock during render
        // All other methods that need scroll info should be passed these cached values
        let (max_scroll, current_scroll) = self.scroll_state();
        let needs_scrollbar = max_scroll > 0;

        // Calculate positions
        let content_end_x = if needs_scrollbar {
            area.x + area.width - 2
        } else {
            area.x + area.width - 1
        };
        let right_x = area.x + area.width - 1;

        let mut render_y = area.y;

        // Header - only if not clipped from top
        if clip_top == 0 {
            self.render_header(area, buf, theme, content_end_x, right_x, needs_scrollbar);
            render_y += 1;
        }

        // Content area
        let reserved_bottom = if clip_bottom == 0 { 1 } else { 0 };
        let reserved_top = if clip_top == 0 { 1 } else { 0 };
        let content_lines_to_show = area.height.saturating_sub(reserved_top + reserved_bottom);

        self.render_content(
            area,
            buf,
            theme,
            content_end_x,
            render_y,
            content_lines_to_show,
            clip_top,
        );

        // Footer - only if not clipped from bottom
        if clip_bottom == 0 {
            self.render_footer(area, buf, theme, content_end_x, right_x, needs_scrollbar);
        }

        // Render scrollbar if needed
        if needs_scrollbar {
            let header_lines = if clip_top == 0 { 1u16 } else { 0 };
            let footer_lines = if clip_bottom == 0 { 1u16 } else { 0 };
            let scrollbar_height = area.height.saturating_sub(header_lines + footer_lines);

            if scrollbar_height > 0 {
                let scrollbar_y = area.y + header_lines;
                let scrollbar_area = Rect::new(content_end_x, scrollbar_y, 1, scrollbar_height);
                // Calculate scroll info from already-fetched values
                let total = (MAX_VISIBLE_ROWS as usize)
                    .saturating_add(max_scroll)
                    .min(u16::MAX as usize) as u16;
                let visible = MAX_VISIBLE_ROWS;
                let offset = max_scroll
                    .saturating_sub(current_scroll)
                    .min(u16::MAX as usize) as u16;
                render_scrollbar(
                    buf,
                    scrollbar_area,
                    offset as usize,
                    total as usize,
                    visible as usize,
                    theme.accent_color,
                    theme.scrollbar_bg_color,
                );
            }
        }
    }

    fn handle_event(
        &mut self,
        event: &Event,
        area: Rect,
        _clip: Option<ClipContext>,
    ) -> EventResult {
        match event {
            // Mouse scroll
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column,
                row,
                ..
            }) => {
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + area.width;

                if in_area && !self.collapsed {
                    self.scroll_down(3);
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column,
                row,
                ..
            }) => {
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + area.width;

                if in_area && !self.collapsed {
                    self.scroll_up(3);
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            // Click to focus/toggle/close
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + area.width;

                if in_area {
                    if self.collapsed {
                        self.collapsed = false;
                        return EventResult::Action(BlockEvent::Expanded);
                    } else if *row == area.y {
                        // Click on header
                        let needs_scrollbar = self.has_scrollbar();
                        let content_end = if needs_scrollbar {
                            area.x + area.width - 2
                        } else {
                            area.x + area.width - 1
                        };
                        // Close button "[x]" is last 3 chars
                        let close_start = content_end.saturating_sub(3);
                        // Pin button "[^]" or "[_]" is 4 chars before close (includes space)
                        let pin_start = close_start.saturating_sub(4);

                        if *column >= close_start && *column < content_end {
                            return EventResult::Action(BlockEvent::Close);
                        } else if *column >= pin_start && *column < close_start {
                            self.pinned = !self.pinned;
                            return EventResult::Action(BlockEvent::Pinned(self.pinned));
                        } else {
                            self.collapsed = true;
                            return EventResult::Action(BlockEvent::Collapsed);
                        }
                    } else {
                        // Click in content - focus
                        self.focused = true;
                        return EventResult::Action(BlockEvent::RequestFocus);
                    }
                }
                EventResult::Ignored
            }
            // Keyboard input (when focused)
            Event::Key(key_event) => {
                if self.focused && self.running {
                    if let Ok(true) = self.handle_key(*key_event) {
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn get_text_content(&self) -> Option<String> {
        // When collapsed, only return title (matches rendered height of 1)
        if self.collapsed {
            return Some(format!("Terminal: {}", self.title));
        }

        if let Ok(parser) = self.parser.lock() {
            Some(parser.screen().contents())
        } else {
            None
        }
    }

    fn tick(&mut self) -> bool {
        // Note: poll() is called separately in poll_terminal_panes() before tick()
        // This only handles cursor blink animation
        if self.focused && self.running {
            self.update_cursor();
            true
        } else {
            false
        }
    }

    fn is_streaming(&self) -> bool {
        self.running
    }
}

impl Drop for TerminalPane {
    fn drop(&mut self) {
        // Signal shutdown to reader thread
        self.shutdown.store(true, Ordering::Relaxed);

        // Kill child process
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }

        // Wait for reader thread to finish (with timeout)
        if let Some(handle) = self.reader_handle.take() {
            // Give the thread a moment to notice shutdown
            std::thread::sleep(std::time::Duration::from_millis(10));
            // Don't block forever - just drop if it doesn't join
            let _ = handle.join();
        }
    }
}
