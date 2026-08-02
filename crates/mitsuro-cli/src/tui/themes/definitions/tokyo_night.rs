use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Tokyo Night theme - popular dark theme with blue/purple tones
pub fn tokyo_night() -> Theme {
    ThemeBuilder::new("tokyo-night", "Tokyo Night")
        .core_colors(CoreColors::new(
            Color::Rgb(26, 27, 38),    // bg
            Color::Rgb(65, 72, 104),   // border
            Color::Rgb(122, 162, 247), // title
            Color::Rgb(187, 154, 247), // accent
            Color::Rgb(192, 202, 245), // text
            Color::Rgb(158, 206, 106), // success
            Color::Rgb(86, 95, 137),   // dim
        ))
        .mode_colors(
            Color::Rgb(158, 206, 106), // view (green)
            Color::Rgb(247, 118, 142), // chat (pink)
            Color::Rgb(187, 154, 247), // plan (purple)
            Color::Rgb(224, 175, 104), // bash (orange)
            Color::Rgb(187, 154, 247), // leader (purple)
        )
        .special_colors(
            Color::Rgb(224, 175, 104), // warning (orange)
            Color::Rgb(247, 118, 142), // error (pink)
            Color::Rgb(31, 32, 45),    // code_bg
        )
        .ui_colors(
            Color::Rgb(125, 207, 255), // cursor
            Color::Rgb(50, 52, 75),    // selection_bg
            Color::Rgb(192, 202, 245), // selection_fg
        )
        .message_colors(
            Color::Rgb(158, 206, 106), // user (green)
            Color::Rgb(125, 207, 255), // assistant (blue)
            Color::Rgb(224, 175, 104), // system (orange)
            Color::Rgb(187, 154, 247), // tool (purple)
        )
        .status_colors(
            Color::Rgb(125, 207, 255), // info
            Color::Rgb(255, 158, 100), // progress
        )
        .extended_colors(|theme| {
            // Tokyo Night specific extended colors
            theme.input_bg_color = Color::Rgb(36, 38, 52);
            theme.input_placeholder_color = Color::Rgb(86, 95, 137);
            theme.input_border_color = Color::Rgb(86, 95, 137);

            theme.user_msg_bg_color = Color::Rgb(34, 37, 49);
            theme.assistant_msg_bg_color = Color::Rgb(35, 38, 52);
            theme.system_msg_bg_color = Color::Rgb(38, 35, 49);
            theme.tool_msg_bg_color = Color::Rgb(34, 35, 50);

            theme.status_bar_bg_color = Color::Rgb(31, 32, 45);
            theme.scrollbar_bg_color = Color::Rgb(36, 38, 52);
            theme.scrollbar_fg_color = Color::Rgb(86, 95, 137);
            theme.scrollbar_hover_color = Color::Rgb(125, 207, 255);

            theme.logo_primary_color = Color::Rgb(125, 207, 255);
            theme.logo_secondary_color = Color::Rgb(187, 154, 247);

            theme.animation_color = Color::Rgb(125, 207, 255);
            theme.processing_color = Color::Rgb(224, 175, 104);
            theme.highlight_color = Color::Rgb(255, 158, 100);
            theme.bubble_color = Color::Rgb(125, 207, 255);

            theme.token_low_color = Color::Rgb(158, 206, 106);
            theme.token_medium_color = Color::Rgb(224, 175, 104);
            theme.token_high_color = Color::Rgb(247, 118, 142);
            theme.token_critical_color = Color::Rgb(247, 118, 142);
        })
        .build()
}
