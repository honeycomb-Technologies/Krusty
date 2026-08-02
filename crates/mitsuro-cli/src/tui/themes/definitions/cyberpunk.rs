use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Cyberpunk theme
pub fn cyberpunk() -> Theme {
    ThemeBuilder::new("cyberpunk", "Cyberpunk")
        .core_colors(CoreColors::new(
            Color::Rgb(38, 10, 57),
            Color::Rgb(139, 0, 255),
            Color::Rgb(255, 0, 247),
            Color::Rgb(0, 255, 255),
            Color::Rgb(200, 200, 255),
            Color::Rgb(0, 255, 179),
            Color::Rgb(100, 80, 130),
        ))
        .mode_colors(
            Color::Rgb(0, 255, 179),
            Color::Rgb(255, 0, 247),
            Color::Rgb(139, 0, 255),
            Color::Rgb(255, 255, 0),
            Color::Rgb(0, 255, 255),
        )
        .special_colors(
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 0, 106),
            Color::Rgb(28, 0, 45),
        )
        .ui_colors(
            Color::Rgb(0, 255, 255),
            Color::Rgb(88, 0, 140),
            Color::Rgb(200, 200, 255),
        )
        .message_colors(
            Color::Rgb(0, 255, 179),
            Color::Rgb(0, 255, 255),
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 0, 247),
        )
        .status_colors(Color::Rgb(255, 0, 247), Color::Rgb(0, 255, 255))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(28, 0, 45);
            theme.input_placeholder_color = Color::Rgb(100, 80, 130);
            theme.input_border_color = Color::Rgb(139, 0, 255);
            theme.user_msg_bg_color = Color::Rgb(33, 5, 50);
            theme.assistant_msg_bg_color = Color::Rgb(35, 5, 52);
            theme.system_msg_bg_color = Color::Rgb(36, 5, 50);
            theme.tool_msg_bg_color = Color::Rgb(33, 7, 53);
            theme.status_bar_bg_color = Color::Rgb(28, 0, 45);
            theme.scrollbar_bg_color = Color::Rgb(38, 10, 57);
            theme.scrollbar_fg_color = Color::Rgb(139, 0, 255);
            theme.scrollbar_hover_color = Color::Rgb(0, 255, 255);
            theme.logo_primary_color = Color::Rgb(255, 0, 247);
            theme.logo_secondary_color = Color::Rgb(0, 255, 255);
            theme.animation_color = Color::Rgb(255, 0, 247);
            theme.processing_color = Color::Rgb(255, 255, 0);
            theme.highlight_color = Color::Rgb(255, 255, 0);
            theme.bubble_color = Color::Rgb(0, 255, 255);
            theme.token_low_color = Color::Rgb(0, 255, 179);
            theme.token_medium_color = Color::Rgb(255, 255, 0);
            theme.token_high_color = Color::Rgb(255, 0, 247);
            theme.token_critical_color = Color::Rgb(255, 0, 106);
        })
        .build()
}
