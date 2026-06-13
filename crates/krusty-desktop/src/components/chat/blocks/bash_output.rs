use gpui::prelude::FluentBuilder as _;
use gpui::{div, AnyElement, IntoElement, ParentElement as _, Styled as _};
use gpui_component::StyledExt as _;

use crate::components::chat::spinner::streaming_spinner;
use crate::design::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BashOutputBlockState {
    pub id: String,
    pub output: String,
    pub running: bool,
}

pub fn bash_output_block(state: &BashOutputBlockState) -> AnyElement {
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
                        .child("bash"),
                )
                .when(state.running, |this| {
                    this.child(streaming_spinner("bash-spinner"))
                }),
        )
        .child(div().text_xs().text_color(theme::text()).child(
            if state.output.is_empty() && state.running {
                "Running…".to_owned()
            } else {
                state.output.clone()
            },
        ))
        .into_any_element()
}
