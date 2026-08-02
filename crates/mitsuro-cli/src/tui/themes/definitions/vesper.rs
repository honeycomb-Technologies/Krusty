use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Vesper theme
pub fn vesper() -> Theme {
    ThemeBuilder::new("vesper", "Vesper")
        .core_colors(CoreColors::new(
            Color::Rgb(26, 26, 28),
            Color::Rgb(40, 40, 40),
            Color::Rgb(255, 199, 153),
            Color::Rgb(255, 199, 153),
            Color::Rgb(200, 200, 200),
            Color::Rgb(153, 255, 228),
            Color::Rgb(160, 160, 160),
        ))
        .mode_colors(
            Color::Rgb(153, 255, 228),
            Color::Rgb(255, 199, 153),
            Color::Rgb(255, 199, 153),
            Color::Rgb(255, 199, 153),
            Color::Rgb(255, 199, 153),
        )
        .special_colors(
            Color::Rgb(255, 199, 153),
            Color::Rgb(255, 128, 128),
            Color::Rgb(16, 16, 16),
        )
        .ui_colors(
            Color::Rgb(255, 199, 153),
            Color::Rgb(30, 30, 30),
            Color::Rgb(200, 200, 200),
        )
        .message_colors(
            Color::Rgb(153, 255, 228),
            Color::Rgb(255, 199, 153),
            Color::Rgb(255, 199, 153),
            Color::Rgb(255, 199, 153),
        )
        .status_colors(Color::Rgb(255, 199, 153), Color::Rgb(255, 199, 153))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(16, 16, 16);
            theme.input_placeholder_color = Color::Rgb(160, 160, 160);
            theme.input_border_color = Color::Rgb(40, 40, 40);
            theme.user_msg_bg_color = Color::Rgb(21, 21, 21);
            theme.assistant_msg_bg_color = Color::Rgb(23, 21, 23);
            theme.system_msg_bg_color = Color::Rgb(24, 21, 21);
            theme.tool_msg_bg_color = Color::Rgb(21, 23, 24);
            theme.status_bar_bg_color = Color::Rgb(16, 16, 16);
            theme.scrollbar_bg_color = Color::Rgb(26, 26, 28);
            theme.scrollbar_fg_color = Color::Rgb(40, 40, 40);
            theme.scrollbar_hover_color = Color::Rgb(255, 199, 153);
            theme.logo_primary_color = Color::Rgb(255, 199, 153);
            theme.logo_secondary_color = Color::Rgb(255, 199, 153);
            theme.animation_color = Color::Rgb(255, 199, 153);
            theme.processing_color = Color::Rgb(255, 199, 153);
            theme.highlight_color = Color::Rgb(255, 199, 153);
            theme.bubble_color = Color::Rgb(255, 199, 153);
            theme.token_low_color = Color::Rgb(153, 255, 228);
            theme.token_medium_color = Color::Rgb(255, 199, 153);
            theme.token_high_color = Color::Rgb(255, 199, 153);
            theme.token_critical_color = Color::Rgb(255, 128, 128);
        })
        .build()
}
