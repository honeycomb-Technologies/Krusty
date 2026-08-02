use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Aura theme
pub fn aura() -> Theme {
    ThemeBuilder::new("aura", "Aura")
        .core_colors(CoreColors::new(
            Color::Rgb(31, 30, 39),
            Color::Rgb(45, 45, 45),
            Color::Rgb(162, 119, 255),
            Color::Rgb(162, 119, 255),
            Color::Rgb(237, 236, 238),
            Color::Rgb(97, 255, 202),
            Color::Rgb(109, 109, 109),
        ))
        .mode_colors(
            Color::Rgb(97, 255, 202),
            Color::Rgb(162, 119, 255),
            Color::Rgb(162, 119, 255),
            Color::Rgb(255, 202, 133),
            Color::Rgb(162, 119, 255),
        )
        .special_colors(
            Color::Rgb(255, 202, 133),
            Color::Rgb(255, 103, 103),
            Color::Rgb(21, 20, 27),
        )
        .ui_colors(
            Color::Rgb(162, 119, 255),
            Color::Rgb(35, 35, 45),
            Color::Rgb(237, 236, 238),
        )
        .message_colors(
            Color::Rgb(97, 255, 202),
            Color::Rgb(162, 119, 255),
            Color::Rgb(255, 202, 133),
            Color::Rgb(162, 119, 255),
        )
        .status_colors(Color::Rgb(162, 119, 255), Color::Rgb(162, 119, 255))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(21, 20, 27);
            theme.input_placeholder_color = Color::Rgb(109, 109, 109);
            theme.input_border_color = Color::Rgb(45, 45, 45);
            theme.user_msg_bg_color = Color::Rgb(26, 25, 32);
            theme.assistant_msg_bg_color = Color::Rgb(28, 25, 34);
            theme.system_msg_bg_color = Color::Rgb(29, 25, 32);
            theme.tool_msg_bg_color = Color::Rgb(26, 27, 35);
            theme.status_bar_bg_color = Color::Rgb(21, 20, 27);
            theme.scrollbar_bg_color = Color::Rgb(31, 30, 39);
            theme.scrollbar_fg_color = Color::Rgb(45, 45, 45);
            theme.scrollbar_hover_color = Color::Rgb(162, 119, 255);
            theme.logo_primary_color = Color::Rgb(162, 119, 255);
            theme.logo_secondary_color = Color::Rgb(162, 119, 255);
            theme.animation_color = Color::Rgb(162, 119, 255);
            theme.processing_color = Color::Rgb(255, 202, 133);
            theme.highlight_color = Color::Rgb(255, 202, 133);
            theme.bubble_color = Color::Rgb(162, 119, 255);
            theme.token_low_color = Color::Rgb(97, 255, 202);
            theme.token_medium_color = Color::Rgb(255, 202, 133);
            theme.token_high_color = Color::Rgb(162, 119, 255);
            theme.token_critical_color = Color::Rgb(255, 103, 103);
        })
        .build()
}
