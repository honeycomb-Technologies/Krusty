use gpui::prelude::FluentBuilder as _;
use gpui::{div, App, ParentElement as _, Styled as _};
use gpui_component::button::Button;

use crate::components::button::{krusty_button, KrustyButtonKind};
use crate::design::theme;

pub fn control_pill(
    id: &'static str,
    label: impl Into<gpui::SharedString>,
    active: bool,
    accent: bool,
    cx: &App,
) -> Button {
    let kind = if active || accent {
        KrustyButtonKind::Secondary
    } else {
        KrustyButtonKind::Ghost
    };

    krusty_button(id, label, kind, cx)
        .px_2()
        .py_1()
        .text_xs()
        .when(active, |this| this.bg(theme::surface_selected()))
}

pub fn control_pill_row(children: Vec<gpui::AnyElement>) -> gpui::Div {
    div().flex().flex_wrap().gap_1().children(children)
}
