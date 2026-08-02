use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Matrix theme
pub fn matrix() -> Theme {
    ThemeBuilder::new("matrix", "Matrix")
        .core_colors(CoreColors::new(
            Color::Rgb(10, 30, 12),
            Color::Rgb(0, 100, 0),
            Color::Rgb(0, 255, 0),
            Color::Rgb(0, 200, 0),
            Color::Rgb(0, 255, 0),
            Color::Rgb(50, 255, 50),
            Color::Rgb(0, 128, 0),
        ))
        .mode_colors(
            Color::Rgb(0, 255, 0),
            Color::Rgb(0, 200, 0),
            Color::Rgb(0, 150, 0),
            Color::Rgb(50, 255, 50),
            Color::Rgb(0, 255, 0),
        )
        .special_colors(
            Color::Rgb(0, 255, 100),
            Color::Rgb(100, 255, 100),
            Color::Rgb(0, 20, 0),
        )
        .ui_colors(
            Color::Rgb(0, 200, 0),
            Color::Rgb(0, 60, 0),
            Color::Rgb(0, 255, 0),
        )
        .message_colors(
            Color::Rgb(50, 255, 50),
            Color::Rgb(0, 200, 0),
            Color::Rgb(0, 255, 100),
            Color::Rgb(0, 255, 0),
        )
        .status_colors(Color::Rgb(0, 255, 0), Color::Rgb(0, 200, 0))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(0, 20, 0);
            theme.input_placeholder_color = Color::Rgb(0, 128, 0);
            theme.input_border_color = Color::Rgb(0, 100, 0);
            theme.user_msg_bg_color = Color::Rgb(5, 25, 5);
            theme.assistant_msg_bg_color = Color::Rgb(7, 25, 7);
            theme.system_msg_bg_color = Color::Rgb(8, 25, 5);
            theme.tool_msg_bg_color = Color::Rgb(5, 27, 8);
            theme.status_bar_bg_color = Color::Rgb(0, 20, 0);
            theme.scrollbar_bg_color = Color::Rgb(10, 30, 12);
            theme.scrollbar_fg_color = Color::Rgb(0, 100, 0);
            theme.scrollbar_hover_color = Color::Rgb(0, 200, 0);
            theme.logo_primary_color = Color::Rgb(0, 255, 0);
            theme.logo_secondary_color = Color::Rgb(0, 200, 0);
            theme.animation_color = Color::Rgb(0, 255, 0);
            theme.processing_color = Color::Rgb(0, 255, 100);
            theme.highlight_color = Color::Rgb(0, 255, 100);
            theme.bubble_color = Color::Rgb(0, 255, 0);
            theme.token_low_color = Color::Rgb(50, 255, 50);
            theme.token_medium_color = Color::Rgb(0, 255, 100);
            theme.token_high_color = Color::Rgb(0, 200, 0);
            theme.token_critical_color = Color::Rgb(100, 255, 100);
        })
        .build()
}
