//! Explore block - Consortium view of parallel sub-agents
//!
//! Displays a compact, animated view of sub-agents exploring the codebase.
//! Features custom whirlpool spinner animation and full box border.

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{ClipContext, EventResult, StreamBlock};
use crate::agent::subagent::{AgentProgress, AgentProgressStatus};
use crate::tui::themes::Theme;

/// Spiral spinner for agent rows - Unicode 16.0 square spirals
/// Rotates clockwise: top-left → top-right → bottom-right → bottom-left
const SPIRAL_FRAMES: &[char] = &['𜱼', '𜱽', '𜱾', '𜱿'];

/// Pincer animation for header - crab claw open/close
/// Both frames same width (4 chars) to prevent text jumping
const PINCER_FRAMES: &[&str] = &["(\\/)", "(||)"];

/// Spiral spinner interval (agent rows)
const SPIRAL_INTERVAL: Duration = Duration::from_millis(150);

/// Pincer animation - slower, more organic timing
const PINCER_BASE_INTERVAL: Duration = Duration::from_millis(800);
const PINCER_VARIANCE: u64 = 400; // +/- randomness

/// State of a single agent in the consortium
#[derive(Debug, Clone)]
struct AgentEntry {
    /// Display name (e.g., "tui", "agent", "main")
    name: String,
    /// Current status
    status: AgentProgressStatus,
    /// Number of tool calls made (actual)
    tool_count: usize,
    /// Final elapsed time in milliseconds (set on completion)
    final_elapsed_ms: Option<u64>,
    /// Token usage estimate (actual)
    tokens: usize,
    /// Current action description
    current_action: Option<String>,
    /// Spinner frame index (each agent has its own phase)
    spinner_idx: usize,
    /// Output text (populated on completion)
    output: String,
    /// Whether output is expanded
    expanded: bool,

    // === Timing ===
    /// When this agent started (for real-time elapsed calculation)
    started_at: Instant,

    // === Display values (interpolated for smooth animation) ===
    /// Displayed tool count (interpolates toward tool_count)
    display_tools: f32,
    /// Displayed tokens (interpolates toward tokens)
    display_tokens: f32,

    // === Cached formatted strings (updated when values change) ===
    cached_tokens: String,
    last_cached_tokens: usize,
}

/// Interpolation speed (0.0 = instant, 1.0 = never moves)
/// Lower = faster catch-up. 0.15 gives smooth ~100ms transitions.
const LERP_SPEED: f32 = 0.15;

impl AgentEntry {
    fn from_progress(progress: &AgentProgress) -> Self {
        let mut entry = Self {
            name: progress.name.clone(),
            status: progress.status.clone(),
            tool_count: progress.tool_count,
            final_elapsed_ms: None,
            tokens: progress.tokens,
            current_action: progress.current_action.clone(),
            spinner_idx: 0,
            output: String::new(),
            expanded: false,
            // Track when agent started for real-time elapsed
            started_at: Instant::now(),
            // Start display values at actual (no initial animation)
            display_tools: progress.tool_count as f32,
            display_tokens: progress.tokens as f32,
            // Initialize cache
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

        // When agent completes, freeze the elapsed time
        if was_running && self.status == AgentProgressStatus::Complete {
            self.final_elapsed_ms = Some(self.started_at.elapsed().as_millis() as u64);
        }
    }

    /// Get current elapsed time in milliseconds
    /// Real-time for running agents, frozen for completed agents
    fn elapsed_ms(&self) -> u64 {
        match self.final_elapsed_ms {
            Some(ms) => ms,
            None => self.started_at.elapsed().as_millis() as u64,
        }
    }

    /// Interpolate display values toward actual values
    /// Returns true if any value changed (needs redraw)
    fn interpolate(&mut self) -> bool {
        let mut changed = false;

        // Tools - interpolate
        let target_tools = self.tool_count as f32;
        if (self.display_tools - target_tools).abs() > 0.01 {
            self.display_tools += (target_tools - self.display_tools) * (1.0 - LERP_SPEED);
            changed = true;
        } else if self.display_tools != target_tools {
            self.display_tools = target_tools;
            changed = true;
        }

        // Tokens - interpolate
        let target_tokens = self.tokens as f32;
        if (self.display_tokens - target_tokens).abs() > 1.0 {
            self.display_tokens += (target_tokens - self.display_tokens) * (1.0 - LERP_SPEED);
            changed = true;
        } else if self.display_tokens != target_tokens {
            self.display_tokens = target_tokens;
            changed = true;
        }

        // Time always needs cache update while running (real-time tick)
        // or if interpolated values changed
        self.update_cache();

        // Always redraw while running (time is ticking)
        if self.status == AgentProgressStatus::Running {
            changed = true;
        }

        changed
    }

    /// Update cached formatted strings
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

    /// Get displayed tool count (rounded for display)
    fn displayed_tools(&self) -> usize {
        self.display_tools.round() as usize
    }

    /// Get cached formatted tokens
    fn format_tokens(&self) -> &str {
        &self.cached_tokens
    }
}

/// Explore block with Consortium design - full box border and whirlpool spinner
pub struct ExploreBlock {
    /// Tool use ID for matching with results
    tool_use_id: Option<String>,
    /// The exploration prompt
    prompt: String,
    /// Agents by task ID
    agents: HashMap<String, AgentEntry>,
    /// Order of agent IDs (for consistent rendering)
    agent_order: Vec<String>,
    /// Whether collapsed
    collapsed: bool,
    /// Whether still streaming
    streaming: bool,
    /// Last spiral spinner update
    last_spiral_update: Instant,
    /// Last pincer animation update
    last_pincer_update: Instant,
    /// Next pincer interval (randomized)
    next_pincer_interval: Duration,
    /// Pincer animation index (for header)
    pincer_idx: usize,
    /// Spiral spinner base index (for agent wave effect)
    spiral_idx: usize,
    /// Selected agent index for keyboard nav
    selected_idx: Option<usize>,
    /// Final summary text
    summary: Option<String>,

