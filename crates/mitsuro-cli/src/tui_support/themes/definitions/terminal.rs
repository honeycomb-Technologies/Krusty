use super::super::Theme;
use ratatui::style::Color;

/// Terminal theme - uses native terminal colors (ANSI 0-15)
/// This theme inherits your terminal's colorscheme, so it will
/// live-update when you change your system/terminal theme (e.g., via Aether)
///
/// ANSI color mapping:
/// 0: Black, 1: Red, 2: Green, 3: Yellow, 4: Blue, 5: Magenta, 6: Cyan, 7: White
/// 8-15: Bright variants of the above
pub fn terminal() -> Theme {
    // Standard ANSI colors - these follow your terminal's colorscheme
    let red = Color::Indexed(1);
    let green = Color::Indexed(2);
    let yellow = Color::Indexed(3);
    let blue = Color::Indexed(4);
    let magenta = Color::Indexed(5);
    let cyan = Color::Indexed(6);
    let white = Color::Indexed(7);
    let bright_black = Color::Indexed(8); // Gray
    let bright_red = Color::Indexed(9);
    let bright_green = Color::Indexed(10);
    let bright_yellow = Color::Indexed(11);
    let bright_blue = Color::Indexed(12);
    let bright_magenta = Color::Indexed(13);
    let bright_cyan = Color::Indexed(14);
    let bright_white = Color::Indexed(15);

    // Reset = terminal's default background (transparent to your theme)
    let bg = Color::Reset;

    Theme {
        name: "terminal".to_string(),
        display_name: "Terminal".to_string(),

        // Core colors - all use terminal's native palette
        bg_color: bg,
        border_color: bright_black,
        title_color: bright_cyan,
        accent_color: bright_magenta,
        text_color: white,
        success_color: green,
        dim_color: bright_black,

        // Mode colors
        mode_view_color: green,
        mode_chat_color: magenta,
        mode_plan_color: cyan,
        mode_bash_color: yellow,
        mode_leader_color: bright_magenta,

        // Special colors
        warning_color: yellow,
        error_color: red,
        code_bg_color: bg, // Same as background - clean look

        // UI element colors
        cursor_color: bright_white,
        selection_bg_color: bright_black,
        selection_fg_color: bright_white,

        // Message role colors (text)
        user_msg_color: green,
        assistant_msg_color: bright_magenta,
        system_msg_color: yellow,
        tool_msg_color: cyan,

        // Status colors
        info_color: cyan,
        progress_color: magenta,

        // Input & Form Colors
        input_bg_color: bg,
        input_placeholder_color: bright_black,
        input_border_color: bright_black,

        // Message Bubble Backgrounds - all transparent/reset
        user_msg_bg_color: bg,
        assistant_msg_bg_color: bg,
        system_msg_bg_color: bg,
        tool_msg_bg_color: bg,

        // Status Bar & UI Components
        status_bar_bg_color: bg,
        scrollbar_bg_color: bg,
        scrollbar_fg_color: bright_black,
        scrollbar_hover_color: white,

        // Branding & Logo
        logo_primary_color: bright_red,
        logo_secondary_color: red,

        // Animation & Effects
        animation_color: bright_cyan,
        processing_color: yellow,
        highlight_color: bright_yellow,
        bubble_color: cyan,

        // Token Usage Indicators
        token_low_color: green,
        token_medium_color: yellow,
        token_high_color: bright_red,
        token_critical_color: red,

        // Syntax Highlighting Colors
        syntax_keyword_color: magenta,
        syntax_function_color: blue,
        syntax_string_color: green,
        syntax_number_color: bright_magenta,
        syntax_comment_color: bright_black,
        syntax_type_color: cyan,
        syntax_variable_color: white,
        syntax_operator_color: bright_white,
        syntax_punctuation_color: white,

        // Diff & Code Display Colors
        diff_add_color: bright_green,
        diff_add_bg_color: bg,
        diff_remove_color: bright_red,
        diff_remove_bg_color: bg,
        diff_context_color: bright_black,
        line_number_color: bright_black,
        link_color: bright_blue,
        running_color: cyan,
    }
}
