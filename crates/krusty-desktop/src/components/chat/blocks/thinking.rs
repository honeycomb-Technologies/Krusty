use gpui::prelude::FluentBuilder as _;
use gpui::{div, AnyElement, IntoElement, ParentElement as _, Styled as _};
use gpui_component::StyledExt as _;

use crate::components::chat::spinner::streaming_spinner;
use crate::design::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinkingBlockState {
    pub content: String,
    pub expanded: bool,
    pub streaming: bool,
}

pub fn thinking_block(state: &ThinkingBlockState) -> AnyElement {
    let preview = state
        .content
        .lines()
        .next()
        .unwrap_or("Thinking…")
        .chars()
        .take(80)
        .collect::<String>();

    div()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::app_bg())
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
                        .child("Thinking"),
                )
                .when(state.streaming, |this| {
                    this.child(streaming_spinner("thinking-spinner"))
                }),
        )
        .child(if state.expanded {
            div()
                .text_xs()
                .text_color(theme::text())
                .child(state.content.clone())
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(theme::text_muted())
                .child(preview)
                .into_any_element()
        })
        .into_any_element()
}
