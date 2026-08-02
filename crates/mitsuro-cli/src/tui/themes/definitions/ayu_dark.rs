use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Ayu Dark theme
pub fn ayu_dark() -> Theme {
    ThemeBuilder::new("ayu-dark", "Ayu Dark")
        .core_colors(CoreColors::new(
            Color::Rgb(10, 14, 20),
            Color::Rgb(37, 51, 64),
            Color::Rgb(57, 186, 230),
            Color::Rgb(255, 180, 84),
            Color::Rgb(230, 225, 207),
            Color::Rgb(145, 179, 98),
            Color::Rgb(95, 106, 119),
        ))
        .mode_colors(
            Color::Rgb(134, 179, 85),
            Color::Rgb(255, 180, 91),
            Color::Rgb(242, 151, 24),
            Color::Rgb(57, 186, 230),
            Color::Rgb(255, 180, 91),
        )
        .special_colors(
            Color::Rgb(255, 180, 84),
            Color::Rgb(240, 113, 113),
            Color::Rgb(21, 29, 40),
        )
        .ui_colors(
            Color::Rgb(255, 180, 91),
            Color::Rgb(27, 35, 44),
            Color::Rgb(191, 197, 206),
        )
        .message_colors(
            Color::Rgb(134, 179, 85),
            Color::Rgb(255, 180, 91),
            Color::Rgb(255, 180, 91),
            Color::Rgb(57, 186, 230),
        )
        .status_colors(Color::Rgb(57, 186, 230), Color::Rgb(255, 180, 91))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(15, 20, 30);
            theme.input_placeholder_color = Color::Rgb(92, 103, 115);
            theme.input_border_color = Color::Rgb(37, 51, 64);
            theme.user_msg_bg_color = Color::Rgb(20, 25, 35);
            theme.assistant_msg_bg_color = Color::Rgb(22, 25, 37);
            theme.system_msg_bg_color = Color::Rgb(23, 25, 35);
            theme.tool_msg_bg_color = Color::Rgb(20, 27, 38);
            theme.status_bar_bg_color = Color::Rgb(15, 20, 30);
            theme.scrollbar_bg_color = Color::Rgb(25, 30, 42);
            theme.scrollbar_fg_color = Color::Rgb(37, 51, 64);
            theme.scrollbar_hover_color = Color::Rgb(255, 180, 91);
            theme.logo_primary_color = Color::Rgb(57, 186, 230);
            theme.logo_secondary_color = Color::Rgb(255, 180, 91);
            theme.animation_color = Color::Rgb(57, 186, 230);
            theme.processing_color = Color::Rgb(255, 180, 91);
            theme.highlight_color = Color::Rgb(255, 180, 91);
            theme.bubble_color = Color::Rgb(57, 186, 230);
            theme.token_low_color = Color::Rgb(134, 179, 85);
            theme.token_medium_color = Color::Rgb(255, 180, 91);
            theme.token_high_color = Color::Rgb(255, 180, 91);
            theme.token_critical_color = Color::Rgb(240, 113, 113);
        })
        .build()
}
