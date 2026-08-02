use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Retro Wave theme
pub fn retro_wave() -> Theme {
    ThemeBuilder::new("retro-wave", "Retro Wave")
        .core_colors(CoreColors::new(
            Color::Rgb(48, 10, 89),
            Color::Rgb(255, 102, 204),
            Color::Rgb(255, 204, 0),
            Color::Rgb(102, 255, 255),
            Color::Rgb(255, 255, 255),
            Color::Rgb(102, 255, 102),
            Color::Rgb(153, 102, 204),
        ))
        .mode_colors(
            Color::Rgb(102, 255, 102),
            Color::Rgb(255, 102, 204),
            Color::Rgb(153, 102, 255),
            Color::Rgb(255, 204, 0),
            Color::Rgb(102, 255, 255),
        )
        .special_colors(
            Color::Rgb(255, 204, 0),
            Color::Rgb(255, 51, 102),
            Color::Rgb(38, 0, 77),
        )
        .ui_colors(
            Color::Rgb(102, 255, 255),
            Color::Rgb(102, 51, 153),
            Color::Rgb(255, 255, 255),
        )
        .message_colors(
            Color::Rgb(102, 255, 102),
            Color::Rgb(102, 255, 255),
            Color::Rgb(255, 204, 0),
            Color::Rgb(255, 204, 0),
        )
        .status_colors(Color::Rgb(255, 204, 0), Color::Rgb(102, 255, 255))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(38, 0, 77);
            theme.input_placeholder_color = Color::Rgb(153, 102, 204);
            theme.input_border_color = Color::Rgb(255, 102, 204);
            theme.user_msg_bg_color = Color::Rgb(43, 5, 82);
            theme.assistant_msg_bg_color = Color::Rgb(45, 5, 84);
            theme.system_msg_bg_color = Color::Rgb(46, 5, 82);
            theme.tool_msg_bg_color = Color::Rgb(43, 7, 85);
            theme.status_bar_bg_color = Color::Rgb(38, 0, 77);
            theme.scrollbar_bg_color = Color::Rgb(48, 10, 89);
            theme.scrollbar_fg_color = Color::Rgb(255, 102, 204);
            theme.scrollbar_hover_color = Color::Rgb(102, 255, 255);
            theme.logo_primary_color = Color::Rgb(255, 204, 0);
            theme.logo_secondary_color = Color::Rgb(102, 255, 255);
            theme.animation_color = Color::Rgb(255, 204, 0);
            theme.processing_color = Color::Rgb(255, 204, 0);
            theme.highlight_color = Color::Rgb(255, 204, 0);
            theme.bubble_color = Color::Rgb(102, 255, 255);
            theme.token_low_color = Color::Rgb(102, 255, 102);
            theme.token_medium_color = Color::Rgb(255, 204, 0);
            theme.token_high_color = Color::Rgb(255, 102, 204);
            theme.token_critical_color = Color::Rgb(255, 51, 102);
        })
        .build()
}
