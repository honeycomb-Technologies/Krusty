use gpui::{div, Div, Styled as _};

use crate::design::theme;

#[allow(dead_code)]
pub fn quiet_panel() -> Div {
    div()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::app_bg())
}

#[allow(dead_code)]
pub fn raised_panel() -> Div {
    div()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
}
