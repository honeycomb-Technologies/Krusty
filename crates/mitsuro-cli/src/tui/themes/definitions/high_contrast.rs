use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// High Contrast theme
pub fn high_contrast() -> Theme {
    ThemeBuilder::new("high-contrast", "High Contrast")
        .core_colors(CoreColors::new(
            Color::Rgb(30, 30, 32),
            Color::Rgb(255, 255, 255),
            Color::Rgb(255, 255, 255),
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 255, 255),
            Color::Rgb(0, 255, 0),
            Color::Rgb(150, 150, 150),
        ))
        .mode_colors(
            Color::Rgb(0, 255, 0),
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 0, 255),
            Color::Rgb(0, 255, 255),
            Color::Rgb(255, 255, 0),
        )
        .special_colors(
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 0, 0),
            Color::Rgb(20, 20, 20),
        )
        .ui_colors(
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 255, 255),
            Color::Rgb(0, 0, 0),
        )
        .message_colors(
            Color::Rgb(0, 255, 0),
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 255, 255),
        )
        .status_colors(Color::Rgb(255, 255, 255), Color::Rgb(255, 255, 0))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(20, 20, 20);
            theme.input_placeholder_color = Color::Rgb(150, 150, 150);
            theme.input_border_color = Color::Rgb(255, 255, 255);
            theme.user_msg_bg_color = Color::Rgb(25, 25, 25);
            theme.assistant_msg_bg_color = Color::Rgb(27, 25, 27);
            theme.system_msg_bg_color = Color::Rgb(28, 25, 25);
            theme.tool_msg_bg_color = Color::Rgb(25, 27, 28);
            theme.status_bar_bg_color = Color::Rgb(20, 20, 20);
            theme.scrollbar_bg_color = Color::Rgb(30, 30, 32);
            theme.scrollbar_fg_color = Color::Rgb(255, 255, 255);
            theme.scrollbar_hover_color = Color::Rgb(255, 255, 0);
            theme.logo_primary_color = Color::Rgb(255, 255, 255);
            theme.logo_secondary_color = Color::Rgb(255, 255, 0);
            theme.animation_color = Color::Rgb(255, 255, 255);
            theme.processing_color = Color::Rgb(255, 255, 0);
            theme.highlight_color = Color::Rgb(255, 255, 0);
            theme.bubble_color = Color::Rgb(255, 255, 0);
            theme.token_low_color = Color::Rgb(0, 255, 0);
            theme.token_medium_color = Color::Rgb(255, 255, 0);
            theme.token_high_color = Color::Rgb(255, 255, 0);
            theme.token_critical_color = Color::Rgb(255, 0, 0);
        })
        .build()
}
