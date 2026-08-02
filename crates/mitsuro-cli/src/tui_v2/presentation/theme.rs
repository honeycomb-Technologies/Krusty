//! Semantic Mitsuro terminal themes.
//!
//! Raw terminal colors live only in this adapter. Components consume semantic
//! roles and therefore cannot change geometry or behavior with a theme.

use ratatui::style::Color;

use crate::tui_v2::model::capability::ColorDepth;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ThemeKind {
    #[default]
    MitsuroDark,
    MitsuroLight,
    TerminalAdaptive,
    HighContrast,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticTheme {
    pub canvas: Color,
    pub surface: Color,
    pub surface_elevated: Color,
    pub surface_strong: Color,
    pub foreground: Color,
    pub foreground_muted: Color,
    pub border: Color,
    pub border_focused: Color,
    pub accent: Color,
    pub accent_surface: Color,
    pub identity: Color,
    pub thinking: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub link: Color,
    pub code_surface: Color,
    pub selection_surface: Color,
    pub diff_add: Color,
    pub diff_add_surface: Color,
    pub diff_remove: Color,
    pub diff_remove_surface: Color,
}

impl SemanticTheme {
    pub const fn resolve(kind: ThemeKind, depth: ColorDepth) -> Self {
        if matches!(depth, ColorDepth::Monochrome) {
            return Self::monochrome();
        }

        match kind {
            ThemeKind::MitsuroDark => Self::mitsuro_dark(depth),
            ThemeKind::MitsuroLight => Self::mitsuro_light(depth),
            ThemeKind::TerminalAdaptive => Self::terminal_adaptive(),
            ThemeKind::HighContrast => Self::high_contrast(depth),
        }
    }

    pub const fn mitsuro_dark(depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::TrueColor => Self {
                // surface matches canvas; elevated/strong match surface (border defines panels).
                canvas: rgb(0x0e0e11),
                surface: rgb(0x0e0e11),
                surface_elevated: rgb(0x0e0e11),
                surface_strong: rgb(0x0e0e11),
                foreground: rgb(0xe8e5ea),
                foreground_muted: rgb(0x9e98a3),
                border: rgb(0x3a383f),
                border_focused: rgb(0x75617e),
                accent: rgb(0x75617e),
                accent_surface: rgb(0x2a222f),
                identity: rgb(0xb89a61),
                thinking: rgb(0x9a82a5),
                success: rgb(0x7f9a86),
                warning: rgb(0xb89a61),
                error: rgb(0xb06f73),
                link: rgb(0x9e91b5),
                code_surface: rgb(0x111216),
                selection_surface: rgb(0x352c3b),
                diff_add: rgb(0x90ad96),
                diff_add_surface: rgb(0x17231b),
                diff_remove: rgb(0xc78286),
                diff_remove_surface: rgb(0x29191c),
            },
            ColorDepth::Ansi256 => Self {
                canvas: Color::Indexed(233),
                surface: Color::Indexed(233),
                surface_elevated: Color::Indexed(233),
                surface_strong: Color::Indexed(233),
                foreground: Color::Indexed(255),
                foreground_muted: Color::Indexed(247),
                border: Color::Indexed(239),
                border_focused: Color::Indexed(96),
                accent: Color::Indexed(96),
                accent_surface: Color::Indexed(236),
                identity: Color::Indexed(137),
                thinking: Color::Indexed(103),
                success: Color::Indexed(108),
                warning: Color::Indexed(137),
                error: Color::Indexed(131),
                link: Color::Indexed(139),
                code_surface: Color::Indexed(233),
                selection_surface: Color::Indexed(237),
                diff_add: Color::Indexed(108),
                diff_add_surface: Color::Indexed(22),
                diff_remove: Color::Indexed(174),
                diff_remove_surface: Color::Indexed(52),
            },
            ColorDepth::Ansi16 => Self {
                canvas: Color::Black,
                surface: Color::Black,
                surface_elevated: Color::Black,
                surface_strong: Color::Black,
                foreground: Color::White,
                foreground_muted: Color::Gray,
                border: Color::DarkGray,
                border_focused: Color::Magenta,
                accent: Color::Magenta,
                accent_surface: Color::Black,
                identity: Color::Yellow,
                thinking: Color::LightMagenta,
                success: Color::Green,
                warning: Color::Yellow,
                error: Color::Red,
                link: Color::LightBlue,
                code_surface: Color::Black,
                selection_surface: Color::DarkGray,
                diff_add: Color::LightGreen,
                diff_add_surface: Color::Black,
                diff_remove: Color::LightRed,
                diff_remove_surface: Color::Black,
            },
            ColorDepth::Monochrome => Self::monochrome(),
        }
    }

    pub const fn mitsuro_light(depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::TrueColor => Self {
                // surface matches canvas; elevated/strong match surface (border defines panels).
                canvas: rgb(0xf4f1ed),
                surface: rgb(0xf4f1ed),
                surface_elevated: rgb(0xf4f1ed),
                surface_strong: rgb(0xf4f1ed),
                foreground: rgb(0x242127),
                foreground_muted: rgb(0x706a73),
                border: rgb(0xbdb5bd),
                border_focused: rgb(0x67546f),
                accent: rgb(0x67546f),
                accent_surface: rgb(0xe2d9e7),
                identity: rgb(0x8f6f35),
                thinking: rgb(0x75617e),
                success: rgb(0x4d7256),
                warning: rgb(0x8f6f35),
                error: rgb(0x98565b),
                link: rgb(0x574b86),
                code_surface: rgb(0xe9e5df),
                selection_surface: rgb(0xd9cfe0),
                diff_add: rgb(0x426d4c),
                diff_add_surface: rgb(0xdbe8dc),
                diff_remove: rgb(0x934f54),
                diff_remove_surface: rgb(0xefd9d9),
            },
            ColorDepth::Ansi256 => Self {
                canvas: Color::Indexed(255),
                surface: Color::Indexed(255),
                surface_elevated: Color::Indexed(255),
                surface_strong: Color::Indexed(255),
                foreground: Color::Indexed(234),
                foreground_muted: Color::Indexed(241),
                border: Color::Indexed(249),
                border_focused: Color::Indexed(96),
                accent: Color::Indexed(96),
                accent_surface: Color::Indexed(253),
                identity: Color::Indexed(136),
                thinking: Color::Indexed(96),
                success: Color::Indexed(65),
                warning: Color::Indexed(136),
                error: Color::Indexed(131),
                link: Color::Indexed(60),
                code_surface: Color::Indexed(254),
                selection_surface: Color::Indexed(189),
                diff_add: Color::Indexed(65),
                diff_add_surface: Color::Indexed(194),
                diff_remove: Color::Indexed(131),
                diff_remove_surface: Color::Indexed(224),
            },
            ColorDepth::Ansi16 => Self {
                canvas: Color::White,
                surface: Color::White,
                surface_elevated: Color::White,
                surface_strong: Color::White,
                foreground: Color::Black,
                foreground_muted: Color::DarkGray,
                border: Color::Gray,
                border_focused: Color::Magenta,
                accent: Color::Magenta,
                accent_surface: Color::White,
                identity: Color::Yellow,
                thinking: Color::Magenta,
                success: Color::Green,
                warning: Color::Yellow,
                error: Color::Red,
                link: Color::Blue,
                code_surface: Color::White,
                selection_surface: Color::Gray,
                diff_add: Color::Green,
                diff_add_surface: Color::White,
                diff_remove: Color::Red,
                diff_remove_surface: Color::White,
            },
            ColorDepth::Monochrome => Self::monochrome(),
        }
    }

    const fn terminal_adaptive() -> Self {
        Self {
            canvas: Color::Reset,
            surface: Color::Reset,
            surface_elevated: Color::Reset,
            surface_strong: Color::Reset,
            foreground: Color::Reset,
            foreground_muted: Color::DarkGray,
            border: Color::DarkGray,
            border_focused: Color::Magenta,
            accent: Color::Magenta,
            accent_surface: Color::Reset,
            identity: Color::Yellow,
            thinking: Color::LightMagenta,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            link: Color::Blue,
            code_surface: Color::Reset,
            selection_surface: Color::DarkGray,
            diff_add: Color::Green,
            diff_add_surface: Color::Reset,
            diff_remove: Color::Red,
            diff_remove_surface: Color::Reset,
        }
    }

    const fn high_contrast(depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::TrueColor => Self {
                canvas: rgb(0x000000),
                surface: rgb(0x000000),
                surface_elevated: rgb(0x000000),
                surface_strong: rgb(0x000000),
                foreground: rgb(0xffffff),
                foreground_muted: rgb(0xd4d4d4),
                border: rgb(0xbebebe),
                border_focused: rgb(0xd7a7ff),
                accent: rgb(0xd7a7ff),
                accent_surface: rgb(0x2b1738),
                identity: rgb(0xffd166),
                thinking: rgb(0xd7a7ff),
                success: rgb(0x8ef0a7),
                warning: rgb(0xffd166),
                error: rgb(0xff8f98),
                link: rgb(0x8fc9ff),
                code_surface: rgb(0x0a0a0a),
                selection_surface: rgb(0x4b275d),
                diff_add: rgb(0x8ef0a7),
                diff_add_surface: rgb(0x0d2b16),
                diff_remove: rgb(0xff8f98),
                diff_remove_surface: rgb(0x351114),
            },
            ColorDepth::Ansi256 => Self {
                canvas: Color::Indexed(16),
                surface: Color::Indexed(16),
                surface_elevated: Color::Indexed(16),
                surface_strong: Color::Indexed(16),
                foreground: Color::Indexed(231),
                foreground_muted: Color::Indexed(252),
                border: Color::Indexed(250),
                border_focused: Color::Indexed(183),
                accent: Color::Indexed(183),
                accent_surface: Color::Indexed(53),
                identity: Color::Indexed(221),
                thinking: Color::Indexed(183),
                success: Color::Indexed(120),
                warning: Color::Indexed(221),
                error: Color::Indexed(210),
                link: Color::Indexed(117),
                code_surface: Color::Indexed(232),
                selection_surface: Color::Indexed(54),
                diff_add: Color::Indexed(120),
                diff_add_surface: Color::Indexed(22),
                diff_remove: Color::Indexed(210),
                diff_remove_surface: Color::Indexed(52),
            },
            ColorDepth::Ansi16 => Self::terminal_adaptive(),
            ColorDepth::Monochrome => Self::monochrome(),
        }
    }

    const fn monochrome() -> Self {
        Self {
            canvas: Color::Reset,
            surface: Color::Reset,
            surface_elevated: Color::Reset,
            surface_strong: Color::Reset,
            foreground: Color::Reset,
            foreground_muted: Color::Reset,
            border: Color::Reset,
            border_focused: Color::Reset,
            accent: Color::Reset,
            accent_surface: Color::Reset,
            identity: Color::Reset,
            thinking: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
            link: Color::Reset,
            code_surface: Color::Reset,
            selection_surface: Color::Reset,
            diff_add: Color::Reset,
            diff_add_surface: Color::Reset,
            diff_remove: Color::Reset,
            diff_remove_surface: Color::Reset,
        }
    }
}

