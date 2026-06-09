use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::StyledExt as _;

use crate::app::KrustyDesktop;
use crate::design::theme;

const CLOSE_BUTTON_WIDTH: f32 = 16.0;
const ADD_BUTTON_WIDTH: f32 = 28.0;

pub fn status_bar(app: &KrustyDesktop, cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .h(px(30.0))
        .border_b_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
        .flex()
        .items_center()
        .child(home_button(cx))
        .child(
            div()
                .id("workspace-tabs")
                .h_full()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(2.0))
                .children(app.projects().iter().enumerate().map(|(index, tab)| {
                    workspace_tab(index, &tab.title, index == app.active_project(), cx)
                }))
                .child(add_workspace_button(cx)),
        )
}

fn home_button(cx: &mut Context<KrustyDesktop>) -> gpui::Stateful<gpui::Div> {
    div()
        .id("krusty-home-logo")
        .h_full()
        .w(px(34.0))
        .flex()
        .items_center()
        .justify_center()
        .border_r_1()
        .border_color(theme::hairline())
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(cx.listener(|view, _, _window, cx| {
            view.open_landing(cx);
        }))
        .child(div().font_semibold().child("K"))
}

fn workspace_tab(
    index: usize,
    title: &str,
    selected: bool,
    cx: &mut Context<KrustyDesktop>,
) -> impl IntoElement {
    div()
        .id(("workspace-tab", index))
        .relative()
        .h_full()
        .flex()
        .items_center()
        .gap_1()
        .pl_2()
        .cursor_pointer()
        .text_sm()
        .text_color(if selected {
            theme::text()
        } else {
            theme::text_muted()
        })
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(cx.listener(move |view, _, _window, cx| {
            view.select_project_tab(index, cx);
        }))
        .child(title.to_owned())
        .child(close_tab_button(index, cx))
        .when(selected, |this| {
            this.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(2.0))
                    .bg(theme::text()),
            )
        })
}

fn close_tab_button(index: usize, cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .id(("close-workspace-tab", index))
        .w(px(CLOSE_BUTTON_WIDTH))
        .h(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_xs()
        .text_color(theme::text())
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(cx.listener(move |view, _, _window, cx| {
            cx.stop_propagation();
            view.close_project_tab(index, cx);
        }))
        .child("×")
}

fn add_workspace_button(cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .id("add-workspace-tab")
        .h_full()
        .w(px(ADD_BUTTON_WIDTH))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_lg()
        .text_color(theme::text())
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(cx.listener(|view, _, _window, cx| {
            cx.stop_propagation();
            view.start_open_workspace_flow(cx);
        }))
        .child("+")
}
