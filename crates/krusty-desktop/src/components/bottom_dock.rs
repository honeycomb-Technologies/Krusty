use crate::app::KrustyDesktop;
use crate::design::theme;
use crate::panels::{PanelKind, SplitAxis};
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::Icon;

const CHAT_ICON: &str = "icons/chat-robot.svg";
const EASEL_ICON: &str = "icons/easel.svg";
const SETTINGS_ICON: &str = "icons/settings.svg";

pub fn bottom_dock(app: &KrustyDesktop, cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .h(px(42.0))
        .border_t_1()
        .border_color(theme::hairline())
        .bg(theme::app_bg())
        .px_2()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    dock_icon("dock-chat", CHAT_ICON, false).on_click(cx.listener(
                        |view, _, _, cx| {
                            view.split_focused(SplitAxis::Horizontal, PanelKind::Chat, cx);
                        },
                    )),
                )
                .child(
                    dock_icon("dock-draw", EASEL_ICON, false).on_click(cx.listener(
                        |view, _, _, cx| {
                            view.split_focused(SplitAxis::Horizontal, PanelKind::ScratchCanvas, cx);
                        },
                    )),
                ),
        )
        .child(div().flex().items_center().gap_1().child(
            dock_icon("dock-settings", SETTINGS_ICON, app.settings_open()).on_click(cx.listener(
                |view, _, _, cx| {
                    view.toggle_settings(cx);
                },
            )),
        ))
}

fn dock_icon(
    id: &'static str,
    icon_path: &'static str,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(34.0))
        .bg(if selected {
            theme::surface_selected()
        } else {
            gpui::transparent_black()
        })
        .hover(|style| style.bg(theme::surface_hover()))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_color(if selected {
            theme::accent()
        } else {
            theme::text_muted()
        })
        .child(Icon::empty().path(icon_path).size(px(22.0)))
}
