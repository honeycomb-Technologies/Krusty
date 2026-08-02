use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Monokai theme
pub fn monokai() -> Theme {
    ThemeBuilder::new("monokai", "Monokai")
        .core_colors(CoreColors::new(
            Color::Rgb(39, 40, 34),
            Color::Rgb(73, 72, 62),
            Color::Rgb(102, 217, 239),
            Color::Rgb(249, 38, 114),
            Color::Rgb(248, 248, 242),
            Color::Rgb(166, 226, 46),
            Color::Rgb(117, 113, 94),
        ))
        .mode_colors(
            Color::Rgb(166, 226, 46),
            Color::Rgb(249, 38, 114),
            Color::Rgb(249, 38, 114),
            Color::Rgb(230, 219, 116),
            Color::Rgb(249, 38, 114),
        )
        .special_colors(
            Color::Rgb(230, 219, 116),
            Color::Rgb(249, 38, 114),
            Color::Rgb(62, 61, 50),
        )
        .ui_colors(
            Color::Rgb(166, 226, 46),
            Color::Rgb(75, 74, 63),
            Color::Rgb(248, 248, 242),
        )
        .message_colors(
            Color::Rgb(166, 226, 46),
            Color::Rgb(166, 226, 46),
            Color::Rgb(230, 219, 116),
            Color::Rgb(102, 217, 239),
        )
        .status_colors(Color::Rgb(102, 217, 239), Color::Rgb(166, 226, 46))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(62, 61, 50);
            theme.input_placeholder_color = Color::Rgb(117, 113, 94);
            theme.input_border_color = Color::Rgb(62, 61, 50);
            theme.user_msg_bg_color = Color::Rgb(67, 66, 55);
            theme.assistant_msg_bg_color = Color::Rgb(69, 66, 57);
            theme.system_msg_bg_color = Color::Rgb(70, 66, 55);
            theme.tool_msg_bg_color = Color::Rgb(67, 68, 58);
            theme.status_bar_bg_color = Color::Rgb(62, 61, 50);
            theme.scrollbar_bg_color = Color::Rgb(72, 71, 62);
            theme.scrollbar_fg_color = Color::Rgb(62, 61, 50);
            theme.scrollbar_hover_color = Color::Rgb(166, 226, 46);
            theme.logo_primary_color = Color::Rgb(102, 217, 239);
            theme.logo_secondary_color = Color::Rgb(166, 226, 46);
            theme.animation_color = Color::Rgb(102, 217, 239);
            theme.processing_color = Color::Rgb(230, 219, 116);
            theme.highlight_color = Color::Rgb(230, 219, 116);
            theme.bubble_color = Color::Rgb(102, 217, 239);
            theme.token_low_color = Color::Rgb(166, 226, 46);
            theme.token_medium_color = Color::Rgb(230, 219, 116);
            theme.token_high_color = Color::Rgb(166, 226, 46);
            theme.token_critical_color = Color::Rgb(249, 38, 114);
        })
        .build()
}
