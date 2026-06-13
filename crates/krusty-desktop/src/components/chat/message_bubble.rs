use gpui::prelude::FluentBuilder as _;
use gpui::{div, AnyElement, IntoElement, ParentElement as _, Styled as _};
use gpui_component::StyledExt as _;

use crate::components::chat::markdown_view::markdown_view;
use crate::components::chat::spinner::streaming_spinner;
use crate::design::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

pub fn message_bubble(role: MessageRole, content: &str, streaming: bool) -> impl IntoElement {
    let (label, border, bg) = match role {
        MessageRole::User => ("You", theme::accent(), theme::app_bg()),
        MessageRole::Assistant => ("Krusty", theme::hairline(), theme::surface_selected()),
        MessageRole::System => ("System", theme::hairline(), theme::app_bg()),
    };

    div()
        .border_1()
        .border_color(border)
        .bg(bg)
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme::text_muted())
                        .child(label),
                )
                .when(streaming && role == MessageRole::Assistant, |this| {
                    this.child(streaming_spinner("assistant-stream-spinner"))
                }),
        )
        .child(message_body(role, content, streaming))
}

fn message_body(role: MessageRole, content: &str, streaming: bool) -> AnyElement {
    if content.is_empty() && streaming && role == MessageRole::Assistant {
        return div()
            .text_sm()
            .text_color(theme::text_muted())
            .child("Streaming…")
            .into_any_element();
    }

    match role {
        MessageRole::Assistant => markdown_view(content).into_any_element(),
        _ => div()
            .text_sm()
            .text_color(theme::text())
            .child(content.to_owned())
            .into_any_element(),
    }
}
