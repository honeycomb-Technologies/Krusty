use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Material Ocean theme
pub fn material_ocean() -> Theme {
    ThemeBuilder::new("material-ocean", "Material Ocean")
        .core_colors(CoreColors::new(
            Color::Rgb(38, 50, 56),
            Color::Rgb(84, 110, 122),
            Color::Rgb(130, 170, 255),
            Color::Rgb(199, 146, 234),
            Color::Rgb(176, 190, 197),
            Color::Rgb(195, 232, 141),
            Color::Rgb(84, 110, 122),
        ))
        .mode_colors(
            Color::Rgb(195, 232, 141),
            Color::Rgb(247, 140, 108),
            Color::Rgb(199, 146, 234),
            Color::Rgb(255, 203, 107),
            Color::Rgb(130, 170, 255),
        )
        .special_colors(
            Color::Rgb(255, 203, 107),
            Color::Rgb(240, 113, 120),
            Color::Rgb(32, 43, 48),
        )
        .ui_colors(
            Color::Rgb(199, 146, 234),
            Color::Rgb(35, 39, 54),
            Color::Rgb(169, 177, 214),
        )
        .message_colors(
            Color::Rgb(195, 232, 141),
            Color::Rgb(199, 146, 234),
            Color::Rgb(255, 203, 107),
            Color::Rgb(130, 170, 255),
        )
        .status_colors(Color::Rgb(130, 170, 255), Color::Rgb(199, 146, 234))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(20, 24, 35);
            theme.input_placeholder_color = Color::Rgb(103, 110, 149);
            theme.input_border_color = Color::Rgb(65, 72, 104);
            theme.user_msg_bg_color = Color::Rgb(25, 29, 40);
            theme.assistant_msg_bg_color = Color::Rgb(27, 29, 42);
            theme.system_msg_bg_color = Color::Rgb(28, 29, 40);
            theme.tool_msg_bg_color = Color::Rgb(25, 31, 43);
            theme.status_bar_bg_color = Color::Rgb(20, 24, 35);
            theme.scrollbar_bg_color = Color::Rgb(30, 34, 47);
            theme.scrollbar_fg_color = Color::Rgb(65, 72, 104);
            theme.scrollbar_hover_color = Color::Rgb(199, 146, 234);
            theme.logo_primary_color = Color::Rgb(130, 170, 255);
            theme.logo_secondary_color = Color::Rgb(199, 146, 234);
            theme.animation_color = Color::Rgb(130, 170, 255);
            theme.processing_color = Color::Rgb(255, 203, 107);
            theme.highlight_color = Color::Rgb(255, 203, 107);
            theme.bubble_color = Color::Rgb(130, 170, 255);
            theme.token_low_color = Color::Rgb(195, 232, 141);
            theme.token_medium_color = Color::Rgb(255, 203, 107);
            theme.token_high_color = Color::Rgb(247, 140, 108);
            theme.token_critical_color = Color::Rgb(240, 113, 120);
        })
        .build()
}
