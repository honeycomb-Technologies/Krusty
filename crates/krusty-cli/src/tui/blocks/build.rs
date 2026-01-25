//! Build block - The Kraken view of parallel builder agents
//!
//! Displays a compact, animated view of Opus builder agents working together.
//! Features tentacle animation and coordination status.
//!
//! "The Kraken" is the builder swarm - Opus agents that write code.
//! (Octopus + Opus = Kraken unleashed)

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

use super::{ClipContext, EventResult, StreamBlock};
use crate::agent::subagent::{AgentProgress, AgentProgressStatus};
use crate::tui::themes::Theme;

/// Rolling sine wave frames - travels left to right like audio equalizer
const WAVE_FRAMES: &[&str] = &[
    "▁▃▅▇▅▃▁",
    "▃▅▇▅▃▁▃",
    "▅▇▅▃▁▃▅",
    "▇▅▃▁▃▅▇",
    "▅▃▁▃▅▇▅",
    "▃▁▃▅▇▅▃",
];

/// Spiral square spinner for agent rows (same as Consortium)
const SPIRAL_FRAMES: &[char] = &['𜱼', '𜱽', '𜱾', '𜱿'];

/// Wave animation interval
const WAVE_INTERVAL: Duration = Duration::from_millis(150);

/// Spiral spinner interval (agent rows)
const SPIRAL_INTERVAL: Duration = Duration::from_millis(180);

/// State of a single builder agent
#[derive(Debug, Clone)]
struct BuilderEntry {
    /// Display name (e.g., "auth", "api", "db")
    name: String,
    /// Current status
    status: AgentProgressStatus,
    /// Number of tool calls made
    tool_count: usize,
    /// Token usage estimate
    tokens: usize,
    /// Current action description
    current_action: Option<String>,
    /// Spinner frame index
    spinner_idx: usize,
    /// Output text (populated on completion)
    output: String,
    /// Whether output is expanded
    expanded: bool,
    /// When this agent started
    started_at: Instant,
    /// Final elapsed time (set on completion)
    final_elapsed_ms: Option<u64>,

    // === Display values (interpolated) ===
    display_tools: f32,
    display_tokens: f32,
    cached_tokens: String,
    last_cached_tokens: usize,
}

const LERP_SPEED: f32 = 0.15;

impl BuilderEntry {
    fn from_progress(progress: &AgentProgress) -> Self {
        let mut entry = Self {
            name: progress.name.clone(),
            status: progress.status.clone(),
            tool_count: progress.tool_count,
            tokens: progress.tokens,
            current_action: progress.current_action.clone(),
            spinner_idx: 0,
            output: String::new(),
            expanded: false,
            started_at: Instant::now(),
            final_elapsed_ms: None,
            display_tools: progress.tool_count as f32,
            display_tokens: progress.tokens as f32,
            cached_tokens: String::new(),
            last_cached_tokens: 0,
        };
        entry.update_cache();
        entry
    }

    fn update(&mut self, progress: &AgentProgress) {
        let was_running = self.status == AgentProgressStatus::Running;
        self.status = progress.status.clone();
        self.tool_count = progress.tool_count;
        self.tokens = progress.tokens;
        self.current_action = progress.current_action.clone();

        if was_running && self.status == AgentProgressStatus::Complete {
            self.final_elapsed_ms = Some(self.started_at.elapsed().as_millis() as u64);
        }
    }

    fn elapsed_ms(&self) -> u64 {
        match self.final_elapsed_ms {
            Some(ms) => ms,
            None => self.started_at.elapsed().as_millis() as u64,
        }
    }

