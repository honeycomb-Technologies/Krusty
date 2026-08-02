use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Nord theme
pub fn nord() -> Theme {
    ThemeBuilder::new("nord", "Nord")
        .core_colors(CoreColors::new(
            Color::Rgb(46, 52, 64),
            Color::Rgb(59, 66, 82),
            Color::Rgb(136, 192, 208),
            Color::Rgb(129, 161, 193),
            Color::Rgb(216, 222, 233),
            Color::Rgb(163, 190, 140),
            Color::Rgb(76, 86, 106),
        ))
        .mode_colors(
            Color::Rgb(163, 190, 140),
            Color::Rgb(180, 142, 173),
            Color::Rgb(129, 161, 193),
            Color::Rgb(208, 135, 112),
            Color::Rgb(129, 161, 193),
        )
        .special_colors(
            Color::Rgb(235, 203, 139),
            Color::Rgb(191, 97, 106),
            Color::Rgb(51, 57, 69),
        )
        .ui_colors(
            Color::Rgb(136, 192, 208),
            Color::Rgb(67, 76, 94),
            Color::Rgb(216, 222, 233),
        )
        .message_colors(
            Color::Rgb(163, 190, 140),
            Color::Rgb(136, 192, 208),
            Color::Rgb(235, 203, 139),
            Color::Rgb(180, 142, 173),
        )
        .status_colors(Color::Rgb(143, 188, 187), Color::Rgb(136, 192, 208))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(53, 59, 72);
            theme.input_placeholder_color = Color::Rgb(76, 86, 106);
            theme.input_border_color = Color::Rgb(59, 66, 82);
            theme.user_msg_bg_color = Color::Rgb(54, 60, 72);
            theme.assistant_msg_bg_color = Color::Rgb(56, 60, 74);
            theme.system_msg_bg_color = Color::Rgb(58, 60, 72);
            theme.tool_msg_bg_color = Color::Rgb(54, 62, 74);
            theme.status_bar_bg_color = Color::Rgb(51, 57, 69);
            theme.scrollbar_bg_color = Color::Rgb(56, 62, 76);
            theme.scrollbar_fg_color = Color::Rgb(59, 66, 82);
            theme.scrollbar_hover_color = Color::Rgb(136, 192, 208);
            theme.logo_primary_color = Color::Rgb(136, 192, 208);
            theme.logo_secondary_color = Color::Rgb(129, 161, 193);
            theme.animation_color = Color::Rgb(136, 192, 208);
            theme.processing_color = Color::Rgb(235, 203, 139);
            theme.highlight_color = Color::Rgb(208, 135, 112);
            theme.bubble_color = Color::Rgb(136, 192, 208);
            theme.token_low_color = Color::Rgb(163, 190, 140);
            theme.token_medium_color = Color::Rgb(235, 203, 139);
            theme.token_high_color = Color::Rgb(180, 142, 173);
            theme.token_critical_color = Color::Rgb(191, 97, 106);
        })
        .build()
}
