use std::{
    any::Any,
    sync::Arc,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::{
    kitty_graphics::PluginFrame, InstalledPluginDescriptor, Plugin, PluginContext,
    PluginEventResult, PluginRenderMode,
};

/// Lightweight managed plugin host representation used until per-plugin
/// worker execution is wired in.
pub struct ManagedPlugin {
    descriptor: InstalledPluginDescriptor,
    frame_tick: u64,
    last_render_mode_flip: Instant,
}

impl ManagedPlugin {
    pub fn new(descriptor: InstalledPluginDescriptor) -> Self {
        Self {
            descriptor,
            frame_tick: 0,
            last_render_mode_flip: Instant::now(),
        }
    }

    fn render_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(format!(
            "{} v{}",
            self.descriptor.name, self.descriptor.version
        )));
        lines.push(Line::from(format!(
            "publisher: {}",
            self.descriptor.publisher
        )));
        lines.push(Line::from(format!(
            "runtime: {}",
            match self.descriptor.runtime {
                crate::plugins::PluginRuntime::Native => "native",
                crate::plugins::PluginRuntime::Wasm => "wasm",
                crate::plugins::PluginRuntime::Js => "js",
            }
        )));
        lines.push(Line::from(format!(
            "mode: {}",
            match self.descriptor.render_mode {
                PluginRenderMode::Text => "text",
                PluginRenderMode::KittyGraphics => "frame",
            }
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(
            self.descriptor
                .description
                .clone()
                .unwrap_or_else(|| "No description provided.".to_string()),
        ));
        lines.push(Line::from(""));
        lines.push(Line::from("This plugin is loaded from ~/.mitsuro/plugins."));
        lines.push(Line::from(match self.descriptor.runtime {
            crate::plugins::PluginRuntime::Wasm => {
                "WASM TUI execution is not wired yet; manifest/runtime support is installed."
            }
            crate::plugins::PluginRuntime::Js => {
                "JS/TS execution is handled by the edon/libnode plugin host."
            }
            crate::plugins::PluginRuntime::Native => "Press r to request native reload.",
        }));
        lines
    }
}

impl Plugin for ManagedPlugin {
    fn id(&self) -> &str {
        &self.descriptor.id
    }

    fn name(&self) -> &str {
        &self.descriptor.name
    }

    fn display_name(&self) -> String {
        format!("{} ({})", self.descriptor.name, self.descriptor.version)
    }

    fn render_mode(&self) -> PluginRenderMode {
        self.descriptor.render_mode
    }

    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &PluginContext) {
        let paragraph = Paragraph::new(self.render_lines()).style(
            Style::default()
                .fg(ctx.theme.text_color)
                .bg(ctx.theme.bg_color),
        );
        paragraph.render(area, buf);
    }

    fn render_frame(&mut self, width: u32, height: u32) -> Option<PluginFrame> {
        if self.descriptor.render_mode != PluginRenderMode::KittyGraphics {
            return None;
        }

        if width == 0 || height == 0 {
            return None;
        }

        let width = width.clamp(16, 640);
        let height = height.clamp(16, 480);
        let mut pixels = vec![0u8; (width * height * 4) as usize];

        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                let motion = self.frame_tick as u32;
                let r = ((x + motion) % 255) as u8;
                let g = ((y + motion * 2) % 255) as u8;
                let b = (((x ^ y) + motion * 3) % 255) as u8;

                pixels[idx] = r;
                pixels[idx + 1] = g;
                pixels[idx + 2] = b;
                pixels[idx + 3] = 255;
            }
        }

        Some(PluginFrame::from_arc(Arc::new(pixels), width, height))
    }

    fn handle_event(&mut self, event: &Event, _area: Rect) -> PluginEventResult {
        if let Event::Key(KeyEvent {
            code: KeyCode::Char('r'),
            ..
        }) = event
        {
            self.frame_tick = 0;
            self.last_render_mode_flip = Instant::now();
            return PluginEventResult::Consumed;
        }

        PluginEventResult::Ignored
    }

    fn tick(&mut self) -> bool {
        if self.descriptor.render_mode == PluginRenderMode::KittyGraphics {
            self.frame_tick = self.frame_tick.wrapping_add(1);
            return true;
        }

        // Keep text plugins mildly dynamic so status updates remain visible.
        if self.last_render_mode_flip.elapsed() > Duration::from_secs(1) {
            self.last_render_mode_flip = Instant::now();
            return true;
        }

        false
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
