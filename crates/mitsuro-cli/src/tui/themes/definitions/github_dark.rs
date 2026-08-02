use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// GitHub Dark theme
pub fn github_dark() -> Theme {
    ThemeBuilder::new("github-dark", "GitHub Dark")
        .core_colors(CoreColors::new(
            Color::Rgb(13, 17, 23),
            Color::Rgb(48, 54, 61),
            Color::Rgb(88, 166, 255),
            Color::Rgb(188, 140, 255),
            Color::Rgb(201, 209, 217),
            Color::Rgb(63, 185, 80),
            Color::Rgb(139, 148, 158),
        ))
        .mode_colors(
            Color::Rgb(63, 185, 80),
            Color::Rgb(255, 123, 114),
            Color::Rgb(188, 140, 255),
            Color::Rgb(219, 171, 9),
            Color::Rgb(88, 166, 255),
        )
        .special_colors(
            Color::Rgb(219, 171, 9),
            Color::Rgb(248, 81, 73),
            Color::Rgb(22, 27, 34),
        )
        .ui_colors(
            Color::Rgb(188, 140, 255),
            Color::Rgb(33, 39, 46),
            Color::Rgb(201, 209, 217),
        )
        .message_colors(
            Color::Rgb(63, 185, 80),
            Color::Rgb(188, 140, 255),
            Color::Rgb(219, 171, 9),
            Color::Rgb(88, 166, 255),
        )
        .status_colors(Color::Rgb(88, 166, 255), Color::Rgb(188, 140, 255))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(22, 27, 34);
            theme.input_placeholder_color = Color::Rgb(139, 148, 158);
            theme.input_border_color = Color::Rgb(48, 54, 61);
            theme.user_msg_bg_color = Color::Rgb(27, 32, 39);
            theme.assistant_msg_bg_color = Color::Rgb(29, 32, 41);
            theme.system_msg_bg_color = Color::Rgb(30, 32, 39);
            theme.tool_msg_bg_color = Color::Rgb(27, 34, 42);
            theme.status_bar_bg_color = Color::Rgb(22, 27, 34);
            theme.scrollbar_bg_color = Color::Rgb(32, 37, 46);
            theme.scrollbar_fg_color = Color::Rgb(48, 54, 61);
            theme.scrollbar_hover_color = Color::Rgb(188, 140, 255);
            theme.logo_primary_color = Color::Rgb(88, 166, 255);
            theme.logo_secondary_color = Color::Rgb(188, 140, 255);
            theme.animation_color = Color::Rgb(88, 166, 255);
            theme.processing_color = Color::Rgb(219, 171, 9);
            theme.highlight_color = Color::Rgb(219, 171, 9);
            theme.bubble_color = Color::Rgb(88, 166, 255);
            theme.token_low_color = Color::Rgb(63, 185, 80);
            theme.token_medium_color = Color::Rgb(219, 171, 9);
            theme.token_high_color = Color::Rgb(255, 123, 114);
            theme.token_critical_color = Color::Rgb(248, 81, 73);
        })
        .build()
}
