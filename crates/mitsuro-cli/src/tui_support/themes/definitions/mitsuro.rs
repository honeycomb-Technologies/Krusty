use super::super::Theme;
use ratatui::style::Color;

/// Original Mitsuro theme - the default dark theme
pub fn mitsuro() -> Theme {
    Theme {
        name: "mitsuro".to_string(),
        display_name: "Mitsuro Original".to_string(),
        // Original main colors from initial commit
        bg_color: Color::Rgb(24, 24, 37), // Original dark blue-gray background
        border_color: Color::Rgb(88, 91, 112), // Original muted purple-gray
        title_color: Color::Rgb(139, 233, 253), // Original bright cyan for UI titles
        accent_color: Color::Rgb(189, 147, 249), // Original purple accent
        text_color: Color::Rgb(203, 213, 225), // Original light gray text
        success_color: Color::Rgb(80, 250, 123), // Original green (unchanged)
        dim_color: Color::Rgb(148, 163, 184), // Original dimmed text
        // Mode colors (original)
        mode_view_color: Color::Rgb(80, 250, 123), // Original green (Normal mode)
        mode_chat_color: Color::Rgb(255, 121, 198), // Original pink (Insert mode)
        mode_plan_color: Color::Rgb(139, 233, 253), // Original cyan (Menu mode)
        mode_bash_color: Color::Rgb(255, 184, 108), // Original orange (Bash mode)
        mode_leader_color: Color::Rgb(189, 147, 249), // Original purple (Leader)
        // Special colors
        warning_color: Color::Rgb(255, 203, 107), // Original bright yellow
        error_color: Color::Rgb(255, 85, 85),     // Red for errors
        code_bg_color: Color::Rgb(30, 30, 45),    // Slightly lighter than main bg
        // UI element colors
        cursor_color: Color::Rgb(189, 147, 249), // Purple accent for cursor
        selection_bg_color: Color::Rgb(51, 65, 85), // Original dark slate
        selection_fg_color: Color::Rgb(203, 213, 225), // Same as text
        // Message role colors (text)
        user_msg_color: Color::Rgb(80, 250, 123), // Green
        assistant_msg_color: Color::Rgb(189, 147, 249), // Purple for assistant
        system_msg_color: Color::Rgb(255, 203, 107), // Yellow
        tool_msg_color: Color::Rgb(139, 233, 253), // Cyan
        // Status colors
        info_color: Color::Rgb(139, 233, 253),     // Cyan
        progress_color: Color::Rgb(189, 147, 249), // Purple for progress

        // Extended theme fields - Original Mitsuro colors

        // Input & Form Colors
        input_bg_color: Color::Rgb(51, 65, 85), // Original input background
        input_placeholder_color: Color::Rgb(100, 116, 139), // Original placeholder
        input_border_color: Color::Rgb(88, 91, 112), // Same as border

        // Message Bubble Backgrounds (original had unique backgrounds)
        user_msg_bg_color: Color::Rgb(40, 40, 60), // Dark purple-blue
        assistant_msg_bg_color: Color::Rgb(50, 50, 50), // Dark gray
        system_msg_bg_color: Color::Rgb(60, 40, 60), // Purple-tinted
        tool_msg_bg_color: Color::Rgb(60, 60, 40), // Yellow-tinted

        // Status Bar & UI Components
        status_bar_bg_color: Color::Rgb(44, 44, 57), // Original status bar bg
        scrollbar_bg_color: Color::Rgb(44, 44, 57),  // Darker background
        scrollbar_fg_color: Color::Rgb(88, 91, 112), // Border color
        scrollbar_hover_color: Color::Rgb(139, 233, 253), // Cyan on hover

        // Branding & Logo (Rust colors)
        logo_primary_color: Color::Rgb(255, 140, 90), // Original rust orange
        logo_secondary_color: Color::Rgb(183, 65, 14), // Original dark rust

        // Animation & Effects
        animation_color: Color::Rgb(255, 140, 90), // Rust orange for animations
        processing_color: Color::Rgb(255, 203, 107), // Yellow for processing
        highlight_color: Color::Rgb(255, 184, 108), // Golden for highlights
        bubble_color: Color::Rgb(139, 233, 253),   // Original ocean cyan for bubbles

        // Token Usage Indicators (original)
        token_low_color: Color::Rgb(80, 250, 123), // Green
        token_medium_color: Color::Rgb(255, 203, 107), // Yellow
        token_high_color: Color::Rgb(255, 121, 198), // Pink
        token_critical_color: Color::Rgb(255, 121, 198), // Pink with warning

        // Syntax Highlighting Colors
        syntax_keyword_color: Color::Rgb(255, 121, 198), // Pink for keywords
        syntax_function_color: Color::Rgb(80, 250, 123), // Green for functions
        syntax_string_color: Color::Rgb(241, 250, 140),  // Light yellow for strings
        syntax_number_color: Color::Rgb(189, 147, 249),  // Purple for numbers
        syntax_comment_color: Color::Rgb(98, 114, 164),  // Blue-gray for comments
        syntax_type_color: Color::Rgb(139, 233, 253),    // Cyan for types
        syntax_variable_color: Color::Rgb(248, 248, 242), // Off-white for variables
        syntax_operator_color: Color::Rgb(255, 121, 198), // Pink for operators
        syntax_punctuation_color: Color::Rgb(248, 248, 242), // Off-white for punctuation

        // Diff & Code Display Colors
        diff_add_color: Color::Rgb(80, 250, 123), // Green for additions
        diff_add_bg_color: Color::Rgb(20, 45, 30), // Subtle green background
        diff_remove_color: Color::Rgb(255, 85, 85), // Red for deletions
        diff_remove_bg_color: Color::Rgb(50, 25, 30), // Subtle red background
        diff_context_color: Color::Rgb(98, 114, 164), // Blue-gray for context
        line_number_color: Color::Rgb(88, 91, 112), // Same as border
        link_color: Color::Rgb(139, 233, 253),    // Cyan for links
        running_color: Color::Rgb(139, 233, 253), // Cyan for running status
    }
}
