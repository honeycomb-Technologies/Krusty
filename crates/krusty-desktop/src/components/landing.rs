use gpui::{div, px, Context, IntoElement, ParentElement as _, Styled as _};
use gpui_component::StyledExt as _;

use crate::app::KrustyDesktop;
use crate::components::button::{krusty_button, KrustyButtonKind};
use crate::design::theme;

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
                .gap_5()
                .child(
                    div()
                        .text_size(px(52.0))
                        .font_semibold()
                        .child("Krusty"),
                )
                .child(
                    div()
                        .w(px(520.0))
                        .text_center()
                        .text_color(theme::text_muted())
                        .child("A native GPUI workspace built around project tabs, Wayland-like tiled panels, and chat-first coding sessions."),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            krusty_button(
                                "landing-open-workspace",
                                "Open Workspace",
                                KrustyButtonKind::Primary,
                                cx,
                            )
                            .w(px(180.0))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.start_workspace(cx);
                            })),
                        )
                        .child(
                            krusty_button(
                                "landing-settings",
                                "Settings",
                                KrustyButtonKind::Secondary,
                                cx,
                            )
                            .w(px(132.0))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.toggle_settings(cx);
                            })),
                        ),
                ),
        )
}