const fn rgb(value: u32) -> Color {
    Color::Rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_tokens_match_the_shared_mitsuro_palette() {
        let theme = SemanticTheme::resolve(ThemeKind::MitsuroDark, ColorDepth::TrueColor);

        assert_eq!(theme.canvas, Color::Rgb(0x0e, 0x0e, 0x11));
        assert_eq!(theme.surface, Color::Rgb(0x0e, 0x0e, 0x11));
        assert_eq!(theme.foreground, Color::Rgb(0xe8, 0xe5, 0xea));
        assert_eq!(theme.accent, Color::Rgb(0x75, 0x61, 0x7e));
        assert_eq!(theme.identity, Color::Rgb(0xb8, 0x9a, 0x61));
        // Continuity: page, field, and panel fills match; border defines containment.
        assert_eq!(theme.canvas, theme.surface);
        assert_eq!(theme.surface, theme.surface_elevated);
        assert_eq!(theme.surface, theme.surface_strong);

        let light = SemanticTheme::resolve(ThemeKind::MitsuroLight, ColorDepth::TrueColor);
        assert_eq!(light.canvas, light.surface);
        assert_eq!(light.surface, light.surface_elevated);
        assert_eq!(light.canvas, Color::Rgb(0xf4, 0xf1, 0xed));

        let hc = SemanticTheme::resolve(ThemeKind::HighContrast, ColorDepth::TrueColor);
        assert_eq!(hc.canvas, hc.surface);
        assert_eq!(hc.surface, hc.surface_elevated);
    }

    #[test]
    fn monochrome_removes_every_color_dependency() {
        for kind in [
            ThemeKind::MitsuroDark,
            ThemeKind::MitsuroLight,
            ThemeKind::TerminalAdaptive,
            ThemeKind::HighContrast,
        ] {
            let theme = SemanticTheme::resolve(kind, ColorDepth::Monochrome);
            assert_eq!(theme, SemanticTheme::monochrome());
        }
    }
}
