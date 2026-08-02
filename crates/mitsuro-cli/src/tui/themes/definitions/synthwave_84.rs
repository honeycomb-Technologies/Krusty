use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Synthwave '84 theme
pub fn synthwave_84() -> Theme {
    ThemeBuilder::new("synthwave-84", "Synthwave '84")
        .core_colors(CoreColors::new(
            Color::Rgb(52, 43, 69),
            Color::Rgb(73, 84, 149),
            Color::Rgb(54, 249, 246),
            Color::Rgb(176, 132, 235),
            Color::Rgb(255, 255, 255),
            Color::Rgb(114, 241, 184),
            Color::Rgb(132, 139, 189),
        ))
        .mode_colors(
            Color::Rgb(114, 241, 184),
            Color::Rgb(176, 132, 235),
            Color::Rgb(176, 132, 235),
            Color::Rgb(254, 222, 93),
            Color::Rgb(176, 132, 235),
        )
        .special_colors(
            Color::Rgb(254, 222, 93),
            Color::Rgb(254, 68, 80),
            Color::Rgb(42, 33, 57),
        )
        .ui_colors(
            Color::Rgb(176, 132, 235),
            Color::Rgb(58, 49, 73),
            Color::Rgb(255, 255, 255),
        )
        .message_colors(
            Color::Rgb(114, 241, 184),
            Color::Rgb(176, 132, 235),
            Color::Rgb(254, 222, 93),
            Color::Rgb(54, 249, 246),
        )
        .status_colors(Color::Rgb(54, 249, 246), Color::Rgb(176, 132, 235))
        .extended_colors(|theme| {
            theme.input_bg_color = Color::Rgb(42, 33, 57);
            theme.input_placeholder_color = Color::Rgb(132, 139, 189);
            theme.input_border_color = Color::Rgb(73, 84, 149);
            theme.user_msg_bg_color = Color::Rgb(47, 38, 62);
            theme.assistant_msg_bg_color = Color::Rgb(49, 38, 64);
            theme.system_msg_bg_color = Color::Rgb(50, 38, 62);
            theme.tool_msg_bg_color = Color::Rgb(47, 40, 65);
            theme.status_bar_bg_color = Color::Rgb(42, 33, 57);
            theme.scrollbar_bg_color = Color::Rgb(52, 43, 69);
            theme.scrollbar_fg_color = Color::Rgb(73, 84, 149);
            theme.scrollbar_hover_color = Color::Rgb(176, 132, 235);
            theme.logo_primary_color = Color::Rgb(54, 249, 246);
            theme.logo_secondary_color = Color::Rgb(176, 132, 235);
            theme.animation_color = Color::Rgb(54, 249, 246);
            theme.processing_color = Color::Rgb(254, 222, 93);
            theme.highlight_color = Color::Rgb(254, 222, 93);
            theme.bubble_color = Color::Rgb(54, 249, 246);
            theme.token_low_color = Color::Rgb(114, 241, 184);
            theme.token_medium_color = Color::Rgb(254, 222, 93);
            theme.token_high_color = Color::Rgb(176, 132, 235);
            theme.token_critical_color = Color::Rgb(254, 68, 80);
        })
        .build()
}
