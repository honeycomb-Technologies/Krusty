//! Terminal capability resolution.

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GlyphMode {
    Unicode,
    Ascii,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Ansi16,
    Monochrome,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityProfile {
    pub glyph_mode: GlyphMode,
    pub color_depth: ColorDepth,
}

impl CapabilityProfile {
    pub fn detect() -> Self {
        let term = std::env::var("TERM").ok();
        let color_term = std::env::var("COLORTERM").ok();
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let force_ascii = std::env::var("MITSURO_ASCII")
            .ok()
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));

        Self::from_environment(
            term.as_deref(),
            color_term.as_deref(),
            no_color,
            force_ascii,
        )
    }

    pub fn from_environment(
        term: Option<&str>,
        color_term: Option<&str>,
        no_color: bool,
        force_ascii: bool,
    ) -> Self {
        let term = term.unwrap_or_default().to_ascii_lowercase();
        let color_term = color_term.unwrap_or_default().to_ascii_lowercase();

        let glyph_mode = if force_ascii || term == "dumb" {
            GlyphMode::Ascii
        } else {
            GlyphMode::Unicode
        };

        let color_depth = if no_color {
            ColorDepth::Monochrome
        } else if color_term.contains("truecolor") || color_term.contains("24bit") {
            ColorDepth::TrueColor
        } else if term.contains("256color") {
            ColorDepth::Ansi256
        } else {
            ColorDepth::Ansi16
        };

        Self {
            glyph_mode,
            color_depth,
        }
    }

    pub const fn supports_rounded_borders(self) -> bool {
        matches!(self.glyph_mode, GlyphMode::Unicode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_unicode_profile_is_selected_before_layout() {
        let profile = CapabilityProfile::from_environment(
            Some("xterm-256color"),
            Some("truecolor"),
            false,
            false,
        );

        assert_eq!(profile.glyph_mode, GlyphMode::Unicode);
        assert_eq!(profile.color_depth, ColorDepth::TrueColor);
    }

    #[test]
    fn dumb_terminal_and_no_color_fail_closed() {
        let profile = CapabilityProfile::from_environment(Some("dumb"), None, true, false);

        assert_eq!(profile.glyph_mode, GlyphMode::Ascii);
        assert_eq!(profile.color_depth, ColorDepth::Monochrome);
        assert!(!profile.supports_rounded_borders());
    }

    #[test]
    fn common_terminal_matrix_resolves_to_bounded_capabilities() {
        let cases = [
            (
                "Ghostty",
                "xterm-ghostty",
                Some("truecolor"),
                ColorDepth::TrueColor,
            ),
            (
                "Kitty",
                "xterm-kitty",
                Some("truecolor"),
                ColorDepth::TrueColor,
            ),
            (
                "WezTerm",
                "xterm-256color",
                Some("truecolor"),
                ColorDepth::TrueColor,
            ),
            (
                "Apple Terminal",
                "xterm-256color",
                None,
                ColorDepth::Ansi256,
            ),
            (
                "iTerm2",
                "xterm-256color",
                Some("truecolor"),
                ColorDepth::TrueColor,
            ),
            (
                "Alacritty",
                "alacritty",
                Some("truecolor"),
                ColorDepth::TrueColor,
            ),
            ("tmux", "screen-256color", None, ColorDepth::Ansi256),
            ("SSH Linux", "xterm-256color", None, ColorDepth::Ansi256),
            ("ANSI 16", "xterm", None, ColorDepth::Ansi16),
        ];

        for (label, term, color_term, expected) in cases {
            let profile = CapabilityProfile::from_environment(Some(term), color_term, false, false);
            assert_eq!(profile.glyph_mode, GlyphMode::Unicode, "{label}");
            assert_eq!(profile.color_depth, expected, "{label}");
        }
    }
}