    fn interpolate(&mut self) -> bool {
        let mut changed = false;

        let target_tools = self.tool_count as f32;
        if (self.display_tools - target_tools).abs() > 0.01 {
            self.display_tools += (target_tools - self.display_tools) * (1.0 - LERP_SPEED);
            changed = true;
        } else if self.display_tools != target_tools {
            self.display_tools = target_tools;
            changed = true;
        }

        let target_tokens = self.tokens as f32;
        if (self.display_tokens - target_tokens).abs() > 1.0 {
            self.display_tokens += (target_tokens - self.display_tokens) * (1.0 - LERP_SPEED);
            changed = true;
        } else if self.display_tokens != target_tokens {
            self.display_tokens = target_tokens;
            changed = true;
        }

        self.update_cache();

        if self.status == AgentProgressStatus::Running {
            changed = true;
        }

        changed
    }

    fn update_cache(&mut self) {
        let displayed_tokens = self.display_tokens as usize;

        if self.last_cached_tokens != displayed_tokens {
            self.last_cached_tokens = displayed_tokens;
            self.cached_tokens = if displayed_tokens >= 1_000_000 {
                format!("{:.1}M", displayed_tokens as f64 / 1_000_000.0)
            } else if displayed_tokens >= 1000 {
                format!("{:.1}k", displayed_tokens as f64 / 1000.0)
            } else {
                format!("{}", displayed_tokens)
            };
        }
    }

    fn spinner_char(&self) -> char {
        SPIRAL_FRAMES[self.spinner_idx % SPIRAL_FRAMES.len()]
    }

    fn displayed_tools(&self) -> usize {
        self.display_tools.round() as usize
    }

    fn format_tokens(&self) -> &str {
        &self.cached_tokens
    }
}

/// Build block with Octopod design - tentacle animation for builder swarm
pub struct BuildBlock {
    /// Tool use ID for matching with results
    tool_use_id: Option<String>,
    /// The build prompt/task
    prompt: String,
    /// Builders by task ID
    builders: HashMap<String, BuilderEntry>,
    /// Order of builder IDs
    builder_order: Vec<String>,
    /// Whether collapsed
    collapsed: bool,
    /// Whether still streaming
    streaming: bool,
    /// Last wave animation update
    last_wave_update: Instant,
    /// Last spiral spinner update
    last_spiral_update: Instant,
    /// Wave animation index
    wave_idx: usize,
    /// Spiral spinner base index
    spiral_idx: usize,
    /// Selected builder index for keyboard nav
    selected_idx: Option<usize>,
    /// Final summary text
    summary: Option<String>,

    // === Cached totals ===
    cached_total_tools: usize,
    cached_total_tokens: usize,
    cached_total_elapsed: u64,
    cached_completed: usize,
    cached_total_time_str: String,
    cached_total_tokens_str: String,

    // === Build context stats ===
    lines_added: usize,
    lines_removed: usize,
}

impl BuildBlock {
    pub fn with_tool_id(prompt: String, tool_use_id: String) -> Self {
        Self {
            tool_use_id: Some(tool_use_id),
            prompt,
            builders: HashMap::new(),
            builder_order: Vec::new(),
            collapsed: false,
            streaming: true,
            last_wave_update: Instant::now(),
            last_spiral_update: Instant::now(),
            wave_idx: 0,
            spiral_idx: 0,
            selected_idx: None,
            summary: None,
            cached_total_tools: 0,
            cached_total_tokens: 0,
            cached_total_elapsed: 0,
            cached_completed: 0,
            cached_total_time_str: "0.0".to_string(),
            cached_total_tokens_str: "0".to_string(),
            lines_added: 0,
            lines_removed: 0,
        }
    }

    pub fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    /// Update builder state from progress
    pub fn update_progress(&mut self, progress: AgentProgress) {
        // Update line diff stats from progress
        self.lines_added = self.lines_added.max(progress.lines_added);
        self.lines_removed = self.lines_removed.max(progress.lines_removed);

        let task_id = progress.task_id.clone();
        if let Some(builder) = self.builders.get_mut(&task_id) {
            builder.update(&progress);
        } else {
            tracing::debug!(task_id = %task_id, name = %progress.name, "BuildBlock: new builder");
            self.builder_order.push(task_id.clone());
            self.builders
                .insert(task_id, BuilderEntry::from_progress(&progress));
        }
    }