    // === Cached totals (updated every tick while streaming) ===
    cached_total_tools: usize,
    cached_total_tokens: usize,
    cached_total_elapsed: u64,
    cached_completed: usize,
    /// Cached formatted strings for footer
    cached_total_time_str: String,
    cached_total_tokens_str: String,
}

impl ExploreBlock {
    pub fn with_tool_id(prompt: String, tool_use_id: String) -> Self {
        Self {
            tool_use_id: Some(tool_use_id),
            prompt,
            agents: HashMap::new(),
            agent_order: Vec::new(),
            collapsed: false,
            streaming: true,
            last_spiral_update: Instant::now(),
            last_pincer_update: Instant::now(),
            next_pincer_interval: PINCER_BASE_INTERVAL,
            pincer_idx: 0,
            spiral_idx: 0,
            selected_idx: None,
            summary: None,
            // Initialize cached totals
            cached_total_tools: 0,
            cached_total_tokens: 0,
            cached_total_elapsed: 0,
            cached_completed: 0,
            cached_total_time_str: "0.0".to_string(),
            cached_total_tokens_str: "0".to_string(),
        }
    }

    pub fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    /// Update agent state from progress
    pub fn update_progress(&mut self, progress: AgentProgress) {
        let task_id = progress.task_id.clone();
        if let Some(agent) = self.agents.get_mut(&task_id) {
            agent.update(&progress);
        } else {
            tracing::debug!(task_id = %task_id, name = %progress.name, "ExploreBlock: new agent");
            self.agent_order.push(task_id.clone());
            self.agents
                .insert(task_id, AgentEntry::from_progress(&progress));
        }
    }

    /// Recalculate and cache totals from all agents
    fn update_cached_totals(&mut self) {
        // Calculate totals - use real elapsed time (max of all agents)
        self.cached_total_tools = self.agents.values().map(|a| a.displayed_tools()).sum();
        self.cached_total_tokens = self
            .agents
            .values()
            .map(|a| a.display_tokens as usize)
            .sum();
        self.cached_total_elapsed = self
            .agents
            .values()
            .map(|a| a.elapsed_ms())
            .max()
            .unwrap_or(0);
        self.cached_completed = self
            .agents
            .values()
            .filter(|a| a.status == AgentProgressStatus::Complete)
            .count();

        // Format time
        let secs = self.cached_total_elapsed / 1000;
        self.cached_total_time_str = if secs >= 60 {
            format!("{}:{:02}", secs / 60, secs % 60)
        } else {
            format!("{}.{}", secs, (self.cached_total_elapsed % 1000) / 100)
        };

        // Format tokens
        self.cached_total_tokens_str = if self.cached_total_tokens >= 1_000_000 {
            format!("{:.1}M", self.cached_total_tokens as f64 / 1_000_000.0)
        } else if self.cached_total_tokens >= 1000 {
            format!("{:.1}k", self.cached_total_tokens as f64 / 1000.0)
        } else {
            format!("{}", self.cached_total_tokens)
        };
    }

