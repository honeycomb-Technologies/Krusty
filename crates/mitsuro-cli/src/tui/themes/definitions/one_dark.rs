use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// One Dark theme
pub fn one_dark() -> Theme {
    ThemeBuilder::new("one-dark", "One Dark")
        .core_colors(CoreColors::new(
            Color::Rgb(40, 44, 52),
            Color::Rgb(61, 68, 81),
            Color::Rgb(97, 175, 239),
            Color::Rgb(198, 120, 221),
            Color::Rgb(171, 178, 191),
            Color::Rgb(152, 195, 121),
            Color::Rgb(92, 99, 112),
        ))
        .mode_colors(
            Color::Rgb(152, 195, 121),
            Color::Rgb(224, 108, 117),
            Color::Rgb(198, 120, 221),
            Color::Rgb(229, 192, 123),
            Color::Rgb(198, 120, 221),
        )
        .special_colors(
            Color::Rgb(229, 192, 123),
            Color::Rgb(224, 108, 117),
            Color::Rgb(33, 37, 43),
        )
        .ui_colors(
            Color::Rgb(198, 120, 221),
            Color::Rgb(60, 66, 80),
            Color::Rgb(171, 178, 191),
        )
        .message_colors(
            Color::Rgb(152, 195, 121),
            Color::Rgb(198, 120, 221),
            Color::Rgb(229, 192, 123),
            Color::Rgb(97, 175, 239),
        )
        .status_colors(Color::Rgb(97, 175, 239), Color::Rgb(198, 120, 221))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(49, 54, 63);
            theme.input_placeholder_color = Color::Rgb(92, 99, 112);
            theme.input_border_color = Color::Rgb(59, 66, 79);
            theme.user_msg_bg_color = Color::Rgb(49, 54, 63);
            theme.assistant_msg_bg_color = Color::Rgb(49, 54, 63);
            theme.system_msg_bg_color = Color::Rgb(49, 54, 63);
            theme.tool_msg_bg_color = Color::Rgb(49, 54, 63);
            theme.status_bar_bg_color = Color::Rgb(49, 54, 63);
            theme.scrollbar_bg_color = Color::Rgb(49, 54, 63);
            theme.scrollbar_fg_color = Color::Rgb(59, 66, 79);
            theme.scrollbar_hover_color = Color::Rgb(198, 120, 221);
            theme.logo_primary_color = Color::Rgb(97, 175, 239);
            theme.logo_secondary_color = Color::Rgb(198, 120, 221);
            theme.animation_color = Color::Rgb(97, 175, 239);
            theme.processing_color = Color::Rgb(229, 192, 123);
            theme.highlight_color = Color::Rgb(229, 192, 123);
            theme.bubble_color = Color::Rgb(97, 175, 239);
            theme.token_low_color = Color::Rgb(152, 195, 121);
            theme.token_medium_color = Color::Rgb(229, 192, 123);
            theme.token_high_color = Color::Rgb(224, 108, 117);
            theme.token_critical_color = Color::Rgb(224, 108, 117);
        })
        .build()
}