    /// Mark build as complete
    pub fn complete(&mut self, output: String) {
        self.streaming = false;

        // Parse builder sections from output
        let mut current_builder: Option<(String, String)> = None;

        for line in output.lines() {
            if line.starts_with("## Builder: ") {
                if let Some((id, text)) = current_builder.take() {
                    if let Some(builder) = self.builders.get_mut(&id) {
                        builder.output = text.trim().to_string();
                        builder.status = AgentProgressStatus::Complete;
                    }
                }
                let id = line.trim_start_matches("## Builder: ").to_string();
                current_builder = Some((id, String::new()));
            } else if line.starts_with("---") || line.starts_with("**Summary**") {
                if let Some((id, text)) = current_builder.take() {
                    if let Some(builder) = self.builders.get_mut(&id) {
                        builder.output = text.trim().to_string();
                        builder.status = AgentProgressStatus::Complete;
                    }
                }
            } else if let Some((_, ref mut text)) = current_builder {
                text.push_str(line);
                text.push('\n');
            }
        }

        if let Some((id, text)) = current_builder.take() {
            if let Some(builder) = self.builders.get_mut(&id) {
                builder.output = text.trim().to_string();
                builder.status = AgentProgressStatus::Complete;
            }
        }

        self.summary = Some(output);
    }

    fn update_cached_totals(&mut self) {
        self.cached_total_tools = self.builders.values().map(|b| b.displayed_tools()).sum();
        self.cached_total_tokens = self
            .builders
            .values()
            .map(|b| b.display_tokens as usize)
            .sum();
        self.cached_total_elapsed = self
            .builders
            .values()
            .map(|b| b.elapsed_ms())
            .max()
            .unwrap_or(0);
        self.cached_completed = self
            .builders
            .values()
            .filter(|b| b.status == AgentProgressStatus::Complete)
            .count();

        let secs = self.cached_total_elapsed / 1000;
        self.cached_total_time_str = if secs >= 60 {
            format!("{}:{:02}", secs / 60, secs % 60)
        } else {
            format!("{}.{}", secs, (self.cached_total_elapsed % 1000) / 100)
        };

        self.cached_total_tokens_str = if self.cached_total_tokens >= 1_000_000 {
            format!("{:.1}M", self.cached_total_tokens as f64 / 1_000_000.0)
        } else if self.cached_total_tokens >= 1000 {
            format!("{:.1}k", self.cached_total_tokens as f64 / 1000.0)
        } else {
            format!("{}", self.cached_total_tokens)
        };
    }