    /// Mark exploration as complete
    pub fn complete(&mut self, output: String) {
        self.streaming = false;

        // Parse agent sections from output
        let mut current_agent: Option<(String, String)> = None;

        for line in output.lines() {
            if line.starts_with("## Agent: ") {
                if let Some((id, text)) = current_agent.take() {
                    if let Some(agent) = self.agents.get_mut(&id) {
                        agent.output = text.trim().to_string();
                        agent.status = AgentProgressStatus::Complete;
                    }
                }
                let id = line.trim_start_matches("## Agent: ").to_string();
                current_agent = Some((id, String::new()));
            } else if line.starts_with("---") || line.starts_with("**Summary**") {
                if let Some((id, text)) = current_agent.take() {
                    if let Some(agent) = self.agents.get_mut(&id) {
                        agent.output = text.trim().to_string();
                        agent.status = AgentProgressStatus::Complete;
                    }
                }
            } else if let Some((_, ref mut text)) = current_agent {
                text.push_str(line);
                text.push('\n');
            }
        }

        if let Some((id, text)) = current_agent.take() {
            if let Some(agent) = self.agents.get_mut(&id) {
                agent.output = text.trim().to_string();
                agent.status = AgentProgressStatus::Complete;
            }
        }

        self.summary = Some(output);
    }

    fn header_pincer(&self) -> &'static str {
        PINCER_FRAMES[self.pincer_idx % PINCER_FRAMES.len()]
    }

    /// Render header: ╭─ Consortium (\/) : 5 agents ─────────────────╮
    fn render_header(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 10 {
            return;
        }

        let border_style = Style::default().fg(theme.border_color);
        let title_style = Style::default()
            .fg(theme.accent_color)
            .add_modifier(Modifier::BOLD);

        // Build header content
        let pincer = if self.streaming {
            self.header_pincer()
        } else {
            "✓"
        };
        let title = format!("Consortium {} : {} agents", pincer, self.agents.len());

        // Calculate positions
        let content_start = area.x + 3; // After "╭─ "

        // Fill entire line with ─ first (establishes baseline)
        let full_line: String = std::iter::repeat_n('─', area.width as usize).collect();
        buf.set_string(area.x, area.y, &full_line, border_style);

        // Left corner
        buf.set_string(area.x, area.y, "╭", border_style);

        // Title (with space padding)
        buf.set_string(content_start, area.y, &title, title_style);

        // Right corner (render last to ensure it's visible)
        buf.set_string(area.x + area.width - 1, area.y, "╮", border_style);
    }

    /// Render a horizontal separator: ├─────────────────────────────────┤
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

