use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Kanagawa theme
pub fn kanagawa() -> Theme {
    ThemeBuilder::new("kanagawa", "Kanagawa")
        .core_colors(CoreColors::new(
            Color::Rgb(31, 31, 40),
            Color::Rgb(54, 54, 70),
            Color::Rgb(126, 156, 216),
            Color::Rgb(210, 126, 153),
            Color::Rgb(220, 215, 186),
            Color::Rgb(152, 187, 108),
            Color::Rgb(114, 113, 105),
        ))
        .mode_colors(
            Color::Rgb(152, 187, 108),
            Color::Rgb(210, 126, 153),
            Color::Rgb(210, 126, 153),
            Color::Rgb(215, 166, 87),
            Color::Rgb(210, 126, 153),
        )
        .special_colors(
            Color::Rgb(215, 166, 87),
            Color::Rgb(232, 36, 36),
            Color::Rgb(26, 26, 34),
        )
        .ui_colors(
            Color::Rgb(210, 126, 153),
            Color::Rgb(64, 64, 80),
            Color::Rgb(220, 215, 186),
        )
        .message_colors(
            Color::Rgb(152, 187, 108),
            Color::Rgb(210, 126, 153),
            Color::Rgb(215, 166, 87),
            Color::Rgb(126, 156, 216),
        )
        .status_colors(Color::Rgb(126, 156, 216), Color::Rgb(210, 126, 153))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(54, 54, 70);
            theme.input_placeholder_color = Color::Rgb(114, 113, 105);
            theme.input_border_color = Color::Rgb(84, 84, 109);
            theme.user_msg_bg_color = Color::Rgb(59, 59, 75);
            theme.assistant_msg_bg_color = Color::Rgb(61, 59, 77);
            theme.system_msg_bg_color = Color::Rgb(62, 59, 75);
            theme.tool_msg_bg_color = Color::Rgb(59, 61, 78);
            theme.status_bar_bg_color = Color::Rgb(54, 54, 70);
            theme.scrollbar_bg_color = Color::Rgb(64, 64, 82);
            theme.scrollbar_fg_color = Color::Rgb(84, 84, 109);
            theme.scrollbar_hover_color = Color::Rgb(210, 126, 153);
            theme.logo_primary_color = Color::Rgb(126, 156, 216);
            theme.logo_secondary_color = Color::Rgb(210, 126, 153);
            theme.animation_color = Color::Rgb(126, 156, 216);
            theme.processing_color = Color::Rgb(215, 166, 87);
            theme.highlight_color = Color::Rgb(215, 166, 87);
            theme.bubble_color = Color::Rgb(126, 156, 216);
            theme.token_low_color = Color::Rgb(152, 187, 108);
            theme.token_medium_color = Color::Rgb(215, 166, 87);
            theme.token_high_color = Color::Rgb(210, 126, 153);
            theme.token_critical_color = Color::Rgb(232, 36, 36);
        })
        .build()
}
