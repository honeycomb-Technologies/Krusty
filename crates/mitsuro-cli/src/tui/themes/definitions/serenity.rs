use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Serenity theme
pub fn serenity() -> Theme {
    ThemeBuilder::new("serenity", "Serenity")
        .core_colors(CoreColors::new(
            Color::Rgb(52, 56, 74),
            Color::Rgb(73, 77, 100),
            Color::Rgb(146, 195, 224),
            Color::Rgb(176, 146, 224),
            Color::Rgb(212, 213, 219),
            Color::Rgb(146, 224, 176),
            Color::Rgb(108, 113, 134),
        ))
        .mode_colors(
            Color::Rgb(146, 224, 176),
            Color::Rgb(224, 146, 195),
            Color::Rgb(176, 146, 224),
            Color::Rgb(224, 195, 146),
            Color::Rgb(146, 195, 224),
        )
        .special_colors(
            Color::Rgb(224, 195, 146),
            Color::Rgb(224, 146, 146),
            Color::Rgb(42, 46, 62),
        )
        .ui_colors(
            Color::Rgb(176, 146, 224),
            Color::Rgb(53, 57, 73),
            Color::Rgb(212, 213, 219),
        )
        .message_colors(
            Color::Rgb(146, 224, 176),
            Color::Rgb(176, 146, 224),
            Color::Rgb(224, 195, 146),
            Color::Rgb(146, 195, 224),
        )
        .status_colors(Color::Rgb(146, 195, 224), Color::Rgb(176, 146, 224))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(42, 46, 62);
            theme.input_placeholder_color = Color::Rgb(108, 113, 134);
            theme.input_border_color = Color::Rgb(73, 77, 100);
            theme.user_msg_bg_color = Color::Rgb(47, 51, 67);
            theme.assistant_msg_bg_color = Color::Rgb(49, 51, 69);
            theme.system_msg_bg_color = Color::Rgb(50, 51, 67);
            theme.tool_msg_bg_color = Color::Rgb(47, 53, 70);
            theme.status_bar_bg_color = Color::Rgb(42, 46, 62);
            theme.scrollbar_bg_color = Color::Rgb(52, 56, 74);
            theme.scrollbar_fg_color = Color::Rgb(73, 77, 100);
            theme.scrollbar_hover_color = Color::Rgb(176, 146, 224);
            theme.logo_primary_color = Color::Rgb(146, 195, 224);
            theme.logo_secondary_color = Color::Rgb(176, 146, 224);
            theme.animation_color = Color::Rgb(146, 195, 224);
            theme.processing_color = Color::Rgb(224, 195, 146);
            theme.highlight_color = Color::Rgb(224, 195, 146);
            theme.bubble_color = Color::Rgb(146, 195, 224);
            theme.token_low_color = Color::Rgb(146, 224, 176);
            theme.token_medium_color = Color::Rgb(224, 195, 146);
            theme.token_high_color = Color::Rgb(224, 146, 195);
            theme.token_critical_color = Color::Rgb(224, 146, 146);
        })
        .build()
}