    /// Render agents with full box (│ on both sides)
    fn render_agents(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.height == 0 || area.width < 20 {
            return;
        }

        let border_style = Style::default().fg(theme.border_color);
        let dim_style = Style::default().fg(theme.dim_color);

        let mut y = area.y;

        for (idx, task_id) in self.agent_order.iter().enumerate() {
            if y >= area.y + area.height {
                break;
            }

            let agent = match self.agents.get(task_id) {
                Some(a) => a,
                None => continue,
            };

            let is_selected = self.selected_idx == Some(idx);

            // Status icon with whirlpool spinner
            let (icon, icon_style) = match agent.status {
                AgentProgressStatus::Running => (
                    agent.spinner_char(),
                    Style::default().fg(theme.accent_color),
                ),
                AgentProgressStatus::Complete => ('✓', Style::default().fg(theme.success_color)),
                AgentProgressStatus::Failed => ('✗', Style::default().fg(theme.error_color)),
            };

            // Truncate name safely at char boundary
            let name: String = agent.name.chars().take(8).collect();
            let name_padded = format!("{:<8}", name);

            // Use displayed (interpolated) values and cached formatted strings
            let tools_str = format!("{:>2} tools", agent.displayed_tools());
            let tokens_str = format!("{:>5}", agent.format_tokens());

            let action = agent.current_action.as_deref().unwrap_or("");
            // Account for: border(2) + spinner(2) + name(9) + 3 dividers(6) + tools(9) + tokens(6) + border(1) = 35
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

            // Left border
            buf.set_string(area.x, y, "│ ", border_style);

            // Agent row content with proper spacing
            let mut x = area.x + 2;

            // Spinner
            buf.set_string(x, y, icon.to_string(), icon_style);
            x += icon.width().unwrap_or(1) as u16 + 1;

            // Name
            buf.set_string(x, y, &name_padded, line_style);
            x += 9;

            // Divider
            buf.set_string(x, y, "│", border_style);
            x += 2;

            // Tools
            buf.set_string(x, y, &tools_str, dim_style);
            x += 9;

            // Divider
            buf.set_string(x, y, "│", border_style);
            x += 2;

            // Tokens
            buf.set_string(x, y, &tokens_str, dim_style);
            x += 6;

            // Divider
            buf.set_string(x, y, "│", border_style);
            x += 2;

            // Action
            buf.set_string(x, y, &action_display, line_style);

            // Right border
            buf.set_string(area.x + area.width - 1, y, "│", border_style);

            y += 1;

            // Expanded output
            if agent.expanded && !agent.output.is_empty() {
                for line in agent.output.lines().take(4) {
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

        // Fill remaining rows with empty bordered lines
        while y < area.y + area.height {
            buf.set_string(area.x, y, "│", border_style);
            buf.set_string(area.x + area.width - 1, y, "│", border_style);
            y += 1;
        }
    }

    /// Render footer: ╰─ 5/5 ✓ │ 108 tools │ 2:09 │ 1.2k tok ────────╯
    fn render_footer(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 10 {
            return;
        }

        let border_style = Style::default().fg(theme.border_color);
        let dim_style = Style::default().fg(theme.dim_color);

        // Fill entire line with ─ first
        let full_line: String = std::iter::repeat_n('─', area.width as usize).collect();
        buf.set_string(area.x, area.y, &full_line, border_style);

        // Left corner
        buf.set_string(area.x, area.y, "╰", border_style);

        // Use cached values (no per-frame calculations)
        let mut x = area.x + 2; // After "╰─"

        // Completion count
        let completion = format!(" {}/{} ✓ ", self.cached_completed, self.agents.len());
        buf.set_string(x, area.y, &completion, dim_style);
        x += completion.width() as u16;

        // Divider
        buf.set_string(x, area.y, "│", border_style);
        x += 1;

        // Tools count
        let tools_str = format!(" {} tools ", self.cached_total_tools);
        buf.set_string(x, area.y, &tools_str, dim_style);
        x += tools_str.width() as u16;

        // Divider
        buf.set_string(x, area.y, "│", border_style);
        x += 1;

        // Time (cached formatted string)
        let time_str = format!(" {} ", self.cached_total_time_str);
        buf.set_string(x, area.y, &time_str, dim_style);
        x += time_str.width() as u16;

        // Divider
        buf.set_string(x, area.y, "│", border_style);
        x += 1;

        // Tokens (cached formatted string)
        let tokens_str = format!(" {} ", self.cached_total_tokens_str);
        buf.set_string(x, area.y, &tokens_str, dim_style);

        // Right corner (render last)
        buf.set_string(area.x + area.width - 1, area.y, "╯", border_style);
    }
}

impl StreamBlock for ExploreBlock {
    fn height(&self, _width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else {
            // Layout: header(1) + agents(n) + separator(1) + footer(1) = 3 + n
            let mut agent_lines = 0u16;
            for task_id in &self.agent_order {
                if let Some(agent) = self.agents.get(task_id) {
                    agent_lines += 1;
                    if agent.expanded {
                        agent_lines += agent.output.lines().count().min(4) as u16;
                    }
                }
            }
            // Minimum 4 lines (header + 1 agent row + separator + footer)
            4 + agent_lines
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

        // Calculate fitted width based on content, capped at area.width
        // Agent row: border(2) + spinner(2) + name(9) + div(2) + tools(9) + div(2) + tokens(6) + div(2) + action(~25) + border(1) = ~60
        let content_width = 68u16;
        let width = content_width.min(area.width);

        // Header (line 0) - skip if clipped
        if clip_top == 0 {
            self.render_header(Rect::new(area.x, area.y, width, 1), buf, theme);
        }

        if self.collapsed || area.height < 2 {
            return;
        }

        if area.height == 2 {
            // Just header and footer
            self.render_footer(Rect::new(area.x, area.y + 1, width, 1), buf, theme);
            return;
        }

        if area.height == 3 {
            // Header, separator, footer
            self.render_separator(Rect::new(area.x, area.y + 1, width, 1), buf, theme);
            self.render_footer(Rect::new(area.x, area.y + 2, width, 1), buf, theme);
            return;
        }

        // Full layout: header | agents | separator | footer
        let agents_height = area.height.saturating_sub(3);
        let agents_area = Rect::new(area.x, area.y + 1, width, agents_height);

        if !self.agents.is_empty() {
            self.render_agents(agents_area, buf, theme);
        } else {
            // Show prompt when no agents yet, with borders
            let border_style = Style::default().fg(theme.border_color);
            let dim_style = Style::default().fg(theme.dim_color);

            for row in 0..agents_height {
                let y = agents_area.y + row;
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

        // Separator before footer
        self.render_separator(
            Rect::new(area.x, area.y + area.height - 2, width, 1),
            buf,
            theme,
        );

        // Footer
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

                // Header click toggles collapse
                if y == area.y {
                    self.collapsed = !self.collapsed;
                    return EventResult::Consumed;
                }

                // Agent click toggles expanded
                if !self.collapsed && y > area.y && y < area.y + area.height - 2 {
                    let idx = (y - area.y - 1) as usize;
                    if idx < self.agent_order.len() {
                        if let Some(agent) = self.agents.get_mut(&self.agent_order[idx]) {
                            agent.expanded = !agent.expanded;
                            return EventResult::Consumed;
                        }
                    }
                }

                EventResult::Ignored
            }
            Event::Key(KeyEvent { code, .. }) => match code {
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some(idx) = self.selected_idx {
                        if let Some(task_id) = self.agent_order.get(idx) {
                            if let Some(agent) = self.agents.get_mut(task_id) {
                                agent.expanded = !agent.expanded;
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
                    } else if !self.agent_order.is_empty() {
                        self.selected_idx = Some(self.agent_order.len() - 1);
                    }
                    EventResult::Consumed
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(idx) = self.selected_idx {
                        if idx < self.agent_order.len() - 1 {
                            self.selected_idx = Some(idx + 1);
                        }
                    } else if !self.agent_order.is_empty() {
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

        // Pincer animation - slow with random timing
        if self.last_pincer_update.elapsed() >= self.next_pincer_interval {
            self.last_pincer_update = Instant::now();
            self.pincer_idx = (self.pincer_idx + 1) % PINCER_FRAMES.len();

            // Randomize next interval using simple hash
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            self.last_pincer_update.hash(&mut hasher);
            let variance =
                (hasher.finish() % (PINCER_VARIANCE * 2)) as i64 - PINCER_VARIANCE as i64;
            self.next_pincer_interval =
                Duration::from_millis((PINCER_BASE_INTERVAL.as_millis() as i64 + variance) as u64);

            needs_redraw = true;
        }

        // Spiral animation - faster, consistent timing
        if self.last_spiral_update.elapsed() >= SPIRAL_INTERVAL {
            self.last_spiral_update = Instant::now();
            self.spiral_idx = (self.spiral_idx + 1) % SPIRAL_FRAMES.len();

            // Advance each running agent's spinner (staggered for visual effect)
            for (i, task_id) in self.agent_order.iter().enumerate() {
                if let Some(agent) = self.agents.get_mut(task_id) {
                    if agent.status == AgentProgressStatus::Running {
                        agent.spinner_idx = (self.spiral_idx + i) % SPIRAL_FRAMES.len();
                    }
                }
            }

            needs_redraw = true;
        }

        // Interpolate display values for smooth number transitions
        for agent in self.agents.values_mut() {
            if agent.interpolate() {
                needs_redraw = true;
            }
        }

        // Always update cached totals while streaming (time is always ticking)
        // This is cheap since we're already iterating agents above
        if !self.agents.is_empty() {
            self.update_cached_totals();
            needs_redraw = true;
        }

        needs_redraw
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }

    fn get_text_content(&self) -> Option<String> {
        let mut content = format!("Consortium: {}\n\n", self.prompt);

        for task_id in &self.agent_order {
            if let Some(agent) = self.agents.get(task_id) {
                content.push_str(&format!(
                    "{} | {} tools | {} tokens | {:?}\n",
                    agent.name, agent.tool_count, agent.tokens, agent.status
                ));
                if !agent.output.is_empty() {
                    content.push_str(&agent.output);
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
