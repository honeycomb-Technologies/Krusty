//! Toast notification system for TUI
//!
//! Simple notifications in the top-right corner for:
//! - Update alerts (new version available, updated successfully)
//! - Confirmations (copied, saved)

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

use crate::tui::themes::Theme;
use crate::tui::utils::truncate_ellipsis;

/// Maximum number of visible toasts
const MAX_VISIBLE_TOASTS: usize = 3;

/// Default toast duration
const DEFAULT_DURATION: Duration = Duration::from_secs(5);

/// Toast width
const TOAST_WIDTH: u16 = 45;

/// Toast height (including borders)
const TOAST_HEIGHT: u16 = 3;

/// Gap between toasts
const TOAST_GAP: u16 = 1;

/// Type of toast notification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastType {
    /// Positive confirmation (copied, saved, updated)
    Success,
}

impl ToastType {
    fn color(&self, theme: &Theme) -> Color {
        match self {
            ToastType::Success => theme.success_color,
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            ToastType::Success => "✓",
        }
    }
}

/// A toast notification
#[derive(Debug, Clone)]
pub struct Toast {
    /// Message to display
    pub message: String,
    /// Type of toast (affects color/icon)
    pub toast_type: ToastType,
    /// How long to show the toast
    pub duration: Duration,
    /// When the toast was created
    pub created_at: Instant,
}

impl Toast {
    /// Create a new success toast
    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, ToastType::Success)
    }

    fn new(message: impl Into<String>, toast_type: ToastType) -> Self {
        Self {
            message: message.into(),
            toast_type,
            duration: DEFAULT_DURATION,
            created_at: Instant::now(),
        }
    }

    /// Check if the toast has expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    /// Get progress (0.0 to 1.0) for progress bar
    pub fn progress(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32();
        (1.0 - (elapsed / total)).max(0.0)
    }
}

/// Queue of toast notifications
#[derive(Debug, Default)]
pub struct ToastQueue {
    toasts: Vec<Toast>,
}

impl ToastQueue {
    /// Create a new empty toast queue
    pub fn new() -> Self {
        Self { toasts: Vec::new() }
    }

    /// Add a toast to the queue
    pub fn push(&mut self, toast: Toast) {
        // Don't add duplicate messages (prevents spamming same toast)
        if self.toasts.iter().any(|t| t.message == toast.message) {
            return;
        }

        // Remove oldest if at capacity
        while self.toasts.len() >= MAX_VISIBLE_TOASTS {
            self.toasts.remove(0);
        }
        self.toasts.push(toast);
    }

    /// Remove expired toasts, returns true if any were removed
    pub fn tick(&mut self) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|t| !t.is_expired());
        self.toasts.len() != before
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// Get visible toasts (most recent first)
    pub fn visible(&self) -> impl Iterator<Item = &Toast> {
        self.toasts.iter().rev().take(MAX_VISIBLE_TOASTS)
    }
}

/// Render toasts in the top-right corner
pub fn render_toasts(buf: &mut Buffer, area: Rect, queue: &ToastQueue, theme: &Theme) {
    if queue.is_empty() {
        return;
    }

    let start_x = area.width.saturating_sub(TOAST_WIDTH + 2);

    for (i, toast) in queue.visible().enumerate() {
        let y = area.y + 4 + (i as u16 * (TOAST_HEIGHT + TOAST_GAP));

        if y + TOAST_HEIGHT > area.y + area.height {
            break; // Don't render off-screen
        }

        let toast_area = Rect::new(start_x, y, TOAST_WIDTH, TOAST_HEIGHT);
        render_toast(buf, toast_area, toast, theme);
    }
}

/// Render a single toast with translucent effect (no solid background)
fn render_toast(buf: &mut Buffer, area: Rect, toast: &Toast, theme: &Theme) {
    let color = toast.toast_type.color(theme);
    let icon = toast.toast_type.icon();
    let border_style = Style::default().fg(color);

    // Content line background only (creates "floating text" effect)
    let content_y = area.y + 1;
    for x in (area.x + 1)..(area.x + area.width - 1) {
        if let Some(cell) = buf.cell_mut((x, content_y)) {
            cell.set_char(' ');
            cell.set_bg(theme.bg_color);
        }
    }

    // Top border (no background)
    if let Some(cell) = buf.cell_mut((area.x, area.y)) {
        cell.set_char('╭').set_style(border_style);
    }
    for x in (area.x + 1)..(area.x + area.width - 1) {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_char('─').set_style(border_style);
        }
    }
    if let Some(cell) = buf.cell_mut((area.x + area.width - 1, area.y)) {
        cell.set_char('╮').set_style(border_style);
    }

    // Side borders
    for y in (area.y + 1)..(area.y + area.height - 1) {
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            cell.set_char('│').set_style(border_style);
        }
        if let Some(cell) = buf.cell_mut((area.x + area.width - 1, y)) {
            cell.set_char('│').set_style(border_style);
        }
    }

    // Bottom border with progress
    if let Some(cell) = buf.cell_mut((area.x, area.y + area.height - 1)) {
        cell.set_char('╰').set_style(border_style);
    }
    let progress_width = ((area.width - 2) as f32 * toast.progress()) as u16;
    for (i, x) in ((area.x + 1)..(area.x + area.width - 1)).enumerate() {
        if let Some(cell) = buf.cell_mut((x, area.y + area.height - 1)) {
            if (i as u16) < progress_width {
                cell.set_char('━').set_fg(color);
            } else {
                cell.set_char('─').set_fg(theme.dim_color);
            }
        }
    }
    if let Some(cell) = buf.cell_mut((area.x + area.width - 1, area.y + area.height - 1)) {
        cell.set_char('╯').set_style(border_style);
    }

    // Icon
    let mut cx = area.x + 2;
    for ch in icon.chars() {
        if let Some(cell) = buf.cell_mut((cx, content_y)) {
            cell.set_char(ch).set_fg(color).set_bg(theme.bg_color);
        }
        // Account for character width (some icons are wide)
        cx += UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
    }
    cx += 1; // Space after icon

    // Message (truncate if needed)
    let max_msg_width = (area.width - 5) as usize;
    let msg_display = truncate_ellipsis(&toast.message, max_msg_width);

    for ch in msg_display.chars() {
        if cx < area.x + area.width - 2 {
            if let Some(cell) = buf.cell_mut((cx, content_y)) {
                cell.set_char(ch)
                    .set_fg(theme.text_color)
                    .set_bg(theme.bg_color);
            }
            // Account for character width
            cx += UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
        }
    }
}
