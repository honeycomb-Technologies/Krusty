use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Catppuccin Mocha theme
pub fn catppuccin_mocha() -> Theme {
    ThemeBuilder::new("catppuccin-mocha", "Catppuccin Mocha")
        .core_colors(CoreColors::new(
            Color::Rgb(30, 30, 46),
            Color::Rgb(88, 91, 112),
            Color::Rgb(137, 220, 235),
            Color::Rgb(203, 166, 247),
            Color::Rgb(205, 214, 244),
            Color::Rgb(166, 227, 161),
            Color::Rgb(108, 112, 134),
        ))
        .mode_colors(
            Color::Rgb(166, 227, 161),
            Color::Rgb(245, 194, 231),
            Color::Rgb(203, 166, 247),
            Color::Rgb(249, 226, 175),
            Color::Rgb(203, 166, 247),
        )
        .special_colors(
            Color::Rgb(249, 226, 175),
            Color::Rgb(243, 139, 168),
            Color::Rgb(24, 24, 37),
        )
        .ui_colors(
            Color::Rgb(203, 166, 247),
            Color::Rgb(50, 52, 75),
            Color::Rgb(205, 214, 244),
        )
        .message_colors(
            Color::Rgb(166, 227, 161),
            Color::Rgb(203, 166, 247),
            Color::Rgb(249, 226, 175),
            Color::Rgb(243, 139, 168),
        )
        .status_colors(Color::Rgb(137, 180, 250), Color::Rgb(203, 166, 247))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(36, 36, 52);
            theme.input_placeholder_color = Color::Rgb(108, 112, 134);
            theme.input_border_color = Color::Rgb(88, 91, 112);
            theme.user_msg_bg_color = Color::Rgb(36, 36, 52);
            theme.assistant_msg_bg_color = Color::Rgb(38, 36, 52);
            theme.system_msg_bg_color = Color::Rgb(40, 36, 50);
            theme.tool_msg_bg_color = Color::Rgb(36, 38, 52);
            theme.status_bar_bg_color = Color::Rgb(35, 35, 51);
            theme.scrollbar_bg_color = Color::Rgb(40, 40, 56);
            theme.scrollbar_fg_color = Color::Rgb(88, 91, 112);
            theme.scrollbar_hover_color = Color::Rgb(203, 166, 247);
            theme.logo_primary_color = Color::Rgb(137, 220, 235);
            theme.logo_secondary_color = Color::Rgb(203, 166, 247);
            theme.animation_color = Color::Rgb(137, 220, 235);
            theme.processing_color = Color::Rgb(249, 226, 175);
            theme.highlight_color = Color::Rgb(245, 194, 231);
            theme.bubble_color = Color::Rgb(137, 220, 235);
            theme.token_low_color = Color::Rgb(166, 227, 161);
            theme.token_medium_color = Color::Rgb(249, 226, 175);
            theme.token_high_color = Color::Rgb(245, 194, 231);
            theme.token_critical_color = Color::Rgb(243, 139, 168);
        })
        .build()
}
