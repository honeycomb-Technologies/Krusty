use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Cobalt 2 theme
pub fn cobalt2() -> Theme {
    ThemeBuilder::new("cobalt2", "Cobalt 2")
        .core_colors(CoreColors::new(
            Color::Rgb(25, 53, 73),
            Color::Rgb(35, 79, 109),
            Color::Rgb(0, 136, 255),
            Color::Rgb(128, 255, 187),
            Color::Rgb(255, 255, 255),
            Color::Rgb(58, 217, 0),
            Color::Rgb(173, 183, 201),
        ))
        .mode_colors(
            Color::Rgb(58, 217, 0),
            Color::Rgb(128, 255, 187),
            Color::Rgb(128, 255, 187),
            Color::Rgb(255, 198, 0),
            Color::Rgb(128, 255, 187),
        )
        .special_colors(
            Color::Rgb(255, 198, 0),
            Color::Rgb(255, 0, 0),
            Color::Rgb(16, 41, 58),
        )
        .ui_colors(
            Color::Rgb(128, 255, 187),
            Color::Rgb(41, 80, 108),
            Color::Rgb(255, 255, 255),
        )
        .message_colors(
            Color::Rgb(58, 217, 0),
            Color::Rgb(128, 255, 187),
            Color::Rgb(255, 198, 0),
            Color::Rgb(0, 136, 255),
        )
        .status_colors(Color::Rgb(0, 136, 255), Color::Rgb(128, 255, 187))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(31, 70, 98);
            theme.input_placeholder_color = Color::Rgb(173, 183, 201);
            theme.input_border_color = Color::Rgb(31, 70, 98);
            theme.user_msg_bg_color = Color::Rgb(36, 75, 103);
            theme.assistant_msg_bg_color = Color::Rgb(38, 75, 105);
            theme.system_msg_bg_color = Color::Rgb(39, 75, 103);
            theme.tool_msg_bg_color = Color::Rgb(36, 77, 106);
            theme.status_bar_bg_color = Color::Rgb(31, 70, 98);
            theme.scrollbar_bg_color = Color::Rgb(41, 80, 110);
            theme.scrollbar_fg_color = Color::Rgb(31, 70, 98);
            theme.scrollbar_hover_color = Color::Rgb(42, 255, 223);
            theme.logo_primary_color = Color::Rgb(0, 136, 255);
            theme.logo_secondary_color = Color::Rgb(42, 255, 223);
            theme.animation_color = Color::Rgb(0, 136, 255);
            theme.processing_color = Color::Rgb(255, 198, 0);
            theme.highlight_color = Color::Rgb(255, 198, 0);
            theme.bubble_color = Color::Rgb(42, 255, 223);
            theme.token_low_color = Color::Rgb(158, 255, 128);
            theme.token_medium_color = Color::Rgb(255, 198, 0);
            theme.token_high_color = Color::Rgb(42, 255, 223);
            theme.token_critical_color = Color::Rgb(255, 0, 136);
        })
        .build()
}