    fn header_wave(&self) -> &'static str {
        WAVE_FRAMES[self.wave_idx % WAVE_FRAMES.len()]
    }

    /// Render header: ╭─ Kraken ▁▃▅▇▅▃▁ : 3 builders ─────────────╮
    fn render_header(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 10 {
            return;
        }

        let border_style = Style::default().fg(theme.border_color);
        let title_style = Style::default()
            .fg(theme.accent_color)
            .add_modifier(Modifier::BOLD);

        let wave = if self.streaming {
            self.header_wave()
        } else {
            "✓"
        };
        let title = format!("Kraken {} : {} builders", wave, self.builders.len());

        let content_start = area.x + 3;

        let full_line = "─".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &full_line, border_style);

        buf.set_string(area.x, area.y, "╭", border_style);
        buf.set_string(content_start, area.y, &title, title_style);
        buf.set_string(area.x + area.width - 1, area.y, "╮", border_style);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 3 {
            return;
        }
        let border_style = Style::default().fg(theme.border_color);
        buf.set_string(area.x, area.y, "├", border_style);
        let fill: String = std::iter::repeat_n('─', (area.width - 2) as usize).collect();
        buf.set_string(area.x + 1, area.y, &fill, border_style);
        buf.set_string(area.x + area.width - 1, area.y, "┤", border_style);
    }

    fn render_builders(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.height == 0 || area.width < 20 {
            return;
        }

        let border_style = Style::default().fg(theme.border_color);
        let dim_style = Style::default().fg(theme.dim_color);

        let mut y = area.y;

        for (idx, task_id) in self.builder_order.iter().enumerate() {
            if y >= area.y + area.height {
                break;
            }

            let builder = match self.builders.get(task_id) {
                Some(b) => b,
                None => continue,
            };

            let is_selected = self.selected_idx == Some(idx);

            let (icon, icon_style) = match builder.status {
                AgentProgressStatus::Running => (
                    builder.spinner_char(),
                    Style::default().fg(theme.accent_color),
                ),
                AgentProgressStatus::Complete => ('✓', Style::default().fg(theme.success_color)),
                AgentProgressStatus::Failed => ('✗', Style::default().fg(theme.error_color)),
            };

            // Truncate name safely at char boundary
            let name: String = builder.name.chars().take(8).collect();
            let name_padded = format!("{:<8}", name);

            let tools_str = format!("{:>2} tools", builder.displayed_tools());
            let tokens_str = format!("{:>5}", builder.format_tokens());

            let action = builder.current_action.as_deref().unwrap_or("");
            let action_max = (area.width as usize).saturating_sub(38);
            // Truncate action safely at char boundary
            let action_display = if action.len() > action_max && action_max > 3 {
                let truncated: String = action.chars().take(action_max.saturating_sub(3)).collect();
                format!("{}...", truncated)
            } else {
                action.to_string()
            };

            let line_style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_color)
            };

            buf.set_string(area.x, y, "│ ", border_style);

            let mut x = area.x + 2;

            buf.set_string(x, y, icon.to_string(), icon_style);
            x += 2;

            buf.set_string(x, y, &name_padded, line_style);
            x += 9;

            buf.set_string(x, y, "│", border_style);
            x += 2;

            buf.set_string(x, y, &tools_str, dim_style);
            x += 9;

            buf.set_string(x, y, "│", border_style);
            x += 2;

            buf.set_string(x, y, &tokens_str, dim_style);
            x += 6;

            buf.set_string(x, y, "│", border_style);
            x += 2;

            buf.set_string(x, y, &action_display, line_style);

            buf.set_string(area.x + area.width - 1, y, "│", border_style);

            y += 1;

            if builder.expanded && !builder.output.is_empty() {
                for line in builder.output.lines().take(4) {
                    if y >= area.y + area.height {
                        break;
                    }
                    buf.set_string(area.x, y, "│   ", border_style);
                    let max_len = (area.width as usize).saturating_sub(5);
                    let truncated = if line.len() > max_len && max_len > 3 {
                        format!("{}...", &line[..max_len - 3])
                    } else {
                        line.to_string()
                    };
                    buf.set_string(area.x + 4, y, &truncated, dim_style);
                    buf.set_string(area.x + area.width - 1, y, "│", border_style);
                    y += 1;
                }
            }
        }

        while y < area.y + area.height {
            buf.set_string(area.x, y, "│", border_style);
            buf.set_string(area.x + area.width - 1, y, "│", border_style);
            y += 1;
        }
    }

    /// Render footer: ╰─ 2/3 ✓ │ 42 tools │ 1:23 │ 15k │ 3 types ─╯
    fn render_footer(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 10 {
            return;
        }

        let border_style = Style::default().fg(theme.border_color);
        let dim_style = Style::default().fg(theme.dim_color);

        let full_line = "─".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &full_line, border_style);

        buf.set_string(area.x, area.y, "╰", border_style);

        let mut x = area.x + 2;

        let completion = format!(" {}/{} ✓ ", self.cached_completed, self.builders.len());
        buf.set_string(x, area.y, &completion, dim_style);
        x += completion.width() as u16;

        buf.set_string(x, area.y, "│", border_style);
        x += 1;

        let tools_str = format!(" {} tools ", self.cached_total_tools);
        buf.set_string(x, area.y, &tools_str, dim_style);
        x += tools_str.width() as u16;

        buf.set_string(x, area.y, "│", border_style);
        x += 1;

        let time_str = format!(" {} ", self.cached_total_time_str);
        buf.set_string(x, area.y, &time_str, dim_style);
        x += time_str.width() as u16;

        buf.set_string(x, area.y, "│", border_style);
        x += 1;

        let tokens_str = format!(" {} ", self.cached_total_tokens_str);
        buf.set_string(x, area.y, &tokens_str, dim_style);
        x += tokens_str.width() as u16;

        // Show line diff stats
        if self.lines_added > 0 || self.lines_removed > 0 {
            buf.set_string(x, area.y, "│", border_style);
            x += 1;

            let add_style = Style::default().fg(theme.success_color);
            let del_style = Style::default().fg(theme.error_color);

            buf.set_string(x, area.y, " +", add_style);
            x += 2;
            let add_str = format!("{} ", self.lines_added);
            buf.set_string(x, area.y, &add_str, add_style);
            x += add_str.width() as u16;

            buf.set_string(x, area.y, "-", del_style);
            x += 1;
            let del_str = format!("{} ", self.lines_removed);
            buf.set_string(x, area.y, &del_str, del_style);
        }
        let _ = x; // silence unused warning

        buf.set_string(area.x + area.width - 1, area.y, "╯", border_style);
    }
}

