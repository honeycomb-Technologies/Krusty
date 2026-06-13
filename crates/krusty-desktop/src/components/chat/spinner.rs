use std::time::Duration;

use gpui::{
    div, Animation, AnimationExt as _, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _,
};
use gpui_component::animation::cubic_bezier;

use crate::design::theme;

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn streaming_spinner(id: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .text_sm()
        .text_color(theme::accent())
        .with_animation(
            id,
            Animation::new(Duration::from_millis(80))
                .repeat()
                .with_easing(cubic_bezier(0.0, 0.0, 1.0, 1.0)),
            |this, delta| {
                let index = (delta * FRAMES.len() as f32) as usize % FRAMES.len();
                this.child(FRAMES[index])
            },
        )
}
