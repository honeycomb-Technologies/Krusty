use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Forest Night theme
pub fn forest_night() -> Theme {
    ThemeBuilder::new("forest-night", "Forest Night")
        .core_colors(CoreColors::new(
            Color::Rgb(45, 60, 60),
            Color::Rgb(59, 85, 80),
            Color::Rgb(163, 230, 163),
            Color::Rgb(107, 203, 119),
            Color::Rgb(221, 237, 221),
            Color::Rgb(144, 238, 144),
            Color::Rgb(100, 130, 125),
        ))
        .mode_colors(
            Color::Rgb(144, 238, 144),
            Color::Rgb(255, 182, 193),
            Color::Rgb(221, 160, 221),
            Color::Rgb(240, 230, 140),
            Color::Rgb(107, 203, 119),
        )
        .special_colors(
            Color::Rgb(240, 230, 140),
            Color::Rgb(205, 92, 92),
            Color::Rgb(35, 50, 48),
        )
        .ui_colors(
            Color::Rgb(107, 203, 119),
            Color::Rgb(45, 60, 58),
            Color::Rgb(221, 237, 221),
        )
        .message_colors(
            Color::Rgb(144, 238, 144),
            Color::Rgb(107, 203, 119),
            Color::Rgb(240, 230, 140),
            Color::Rgb(163, 230, 163),
        )
        .status_colors(Color::Rgb(163, 230, 163), Color::Rgb(107, 203, 119))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(35, 50, 48);
            theme.input_placeholder_color = Color::Rgb(100, 130, 125);
            theme.input_border_color = Color::Rgb(59, 85, 80);
            theme.user_msg_bg_color = Color::Rgb(40, 55, 53);
            theme.assistant_msg_bg_color = Color::Rgb(42, 55, 55);
            theme.system_msg_bg_color = Color::Rgb(43, 55, 53);
            theme.tool_msg_bg_color = Color::Rgb(40, 57, 56);
            theme.status_bar_bg_color = Color::Rgb(35, 50, 48);
            theme.scrollbar_bg_color = Color::Rgb(45, 60, 60);
            theme.scrollbar_fg_color = Color::Rgb(59, 85, 80);
            theme.scrollbar_hover_color = Color::Rgb(107, 203, 119);
            theme.logo_primary_color = Color::Rgb(163, 230, 163);
            theme.logo_secondary_color = Color::Rgb(107, 203, 119);
            theme.animation_color = Color::Rgb(163, 230, 163);
            theme.processing_color = Color::Rgb(240, 230, 140);
            theme.highlight_color = Color::Rgb(240, 230, 140);
            theme.bubble_color = Color::Rgb(144, 238, 144);
            theme.token_low_color = Color::Rgb(144, 238, 144);
            theme.token_medium_color = Color::Rgb(240, 230, 140);
            theme.token_high_color = Color::Rgb(255, 182, 193);
            theme.token_critical_color = Color::Rgb(205, 92, 92);
        })
        .build()
}