impl StreamBlock for BuildBlock {
    fn height(&self, _width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else {
            let mut builder_lines = 0u16;
            for task_id in &self.builder_order {
                if let Some(builder) = self.builders.get(task_id) {
                    builder_lines += 1;
                    if builder.expanded {
                        builder_lines += builder.output.lines().count().min(4) as u16;
                    }
                }
            }
            4 + builder_lines
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
        if area.height == 0 || area.width == 0 {
            return;
        }

        let (clip_top, _clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        let content_width = 68u16;
        let width = content_width.min(area.width);

        if clip_top == 0 {
            self.render_header(Rect::new(area.x, area.y, width, 1), buf, theme);
        }

        if self.collapsed || area.height < 2 {
            return;
        }

        if area.height == 2 {
            self.render_footer(Rect::new(area.x, area.y + 1, width, 1), buf, theme);
            return;
        }

        if area.height == 3 {
            self.render_separator(Rect::new(area.x, area.y + 1, width, 1), buf, theme);
            self.render_footer(Rect::new(area.x, area.y + 2, width, 1), buf, theme);
            return;
        }

        let builders_height = area.height.saturating_sub(3);
        let builders_area = Rect::new(area.x, area.y + 1, width, builders_height);

        if !self.builders.is_empty() {
            self.render_builders(builders_area, buf, theme);
        } else {
            let border_style = Style::default().fg(theme.border_color);
            let dim_style = Style::default().fg(theme.dim_color);

            for row in 0..builders_height {
                let y = builders_area.y + row;
                buf.set_string(area.x, y, "│", border_style);
                if row == 0 {
                    let prompt_max = (width as usize).saturating_sub(4);
                    let prompt_display = if self.prompt.len() > prompt_max && prompt_max > 3 {
                        format!(" {}...", &self.prompt[..prompt_max - 4])
                    } else {
                        format!(" {}", self.prompt)
                    };
                    buf.set_string(area.x + 1, y, &prompt_display, dim_style);
                }
                buf.set_string(area.x + width - 1, y, "│", border_style);
            }
        }

        self.render_separator(
            Rect::new(area.x, area.y + area.height - 2, width, 1),
            buf,
            theme,
        );

        self.render_footer(
            Rect::new(area.x, area.y + area.height - 1, width, 1),
            buf,
            theme,
        );
    }

    fn handle_event(
        &mut self,
        event: &Event,
        area: Rect,
        _clip: Option<ClipContext>,
    ) -> EventResult {
        match event {
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                row,
                ..
            }) => {
                let y = *row;

                if y == area.y {
                    self.collapsed = !self.collapsed;
                    return EventResult::Consumed;
                }

                if !self.collapsed && y > area.y && y < area.y + area.height - 2 {
                    let idx = (y - area.y - 1) as usize;
                    if idx < self.builder_order.len() {
                        if let Some(builder) = self.builders.get_mut(&self.builder_order[idx]) {
                            builder.expanded = !builder.expanded;
                            return EventResult::Consumed;
                        }
                    }
                }

                EventResult::Ignored
            }
            Event::Key(KeyEvent { code, .. }) => match code {
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some(idx) = self.selected_idx {
                        if let Some(task_id) = self.builder_order.get(idx) {
                            if let Some(builder) = self.builders.get_mut(task_id) {
                                builder.expanded = !builder.expanded;
                                return EventResult::Consumed;
                            }
                        }
                    } else {
                        self.collapsed = !self.collapsed;
                        return EventResult::Consumed;
                    }
                    EventResult::Ignored
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(idx) = self.selected_idx {
                        if idx > 0 {
                            self.selected_idx = Some(idx - 1);
                        }
                    } else if !self.builder_order.is_empty() {
                        self.selected_idx = Some(self.builder_order.len() - 1);
                    }
                    EventResult::Consumed
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(idx) = self.selected_idx {
                        if idx < self.builder_order.len() - 1 {
                            self.selected_idx = Some(idx + 1);
                        }
                    } else if !self.builder_order.is_empty() {
                        self.selected_idx = Some(0);
                    }
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            _ => EventResult::Ignored,
        }
    }

    fn tick(&mut self) -> bool {
        if !self.streaming {
            return false;
        }

        let mut needs_redraw = false;

        // Wave animation
        if self.last_wave_update.elapsed() >= WAVE_INTERVAL {
            self.last_wave_update = Instant::now();
            self.wave_idx = (self.wave_idx + 1) % WAVE_FRAMES.len();
            needs_redraw = true;
        }

        // Spiral spinner animation
        if self.last_spiral_update.elapsed() >= SPIRAL_INTERVAL {
            self.last_spiral_update = Instant::now();
            self.spiral_idx = (self.spiral_idx + 1) % SPIRAL_FRAMES.len();

            for (i, task_id) in self.builder_order.iter().enumerate() {
                if let Some(builder) = self.builders.get_mut(task_id) {
                    if builder.status == AgentProgressStatus::Running {
                        builder.spinner_idx = (self.spiral_idx + i) % SPIRAL_FRAMES.len();
                    }
                }
            }

            needs_redraw = true;
        }

        // Interpolate display values
        for builder in self.builders.values_mut() {
            if builder.interpolate() {
                needs_redraw = true;
            }
        }

        // Update cached totals
        if !self.builders.is_empty() {
            self.update_cached_totals();
            needs_redraw = true;
        }

        needs_redraw
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }

    fn get_text_content(&self) -> Option<String> {
        let mut content = format!("Kraken Build: {}\n\n", self.prompt);

        for task_id in &self.builder_order {
            if let Some(builder) = self.builders.get(task_id) {
                content.push_str(&format!(
                    "{} | {} tools | {} tokens | {:?}\n",
                    builder.name, builder.tool_count, builder.tokens, builder.status
                ));
                if !builder.output.is_empty() {
                    content.push_str(&builder.output);
                    content.push('\n');
                }
            }
        }

        if let Some(ref summary) = self.summary {
            content.push_str(&format!("\nSummary:\n{}\n", summary));
        }

        Some(content)
    }
}
