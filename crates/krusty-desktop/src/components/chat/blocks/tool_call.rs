use gpui::prelude::FluentBuilder as _;
use gpui::{div, AnyElement, IntoElement, ParentElement as _, Styled as _};
use gpui_component::tag::Tag;
use gpui_component::StyledExt as _;
use gpui_component::{Sizable as _, Size};

use crate::design::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolCallStatus {
    Started,
    Executing,
    Complete,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallBlockState {
    pub id: String,
    pub name: String,
    pub status: ToolCallStatus,
    pub output: String,
}

pub fn tool_call_block(state: &ToolCallBlockState) -> AnyElement {
    let status_label = match state.status {
        ToolCallStatus::Started => "started",
        ToolCallStatus::Executing => "running",
        ToolCallStatus::Complete => "done",
        ToolCallStatus::Error => "error",
    };

    div()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
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
                        .text_color(theme::text())
                        .child(format!("Tool: {}", state.name)),
                )
                .child(
                    Tag::secondary()
                        .outline()
                        .with_size(Size::Small)
                        .child(status_label),
                ),
        )
        .when(!state.output.is_empty(), |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(theme::text_muted())
                    .child(state.output.clone()),
            )
        })
        .into_any_element()
}
