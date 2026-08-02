//! Shared containment surface with clip-safe borders.

use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear},
    Frame,
};

use crate::tui_v2::{
    layout::snapshot::intersect,
    model::capability::CapabilityProfile,
    presentation::{symbols::ASCII_BORDER, theme::SemanticTheme},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceLevel {
    Canvas,
    Subtle,
    Elevated,
    Strong,
    /// Borders/content only — never paints a fill (composer message bar).
    Transparent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BorderMode {
    None,
    Divider,
    Full,
}

#[derive(Clone, Copy, Debug)]
pub struct Surface<'a> {
    pub level: SurfaceLevel,
    pub border: BorderMode,
    pub focused: bool,
    pub title: Option<&'a str>,
    pub footer: Option<&'a str>,
}

impl Default for Surface<'_> {
    fn default() -> Self {
        Self {
            level: SurfaceLevel::Subtle,
            border: BorderMode::None,
            focused: false,
            title: None,
            footer: None,
        }
    }
}

impl Surface<'_> {
    pub fn render(
        self,
        frame: &mut Frame,
        area: Rect,
        theme: SemanticTheme,
        capability: CapabilityProfile,
    ) -> Rect {
        let area = intersect(area, frame.area());
        if area.is_empty() {
            return area;
        }
        let transparent = matches!(self.level, SurfaceLevel::Transparent);
        // Continuity: product panels share the page fill; borders define containment.
        // Elevated/Strong stay as semantic levels but no longer paint a stepped plate.
        let background = match self.level {
            SurfaceLevel::Canvas
            | SurfaceLevel::Subtle
            | SurfaceLevel::Elevated
            | SurfaceLevel::Strong => theme.surface,
            SurfaceLevel::Transparent => theme.canvas,
        };
        let borders = match self.border {
            BorderMode::None => Borders::NONE,
            BorderMode::Divider => Borders::BOTTOM,
            BorderMode::Full => Borders::ALL,
        };
        let border_set = if capability.supports_rounded_borders() {
            ratatui::symbols::border::ROUNDED
        } else {
            ASCII_BORDER
        };
        let mut block = Block::default()
            .borders(borders)
            .border_set(border_set)
            .border_style(Style::default().fg(if self.focused {
                theme.border_focused
            } else {
                theme.border
            }));
        // Transparent surfaces keep border-only styling (no fill, no Clear wipe).
        if transparent {
            block = block.style(Style::default().fg(theme.foreground));
        } else {
            block = block.style(Style::default().bg(background).fg(theme.foreground));
        }
        if let Some(title) = self.title {
            block = block.title(title);
        }
        if let Some(footer) = self.footer {
            block = block.title_bottom(footer);
        }
        let inner = block.inner(area);

        if !transparent {
            frame.render_widget(Clear, area);
        }
        frame.render_widget(block, area);
        inner
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::tui_v2::{
        model::capability::{CapabilityProfile, ColorDepth, GlyphMode},
        presentation::theme::{SemanticTheme, ThemeKind},
    };

    use super::*;

    #[test]
    fn partially_offscreen_surface_is_clipped_before_border_rendering() {
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).expect("terminal");
        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Ascii,
            color_depth: ColorDepth::Monochrome,
        };
        let theme = SemanticTheme::resolve(ThemeKind::MitsuroDark, capability.color_depth);

        terminal
            .draw(|frame| {
                Surface {
                    level: SurfaceLevel::Elevated,
                    border: BorderMode::Full,
                    focused: true,
                    title: None,
                    footer: None,
                }
                .render(frame, Rect::new(18, 6, 10, 10), theme, capability);
            })
            .expect("clipped render");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.area, Rect::new(0, 0, 20, 8));
        assert_eq!(buffer.cell((18, 6)).expect("top-left").symbol(), "+");
        assert_eq!(buffer.cell((19, 7)).expect("bottom-right").symbol(), "+");
    }
}
