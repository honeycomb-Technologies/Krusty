use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Palenight theme
pub fn palenight() -> Theme {
    ThemeBuilder::new("palenight", "Palenight")
        .core_colors(CoreColors::new(
            Color::Rgb(41, 45, 62),
            Color::Rgb(68, 71, 90),
            Color::Rgb(130, 170, 255),
            Color::Rgb(199, 146, 234),
            Color::Rgb(166, 172, 205),
            Color::Rgb(195, 232, 141),
            Color::Rgb(103, 110, 149),
        ))
        .mode_colors(
            Color::Rgb(195, 232, 141),
            Color::Rgb(199, 146, 234),
            Color::Rgb(199, 146, 234),
            Color::Rgb(255, 203, 107),
            Color::Rgb(199, 146, 234),
        )
        .special_colors(
            Color::Rgb(255, 203, 107),
            Color::Rgb(240, 113, 120),
            Color::Rgb(50, 54, 74),
        )
        .ui_colors(
            Color::Rgb(199, 146, 234),
            Color::Rgb(60, 64, 84),
            Color::Rgb(166, 172, 205),
        )
        .message_colors(
            Color::Rgb(195, 232, 141),
            Color::Rgb(199, 146, 234),
            Color::Rgb(255, 203, 107),
            Color::Rgb(130, 170, 255),
        )
        .status_colors(Color::Rgb(130, 170, 255), Color::Rgb(199, 146, 234))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(50, 54, 74);
            theme.input_placeholder_color = Color::Rgb(103, 110, 149);
            theme.input_border_color = Color::Rgb(50, 54, 74);
            theme.user_msg_bg_color = Color::Rgb(55, 59, 79);
            theme.assistant_msg_bg_color = Color::Rgb(57, 59, 81);
            theme.system_msg_bg_color = Color::Rgb(58, 59, 79);
            theme.tool_msg_bg_color = Color::Rgb(55, 61, 82);
            theme.status_bar_bg_color = Color::Rgb(50, 54, 74);
            theme.scrollbar_bg_color = Color::Rgb(60, 64, 86);
            theme.scrollbar_fg_color = Color::Rgb(50, 54, 74);
            theme.scrollbar_hover_color = Color::Rgb(137, 221, 255);
            theme.logo_primary_color = Color::Rgb(130, 170, 255);
            theme.logo_secondary_color = Color::Rgb(137, 221, 255);
            theme.animation_color = Color::Rgb(130, 170, 255);
            theme.processing_color = Color::Rgb(255, 203, 107);
            theme.highlight_color = Color::Rgb(255, 203, 107);
            theme.bubble_color = Color::Rgb(130, 170, 255);
            theme.token_low_color = Color::Rgb(195, 232, 141);
            theme.token_medium_color = Color::Rgb(255, 203, 107);
            theme.token_high_color = Color::Rgb(137, 221, 255);
            theme.token_critical_color = Color::Rgb(240, 113, 120);
        })
        .build()
}
