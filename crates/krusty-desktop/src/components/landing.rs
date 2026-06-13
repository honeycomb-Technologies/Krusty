use gpui::{div, px, Context, IntoElement, ParentElement as _, Styled as _};
use gpui_component::button::Button;
use gpui_component::Icon;

use crate::app::KrustyDesktop;
use crate::components::brand;
use crate::components::button::{krusty_icon_button, KrustyButtonKind};
use crate::design::theme;

const OPEN_FOLDER_ICON: &str = "icons/folder-open.svg";
const NEW_FOLDER_ICON: &str = "icons/folder-plus.svg";
const MAKO_ICON: &str = "icons/mako-shark.svg";

pub fn landing_page(cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px_8()
        .child(
            div()
                .mb(px(54.0))
                .flex()
                .flex_col()
                .items_center()
                .gap_4()
                .child(brand::animated_wordmark())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            landing_icon_button(
                                "landing-new-workspace",
                                NEW_FOLDER_ICON,
                                "New folder",
                                cx,
                            )
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.start_new_workspace(cx);
                            })),
                        )
                        .child(landing_divider())
                        .child(
                            landing_icon_button(
                                "landing-open-workspace",
                                OPEN_FOLDER_ICON,
                                "Open folder",
                                cx,
                            )
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.start_workspace(cx);
                            })),
                        )
                        .child(landing_divider())
                        .child(
                            landing_icon_button("landing-mako", MAKO_ICON, "Open Mako", cx)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.open_mako(cx);
                                })),
                        ),
                ),
        )
}

fn landing_icon_button(
    id: &'static str,
    icon_path: &'static str,
    tooltip: &'static str,
    cx: &gpui::App,
) -> Button {
    krusty_icon_button(
        id,
        Icon::empty().path(icon_path).size(px(24.0)),
        KrustyButtonKind::Ghost,
        cx,
    )
    .w(px(42.0))
    .h(px(38.0))
    .tooltip(tooltip)
}

fn landing_divider() -> impl IntoElement {
    div().w(px(1.0)).h(px(28.0)).bg(theme::hairline())
}
