use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Solarized Dark theme
pub fn solarized_dark() -> Theme {
    ThemeBuilder::new("solarized-dark", "Solarized Dark")
        .core_colors(CoreColors::new(
            Color::Rgb(0, 43, 54),
            Color::Rgb(88, 110, 117),
            Color::Rgb(38, 139, 210),
            Color::Rgb(108, 113, 196),
            Color::Rgb(131, 148, 150),
            Color::Rgb(133, 153, 0),
            Color::Rgb(88, 110, 117),
        ))
        .mode_colors(
            Color::Rgb(133, 153, 0),
            Color::Rgb(211, 54, 130),
            Color::Rgb(108, 113, 196),
            Color::Rgb(203, 75, 22),
            Color::Rgb(108, 113, 196),
        )
        .special_colors(
            Color::Rgb(181, 137, 0),
            Color::Rgb(220, 50, 47),
            Color::Rgb(7, 54, 66),
        )
        .ui_colors(
            Color::Rgb(108, 113, 196),
            Color::Rgb(10, 66, 81),
            Color::Rgb(147, 161, 161),
        )
        .message_colors(
            Color::Rgb(133, 153, 0),
            Color::Rgb(108, 113, 196),
            Color::Rgb(181, 137, 0),
            Color::Rgb(38, 139, 210),
        )
        .status_colors(Color::Rgb(38, 139, 210), Color::Rgb(108, 113, 196))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(7, 54, 66);
            theme.input_placeholder_color = Color::Rgb(88, 110, 117);
            theme.input_border_color = Color::Rgb(7, 54, 66);
            theme.user_msg_bg_color = Color::Rgb(12, 59, 71);
            theme.assistant_msg_bg_color = Color::Rgb(14, 59, 73);
            theme.system_msg_bg_color = Color::Rgb(15, 59, 71);
            theme.tool_msg_bg_color = Color::Rgb(12, 61, 74);
            theme.status_bar_bg_color = Color::Rgb(7, 54, 66);
            theme.scrollbar_bg_color = Color::Rgb(17, 64, 78);
            theme.scrollbar_fg_color = Color::Rgb(7, 54, 66);
            theme.scrollbar_hover_color = Color::Rgb(108, 113, 196);
            theme.logo_primary_color = Color::Rgb(38, 139, 210);
            theme.logo_secondary_color = Color::Rgb(108, 113, 196);
            theme.animation_color = Color::Rgb(38, 139, 210);
            theme.processing_color = Color::Rgb(181, 137, 0);
            theme.highlight_color = Color::Rgb(181, 137, 0);
            theme.bubble_color = Color::Rgb(38, 139, 210);
            theme.token_low_color = Color::Rgb(133, 153, 0);
            theme.token_medium_color = Color::Rgb(181, 137, 0);
            theme.token_high_color = Color::Rgb(211, 54, 130);
            theme.token_critical_color = Color::Rgb(220, 50, 47);
        })
        .build()
}
