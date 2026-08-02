use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Rosé Pine theme
pub fn rosepine() -> Theme {
    ThemeBuilder::new("rosepine", "Rosé Pine")
        .core_colors(CoreColors::new(
            Color::Rgb(25, 23, 36),
            Color::Rgb(38, 35, 58),
            Color::Rgb(156, 207, 216),
            Color::Rgb(196, 167, 231),
            Color::Rgb(224, 222, 244),
            Color::Rgb(49, 116, 143),
            Color::Rgb(144, 140, 170),
        ))
        .mode_colors(
            Color::Rgb(49, 116, 143),
            Color::Rgb(235, 188, 186),
            Color::Rgb(196, 167, 231),
            Color::Rgb(246, 193, 119),
            Color::Rgb(196, 167, 231),
        )
        .special_colors(
            Color::Rgb(246, 193, 119),
            Color::Rgb(235, 111, 146),
            Color::Rgb(38, 35, 58),
        )
        .ui_colors(
            Color::Rgb(196, 167, 231),
            Color::Rgb(44, 41, 64),
            Color::Rgb(224, 222, 244),
        )
        .message_colors(
            Color::Rgb(49, 116, 143),
            Color::Rgb(235, 188, 186),
            Color::Rgb(246, 193, 119),
            Color::Rgb(156, 207, 216),
        )
        .status_colors(Color::Rgb(156, 207, 216), Color::Rgb(235, 188, 186))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(38, 35, 58);
            theme.input_placeholder_color = Color::Rgb(110, 106, 134);
            theme.input_border_color = Color::Rgb(64, 61, 82);
            theme.user_msg_bg_color = Color::Rgb(43, 40, 63);
            theme.assistant_msg_bg_color = Color::Rgb(45, 40, 65);
            theme.system_msg_bg_color = Color::Rgb(46, 40, 63);
            theme.tool_msg_bg_color = Color::Rgb(43, 42, 66);
            theme.status_bar_bg_color = Color::Rgb(38, 35, 58);
            theme.scrollbar_bg_color = Color::Rgb(48, 45, 70);
            theme.scrollbar_fg_color = Color::Rgb(64, 61, 82);
            theme.scrollbar_hover_color = Color::Rgb(235, 188, 186);
            theme.logo_primary_color = Color::Rgb(156, 207, 216);
            theme.logo_secondary_color = Color::Rgb(235, 188, 186);
            theme.animation_color = Color::Rgb(156, 207, 216);
            theme.processing_color = Color::Rgb(246, 193, 119);
            theme.highlight_color = Color::Rgb(246, 193, 119);
            theme.bubble_color = Color::Rgb(156, 207, 216);
            theme.token_low_color = Color::Rgb(49, 116, 143);
            theme.token_medium_color = Color::Rgb(246, 193, 119);
            theme.token_high_color = Color::Rgb(235, 188, 186);
            theme.token_critical_color = Color::Rgb(235, 111, 146);
        })
        .build()
}
