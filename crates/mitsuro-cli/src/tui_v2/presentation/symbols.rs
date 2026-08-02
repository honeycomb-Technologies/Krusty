//! Width-stable terminal symbols selected before layout.

use ratatui::symbols::border;

use crate::tui_v2::model::capability::GlyphMode;

pub const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// Cadence for cascade / running-tool braille (~14 fps).
pub const RUNNING_FRAME_INTERVAL_MS: u64 = 70;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Symbols {
    pub divider: &'static str,
    pub field: &'static str,
    pub pulse: &'static str,
    pub success: &'static str,
    pub failure: &'static str,
    pub warning: &'static str,
    pub paused: &'static str,
    pub collapsed: &'static str,
    pub expanded: &'static str,
    /// Running-tool animation (cascade braille). Terminal status after done:
    /// `success` / `failure` / `warning` — not these frames.
    pub pulse_frames: &'static [&'static str],
    pub wait_frames: &'static [&'static str; 4],
}

impl Symbols {
    pub const fn for_mode(mode: GlyphMode) -> Self {
        match mode {
            GlyphMode::Unicode => Self {
                divider: "─",
                field: "◦",
                pulse: "•",
                success: "✓",
                failure: "×",
                warning: "!",
                paused: "Ⅱ",
                collapsed: "›",
                expanded: "⌄",
                // Cascade: left column fills → right fills → drain.
                pulse_frames: &[
                    "⠁", "⠃", "⠇", "⡇", "⡏", "⡟", "⡿", "⣿", "⢿", "⣻", "⣽", "⣾", "⣷",
                    "⣧", "⣇", "⡄", "⢀", " ",
                ],
                wait_frames: &["·  ", "·· ", "···", " ··"],
            },
            GlyphMode::Ascii => Self {
                divider: "-",
                field: "o",
                pulse: "O",
                success: "+",
                failure: "x",
                warning: "!",
                paused: "=",
                collapsed: ">",
                expanded: "v",
                // Ascii stand-in for cascade fill/drain.
                pulse_frames: &[".", ":", "|", "H", "#", "H", "|", ":", ".", " "],
                wait_frames: &[".  ", ".. ", "...", " .."],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    #[test]
    fn every_selected_symbol_has_a_fixed_measured_width() {
        for mode in [GlyphMode::Unicode, GlyphMode::Ascii] {
            let symbols = Symbols::for_mode(mode);
            for symbol in [
                symbols.field,
                symbols.pulse,
                symbols.success,
                symbols.failure,
                symbols.warning,
                symbols.paused,
                symbols.collapsed,
                symbols.expanded,
            ] {
                assert_eq!(UnicodeWidthStr::width(symbol), 1, "{mode:?}: {symbol:?}");
            }
            for frame in symbols.pulse_frames {
                assert_eq!(UnicodeWidthStr::width(*frame), 1);
            }
            for frame in symbols.wait_frames {
                assert_eq!(UnicodeWidthStr::width(*frame), 3);
            }
        }
    }
}
