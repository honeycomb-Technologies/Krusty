//! Confirmation gate for MCP App initiated `ui/message` follow-ups.

use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::scroll::ScrollableElement as _;

use crate::app::MitsuroApp;
use crate::theme;

pub fn mcp_app_message_dialog(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let (title, prompt, image_count) = app
        .pending_mcp_app_message()
        .expect("dialog renders only while an MCP app follow-up is pending");
    let detail = if image_count == 0 {
        prompt.to_owned()
    } else if prompt.is_empty() {
        format!("{image_count} image attachment(s)")
    } else {
        format!("{prompt}\n\n{image_count} image attachment(s)")
    };

    div()
        .id("mcp-app-message-dialog-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::hex_alpha(0x000000, 0.66))
        .child(
            div()
                .id("mcp-app-message-dialog")
                .w(px(520.0))
                .max_w_full()
                .mx(px(24.0))
                .rounded(px(16.0))
                .border_1()
                .border_color(colors.border_heavy)
                .bg(colors.bg_elevated)
                .p(px(20.0))
                .flex()
                .flex_col()
                .gap(px(16.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(format!("Send a message from {title}?")),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.text_secondary)
                                .child("This starts a real model turn in the current chat."),
                        ),
                )
                .child(
                    div()
                        .max_h(px(240.0))
                        .overflow_y_scrollbar()
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(colors.border_subtle)
                        .bg(colors.bg_sidebar)
                        .p(px(12.0))
                        .text_sm()
                        .text_color(colors.text)
                        .whitespace_normal()
                        .child(detail),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            div()
                                .id("mcp-app-message-cancel")
                                .px(px(12.0))
                                .py(px(7.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(colors.border)
                                .cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(|app, _, _window, cx| {
                                    app.cancel_mcp_app_message(cx);
                                }))
                                .child("Cancel"),
                        )
                        .child(
                            div()
                                .id("mcp-app-message-confirm")
                                .px(px(12.0))
                                .py(px(7.0))
                                .rounded(px(8.0))
                                .bg(colors.accent)
                                .text_color(colors.fg_button_primary)
                                .cursor_pointer()
                                .hover(|style| style.opacity(0.9))
                                .on_click(cx.listener(|app, _, _window, cx| {
                                    app.confirm_mcp_app_message(cx);
                                }))
                                .child("Send message"),
                        ),
                ),
        )
}
