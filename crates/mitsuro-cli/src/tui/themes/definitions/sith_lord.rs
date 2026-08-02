use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Sith Lord theme
pub fn sith_lord() -> Theme {
    ThemeBuilder::new("sith-lord", "Sith Lord")
        .core_colors(CoreColors::new(
            Color::Rgb(30, 10, 12),
            Color::Rgb(139, 0, 0),
            Color::Rgb(220, 20, 60),
            Color::Rgb(255, 0, 0),
            Color::Rgb(192, 192, 192),
            Color::Rgb(255, 69, 0),
            Color::Rgb(105, 105, 105),
        ))
        .mode_colors(
            Color::Rgb(255, 69, 0),
            Color::Rgb(220, 20, 60),
            Color::Rgb(139, 0, 0),
            Color::Rgb(255, 0, 0),
            Color::Rgb(178, 34, 34),
        )
        .special_colors(
            Color::Rgb(255, 140, 0),
            Color::Rgb(255, 0, 0),
            Color::Rgb(20, 0, 0),
        )
        .ui_colors(
            Color::Rgb(255, 0, 0),
            Color::Rgb(80, 0, 0),
            Color::Rgb(192, 192, 192),
        )
        .message_colors(
            Color::Rgb(255, 69, 0),
            Color::Rgb(255, 0, 0),
            Color::Rgb(255, 140, 0),
            Color::Rgb(220, 20, 60),
        )
        .status_colors(Color::Rgb(220, 20, 60), Color::Rgb(255, 0, 0))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(20, 0, 0);
            theme.input_placeholder_color = Color::Rgb(105, 105, 105);
            theme.input_border_color = Color::Rgb(139, 0, 0);
            theme.user_msg_bg_color = Color::Rgb(25, 5, 5);
            theme.assistant_msg_bg_color = Color::Rgb(27, 5, 7);
            theme.system_msg_bg_color = Color::Rgb(28, 5, 5);
            theme.tool_msg_bg_color = Color::Rgb(25, 7, 8);
            theme.status_bar_bg_color = Color::Rgb(20, 0, 0);
            theme.scrollbar_bg_color = Color::Rgb(30, 10, 12);
            theme.scrollbar_fg_color = Color::Rgb(139, 0, 0);
            theme.scrollbar_hover_color = Color::Rgb(255, 0, 0);
            theme.logo_primary_color = Color::Rgb(220, 20, 60);
            theme.logo_secondary_color = Color::Rgb(255, 0, 0);
            theme.animation_color = Color::Rgb(220, 20, 60);
            theme.processing_color = Color::Rgb(255, 140, 0);
            theme.highlight_color = Color::Rgb(255, 140, 0);
            theme.bubble_color = Color::Rgb(255, 0, 0);
            theme.token_low_color = Color::Rgb(255, 69, 0);
            theme.token_medium_color = Color::Rgb(255, 140, 0);
            theme.token_high_color = Color::Rgb(220, 20, 60);
            theme.token_critical_color = Color::Rgb(255, 0, 0);
        })
        .build()
}
