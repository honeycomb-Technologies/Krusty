use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Zenburn theme
pub fn zenburn() -> Theme {
    ThemeBuilder::new("zenburn", "Zenburn")
        .core_colors(CoreColors::new(
            Color::Rgb(63, 63, 63),
            Color::Rgb(79, 79, 79),
            Color::Rgb(240, 223, 175),
            Color::Rgb(220, 163, 163),
            Color::Rgb(220, 220, 204),
            Color::Rgb(127, 159, 127),
            Color::Rgb(111, 111, 111),
        ))
        .mode_colors(
            Color::Rgb(127, 159, 127),
            Color::Rgb(220, 163, 163),
            Color::Rgb(183, 183, 163),
            Color::Rgb(240, 223, 175),
            Color::Rgb(220, 163, 163),
        )
        .special_colors(
            Color::Rgb(240, 223, 175),
            Color::Rgb(220, 163, 163),
            Color::Rgb(53, 53, 53),
        )
        .ui_colors(
            Color::Rgb(220, 163, 163),
            Color::Rgb(95, 95, 95),
            Color::Rgb(220, 220, 204),
        )
        .message_colors(
            Color::Rgb(127, 159, 127),
            Color::Rgb(220, 163, 163),
            Color::Rgb(240, 223, 175),
            Color::Rgb(240, 223, 175),
        )
        .status_colors(Color::Rgb(240, 223, 175), Color::Rgb(220, 163, 163))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(79, 79, 79);
            theme.input_placeholder_color = Color::Rgb(108, 108, 108);
            theme.input_border_color = Color::Rgb(95, 95, 95);
            theme.user_msg_bg_color = Color::Rgb(84, 84, 84);
            theme.assistant_msg_bg_color = Color::Rgb(86, 84, 86);
            theme.system_msg_bg_color = Color::Rgb(87, 84, 84);
            theme.tool_msg_bg_color = Color::Rgb(84, 86, 87);
            theme.status_bar_bg_color = Color::Rgb(79, 79, 79);
            theme.scrollbar_bg_color = Color::Rgb(89, 89, 91);
            theme.scrollbar_fg_color = Color::Rgb(95, 95, 95);
            theme.scrollbar_hover_color = Color::Rgb(220, 163, 163);
            theme.logo_primary_color = Color::Rgb(240, 223, 175);
            theme.logo_secondary_color = Color::Rgb(220, 163, 163);
            theme.animation_color = Color::Rgb(240, 223, 175);
            theme.processing_color = Color::Rgb(240, 223, 175);
            theme.highlight_color = Color::Rgb(240, 223, 175);
            theme.bubble_color = Color::Rgb(127, 159, 127);
            theme.token_low_color = Color::Rgb(127, 159, 127);
            theme.token_medium_color = Color::Rgb(240, 223, 175);
            theme.token_high_color = Color::Rgb(220, 163, 163);
            theme.token_critical_color = Color::Rgb(220, 163, 163);
        })
        .build()
}
