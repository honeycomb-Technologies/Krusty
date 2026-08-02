use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Night Owl theme
pub fn night_owl() -> Theme {
    ThemeBuilder::new("night-owl", "Night Owl")
        .core_colors(CoreColors::new(
            Color::Rgb(1, 22, 39),
            Color::Rgb(29, 59, 83),
            Color::Rgb(130, 170, 255),
            Color::Rgb(130, 170, 255),
            Color::Rgb(214, 222, 235),
            Color::Rgb(34, 193, 195),
            Color::Rgb(99, 119, 119),
        ))
        .mode_colors(
            Color::Rgb(34, 193, 195),
            Color::Rgb(255, 203, 107),
            Color::Rgb(130, 170, 255),
            Color::Rgb(247, 127, 190),
            Color::Rgb(130, 170, 255),
        )
        .special_colors(
            Color::Rgb(255, 203, 107),
            Color::Rgb(239, 83, 80),
            Color::Rgb(1, 29, 51),
        )
        .ui_colors(
            Color::Rgb(130, 170, 255),
            Color::Rgb(15, 43, 68),
            Color::Rgb(214, 222, 235),
        )
        .message_colors(
            Color::Rgb(34, 193, 195),
            Color::Rgb(130, 170, 255),
            Color::Rgb(255, 203, 107),
            Color::Rgb(127, 219, 202),
        )
        .status_colors(Color::Rgb(127, 219, 202), Color::Rgb(130, 170, 255))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(1, 29, 51);
            theme.input_placeholder_color = Color::Rgb(87, 109, 143);
            theme.input_border_color = Color::Rgb(5, 63, 88);
            theme.user_msg_bg_color = Color::Rgb(6, 34, 56);
            theme.assistant_msg_bg_color = Color::Rgb(8, 34, 58);
            theme.system_msg_bg_color = Color::Rgb(9, 34, 56);
            theme.tool_msg_bg_color = Color::Rgb(6, 36, 59);
            theme.status_bar_bg_color = Color::Rgb(1, 29, 51);
            theme.scrollbar_bg_color = Color::Rgb(11, 39, 63);
            theme.scrollbar_fg_color = Color::Rgb(5, 63, 88);
            theme.scrollbar_hover_color = Color::Rgb(130, 170, 255);
            theme.logo_primary_color = Color::Rgb(127, 219, 202);
            theme.logo_secondary_color = Color::Rgb(130, 170, 255);
            theme.animation_color = Color::Rgb(127, 219, 202);
            theme.processing_color = Color::Rgb(255, 203, 107);
            theme.highlight_color = Color::Rgb(255, 203, 107);
            theme.bubble_color = Color::Rgb(127, 219, 202);
            theme.token_low_color = Color::Rgb(34, 193, 195);
            theme.token_medium_color = Color::Rgb(255, 203, 107);
            theme.token_high_color = Color::Rgb(255, 203, 107);
            theme.token_critical_color = Color::Rgb(239, 83, 80);
        })
        .build()
}
