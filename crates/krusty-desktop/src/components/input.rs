use gpui::{div, Div, Entity, ParentElement as _, Styled as _};
use gpui_component::input::{Input, InputState};

use crate::design::theme;

pub fn krusty_input(state: &Entity<InputState>) -> Div {
    div()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
        .child(Input::new(state).appearance(false))
}
