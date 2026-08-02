use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Gruvbox Dark theme
pub fn gruvbox_dark() -> Theme {
    ThemeBuilder::new("gruvbox-dark", "Gruvbox Dark")
        .core_colors(CoreColors::new(
            Color::Rgb(40, 40, 40),
            Color::Rgb(80, 73, 69),
            Color::Rgb(142, 192, 124),
            Color::Rgb(211, 134, 155),
            Color::Rgb(235, 219, 178),
            Color::Rgb(184, 187, 38),
            Color::Rgb(146, 131, 116),
        ))
        .mode_colors(
            Color::Rgb(184, 187, 38),
            Color::Rgb(211, 134, 155),
            Color::Rgb(177, 98, 134),
            Color::Rgb(254, 128, 25),
            Color::Rgb(177, 98, 134),
        )
        .special_colors(
            Color::Rgb(254, 128, 25),
            Color::Rgb(251, 73, 52),
            Color::Rgb(29, 32, 33),
        )
        .ui_colors(
            Color::Rgb(211, 134, 155),
            Color::Rgb(60, 56, 54),
            Color::Rgb(235, 219, 178),
        )
        .message_colors(
            Color::Rgb(142, 192, 124),
            Color::Rgb(211, 134, 155),
            Color::Rgb(254, 128, 25),
            Color::Rgb(184, 187, 38),
        )
        .status_colors(Color::Rgb(131, 165, 152), Color::Rgb(211, 134, 155))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(50, 48, 47);
            theme.input_placeholder_color = Color::Rgb(146, 131, 116);
            theme.input_border_color = Color::Rgb(80, 73, 69);
            theme.user_msg_bg_color = Color::Rgb(50, 50, 50);
            theme.assistant_msg_bg_color = Color::Rgb(52, 50, 50);
            theme.system_msg_bg_color = Color::Rgb(54, 50, 48);
            theme.tool_msg_bg_color = Color::Rgb(50, 52, 50);
            theme.status_bar_bg_color = Color::Rgb(45, 45, 45);
            theme.scrollbar_bg_color = Color::Rgb(50, 50, 50);
            theme.scrollbar_fg_color = Color::Rgb(80, 73, 69);
            theme.scrollbar_hover_color = Color::Rgb(211, 134, 155);
            theme.logo_primary_color = Color::Rgb(142, 192, 124);
            theme.logo_secondary_color = Color::Rgb(211, 134, 155);
            theme.animation_color = Color::Rgb(142, 192, 124);
            theme.processing_color = Color::Rgb(254, 128, 25);
            theme.highlight_color = Color::Rgb(254, 128, 25);
            theme.bubble_color = Color::Rgb(131, 165, 152);
            theme.token_low_color = Color::Rgb(184, 187, 38);
            theme.token_medium_color = Color::Rgb(254, 128, 25);
            theme.token_high_color = Color::Rgb(211, 134, 155);
            theme.token_critical_color = Color::Rgb(251, 73, 52);
        })
        .build()
}
