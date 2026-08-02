use crate::tui::themes::{
    base::{CoreColors, ThemeBuilder},
    Theme,
};
use ratatui::style::Color;

/// Dracula theme - popular dark theme with vibrant colors
pub fn dracula() -> Theme {
    ThemeBuilder::new("dracula", "Dracula")
        .core_colors(CoreColors::new(
            Color::Rgb(40, 42, 54),    // bg
            Color::Rgb(68, 71, 90),    // border
            Color::Rgb(139, 233, 253), // title (cyan)
            Color::Rgb(189, 147, 249), // accent (purple)
            Color::Rgb(248, 248, 242), // text
            Color::Rgb(80, 250, 123),  // success (green)
            Color::Rgb(98, 114, 164),  // dim
        ))
        .mode_colors(
            Color::Rgb(80, 250, 123),  // view (green)
            Color::Rgb(255, 121, 198), // chat (pink)
            Color::Rgb(189, 147, 249), // plan (purple)
            Color::Rgb(255, 184, 108), // bash (orange)
            Color::Rgb(189, 147, 249), // leader (purple)
        )
        .special_colors(
            Color::Rgb(255, 184, 108), // warning (orange)
            Color::Rgb(255, 85, 85),   // error (red)
            Color::Rgb(44, 46, 60),    // code_bg
        )
        .ui_colors(
            Color::Rgb(189, 147, 249), // cursor (purple)
            Color::Rgb(60, 60, 80),    // selection_bg
            Color::Rgb(248, 248, 242), // selection_fg
        )
        .message_colors(
            Color::Rgb(80, 250, 123),  // user (green)
            Color::Rgb(189, 147, 249), // assistant (purple)
            Color::Rgb(255, 184, 108), // system (orange)
            Color::Rgb(255, 121, 198), // tool (pink)
        )
        .status_colors(
            Color::Rgb(139, 233, 253), // info (cyan)
            Color::Rgb(189, 147, 249), // progress (purple)
        )
        .extended_colors(|theme| {
            // Dracula specific extended colors
            theme.input_bg_color = Color::Rgb(45, 47, 60);
            theme.input_placeholder_color = Color::Rgb(98, 114, 164);
            theme.input_border_color = Color::Rgb(68, 71, 90);

            theme.user_msg_bg_color = Color::Rgb(48, 50, 64);
            theme.assistant_msg_bg_color = Color::Rgb(50, 48, 64);
            theme.system_msg_bg_color = Color::Rgb(52, 48, 62);
            theme.tool_msg_bg_color = Color::Rgb(48, 52, 62);

            theme.status_bar_bg_color = Color::Rgb(44, 46, 60);
            theme.scrollbar_bg_color = Color::Rgb(50, 52, 66);
            theme.scrollbar_fg_color = Color::Rgb(68, 71, 90);
            theme.scrollbar_hover_color = Color::Rgb(189, 147, 249);

            theme.logo_primary_color = Color::Rgb(139, 233, 253);
            theme.logo_secondary_color = Color::Rgb(189, 147, 249);

            theme.animation_color = Color::Rgb(139, 233, 253);
            theme.processing_color = Color::Rgb(255, 184, 108);
            theme.highlight_color = Color::Rgb(255, 121, 198);
            theme.bubble_color = Color::Rgb(189, 147, 249);

            theme.token_low_color = Color::Rgb(80, 250, 123);
            theme.token_medium_color = Color::Rgb(255, 184, 108);
            theme.token_high_color = Color::Rgb(255, 121, 198);
            theme.token_critical_color = Color::Rgb(255, 85, 85);
        })
        .build()
}
