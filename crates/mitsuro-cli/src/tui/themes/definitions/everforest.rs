use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Everforest theme
pub fn everforest() -> Theme {
    ThemeBuilder::new("everforest", "Everforest")
        .core_colors(CoreColors::new(
            Color::Rgb(45, 53, 59),
            Color::Rgb(53, 64, 70),
            Color::Rgb(127, 187, 179),
            Color::Rgb(214, 153, 182),
            Color::Rgb(211, 198, 170),
            Color::Rgb(167, 192, 128),
            Color::Rgb(133, 146, 137),
        ))
        .mode_colors(
            Color::Rgb(167, 192, 128),
            Color::Rgb(214, 153, 182),
            Color::Rgb(214, 153, 182),
            Color::Rgb(230, 152, 117),
            Color::Rgb(214, 153, 182),
        )
        .special_colors(
            Color::Rgb(230, 152, 117),
            Color::Rgb(230, 126, 128),
            Color::Rgb(35, 42, 46),
        )
        .ui_colors(
            Color::Rgb(214, 153, 182),
            Color::Rgb(62, 73, 78),
            Color::Rgb(211, 198, 170),
        )
        .message_colors(
            Color::Rgb(167, 192, 128),
            Color::Rgb(214, 153, 182),
            Color::Rgb(230, 152, 117),
            Color::Rgb(167, 192, 128),
        )
        .status_colors(Color::Rgb(167, 192, 128), Color::Rgb(214, 153, 182))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(52, 63, 68);
            theme.input_placeholder_color = Color::Rgb(122, 132, 120);
            theme.input_border_color = Color::Rgb(133, 146, 137);
            theme.user_msg_bg_color = Color::Rgb(57, 68, 73);
            theme.assistant_msg_bg_color = Color::Rgb(59, 68, 75);
            theme.system_msg_bg_color = Color::Rgb(60, 68, 73);
            theme.tool_msg_bg_color = Color::Rgb(57, 70, 76);
            theme.status_bar_bg_color = Color::Rgb(52, 63, 68);
            theme.scrollbar_bg_color = Color::Rgb(62, 73, 80);
            theme.scrollbar_fg_color = Color::Rgb(133, 146, 137);
            theme.scrollbar_hover_color = Color::Rgb(214, 153, 182);
            theme.logo_primary_color = Color::Rgb(167, 192, 128);
            theme.logo_secondary_color = Color::Rgb(214, 153, 182);
            theme.animation_color = Color::Rgb(167, 192, 128);
            theme.processing_color = Color::Rgb(230, 152, 117);
            theme.highlight_color = Color::Rgb(230, 152, 117);
            theme.bubble_color = Color::Rgb(167, 192, 128);
            theme.token_low_color = Color::Rgb(167, 192, 128);
            theme.token_medium_color = Color::Rgb(230, 152, 117);
            theme.token_high_color = Color::Rgb(214, 153, 182);
            theme.token_critical_color = Color::Rgb(230, 126, 128);
        })
        .build()
}
