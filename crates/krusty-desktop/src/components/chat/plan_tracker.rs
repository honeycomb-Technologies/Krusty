use gpui::{div, AnyElement, IntoElement, ParentElement as _, Styled as _};
use gpui_component::tag::Tag;
use gpui_component::StyledExt as _;
use gpui_component::{Sizable as _, Size};

use crate::api::PlanItem;
use crate::design::theme;

pub fn plan_tracker(items: &[PlanItem]) -> Option<AnyElement> {
    if items.is_empty() {
        return None;
    }

    let completed = items.iter().filter(|item| item.completed).count();
    Some(
        div()
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::surface())
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
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
                            .child("Plan"),
                    )
                    .child(
                        Tag::secondary()
                            .outline()
                            .with_size(Size::Small)
                            .child(format!("{completed}/{}", items.len())),
                    ),
            )
            .children(items.iter().enumerate().map(|(index, item)| {
                div()
                    .text_xs()
                    .text_color(if item.completed {
                        theme::text_muted()
                    } else {
                        theme::text()
                    })
                    .child(format!(
                        "{}. {}{}",
                        index + 1,
                        item.content,
                        if item.completed { " ✓" } else { "" }
                    ))
            }))
            .into_any_element(),
    )
}
