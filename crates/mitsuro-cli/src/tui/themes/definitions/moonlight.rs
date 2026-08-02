use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Moonlight theme
pub fn moonlight() -> Theme {
    ThemeBuilder::new("moonlight", "Moonlight")
        .core_colors(CoreColors::new(
            Color::Rgb(33, 35, 55),
            Color::Rgb(64, 60, 100),
            Color::Rgb(130, 170, 255),
            Color::Rgb(252, 167, 234),
            Color::Rgb(163, 172, 225),
            Color::Rgb(195, 232, 141),
            Color::Rgb(122, 136, 207),
        ))
        .mode_colors(
            Color::Rgb(195, 232, 141),
            Color::Rgb(252, 167, 234),
            Color::Rgb(199, 146, 234),
            Color::Rgb(255, 203, 107),
            Color::Rgb(130, 170, 255),
        )
        .special_colors(
            Color::Rgb(255, 203, 107),
            Color::Rgb(255, 119, 119),
            Color::Rgb(40, 44, 60),
        )
        .ui_colors(
            Color::Rgb(255, 146, 164),
            Color::Rgb(50, 54, 70),
            Color::Rgb(195, 199, 221),
        )
        .message_colors(
            Color::Rgb(195, 232, 141),
            Color::Rgb(255, 146, 164),
            Color::Rgb(255, 203, 107),
            Color::Rgb(130, 220, 235),
        )
        .status_colors(Color::Rgb(130, 220, 235), Color::Rgb(255, 146, 164))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(40, 44, 60);
            theme.input_placeholder_color = Color::Rgb(100, 106, 134);
            theme.input_border_color = Color::Rgb(68, 74, 102);
            theme.user_msg_bg_color = Color::Rgb(45, 49, 65);
            theme.assistant_msg_bg_color = Color::Rgb(47, 49, 67);
            theme.system_msg_bg_color = Color::Rgb(48, 49, 65);
            theme.tool_msg_bg_color = Color::Rgb(45, 51, 68);
            theme.status_bar_bg_color = Color::Rgb(40, 44, 60);
            theme.scrollbar_bg_color = Color::Rgb(50, 54, 72);
            theme.scrollbar_fg_color = Color::Rgb(68, 74, 102);
            theme.scrollbar_hover_color = Color::Rgb(255, 146, 164);
            theme.logo_primary_color = Color::Rgb(130, 220, 235);
            theme.logo_secondary_color = Color::Rgb(255, 146, 164);
            theme.animation_color = Color::Rgb(130, 220, 235);
            theme.processing_color = Color::Rgb(255, 203, 107);
            theme.highlight_color = Color::Rgb(255, 203, 107);
            theme.bubble_color = Color::Rgb(130, 220, 235);
            theme.token_low_color = Color::Rgb(195, 232, 141);
            theme.token_medium_color = Color::Rgb(255, 203, 107);
            theme.token_high_color = Color::Rgb(255, 146, 164);
            theme.token_critical_color = Color::Rgb(255, 119, 119);
        })
        .build()
}
