//! Client-decorated application header matching the reference desktop menu bar.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Sizable as _, TitleBar};

use crate::app::{AppMenu, MitsuroApp, ProductMode};
use crate::theme;

const HEADER_HEIGHT: f32 = 34.0;

pub fn app_header(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    TitleBar::new()
        .bg(colors.bg_under)
        .border_color(colors.border_subtle)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .gap(px(2.0))
                .child(header_icon_button(
                    "app-header-sidebar",
                    if app.thread_sidebar_visible() {
                        IconName::PanelLeftClose
                    } else {
                        IconName::PanelLeftOpen
                    },
                    "Toggle sidebar",
                    app.thread_sidebar_toggle_available(),
                    cx,
                    |app, _, cx| app.toggle_thread_sidebar(cx),
                ))
                .child(header_icon_button(
                    "app-header-back",
                    IconName::ArrowLeft,
                    "Back",
                    app.can_navigate_back(),
                    cx,
                    |app, window, cx| app.navigate_back(window, cx),
                ))
                .child(header_icon_button(
                    "app-header-forward",
                    IconName::ArrowRight,
                    "Forward",
                    app.can_navigate_forward(),
                    cx,
                    |app, window, cx| app.navigate_forward(window, cx),
                ))
                .child(header_menu_button("File", AppMenu::File, app, cx))
                .child(header_menu_button("Edit", AppMenu::Edit, app, cx))
                .child(header_menu_button("View", AppMenu::View, app, cx))
                .child(header_menu_button("Help", AppMenu::Help, app, cx)),
        )
}

pub fn app_menu_overlay(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let menu = app
        .app_menu()
        .expect("menu overlay requires an active menu");
    let colors = theme::colors();
    let left = match menu {
        AppMenu::File => 82.0,
        AppMenu::Edit => 118.0,
        AppMenu::View => 156.0,
        AppMenu::Help => 196.0,
    };
    let popup = div()
        .id("app-menu-popup")
        .absolute()
        .top(px(2.0))
        .left(px(left))
        .w(px(232.0))
        .p(px(5.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(colors.border_heavy)
        .bg(colors.bg_elevated)
        .flex()
        .flex_col()
        .gap(px(2.0))
        .children(menu_items(menu, app, cx));

    div()
        .id("app-menu-backdrop")
        .absolute()
        // Leave the title bar interactive so an open menu can switch directly
        // to another File/Edit/View/Help menu in one click.
        .top(px(HEADER_HEIGHT))
        .bottom_0()
        .left_0()
        .right_0()
        .on_click(cx.listener(|app, _, _, cx| app.close_app_menu(cx)))
        .child(popup)
}

fn menu_items(
    menu: AppMenu,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> Vec<gpui::AnyElement> {
    match menu {
        AppMenu::File => vec![
            menu_item("menu-new-chat", "New chat", true, cx, |app, window, cx| {
                app.new_codex_thread_from_menu(window, cx)
            }),
            menu_item("menu-settings", "Settings", true, cx, |app, window, cx| {
                app.set_mode(ProductMode::Settings, window, cx)
            }),
            menu_separator(),
            menu_item(
                "menu-close-window",
                "Close window",
                true,
                cx,
                |_, window, _| window.remove_window(),
            ),
        ],
        AppMenu::Edit => vec![menu_item(
            "menu-find-conversation",
            "Find in conversation",
            app.selected_thread_id().is_some(),
            cx,
            |app, window, cx| {
                app.close_app_menu(cx);
                app.open_thread_find(window, cx);
            },
        )],
        AppMenu::View => vec![
            menu_item(
                "menu-toggle-sidebar",
                if app.thread_sidebar_visible() {
                    "Hide sidebar"
                } else {
                    "Show sidebar"
                },
                app.thread_sidebar_toggle_available(),
                cx,
                |app, _, cx| app.toggle_thread_sidebar(cx),
            ),
            menu_separator(),
            menu_item("menu-work", "Work", true, cx, |app, window, cx| {
                app.set_mode(ProductMode::Work, window, cx)
            }),
            menu_item("menu-terminal", "Terminal", true, cx, |app, window, cx| {
                app.set_mode(ProductMode::Terminal, window, cx)
            }),
            menu_item("menu-files", "Files", true, cx, |app, window, cx| {
                app.set_mode(ProductMode::Files, window, cx)
            }),
        ],
        AppMenu::Help => vec![
            menu_item(
                "menu-feedback",
                "Send feedback",
                true,
                cx,
                |app, window, cx| {
                    app.close_app_menu(cx);
                    app.open_feedback_dialog(window, cx);
                },
            ),
            menu_item(
                "menu-documentation",
                "Mitsuro documentation",
                true,
                cx,
                |app, _, cx| app.open_help_documentation(cx),
            ),
        ],
    }
}

fn header_icon_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    enabled: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .size(px(28.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
        })
        .when(!enabled, |this| this.opacity(0.35))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(
            Icon::new(icon)
                .with_size(px(13.0))
                .text_color(colors.text_tertiary),
        )
}

fn header_menu_button(
    label: &'static str,
    menu: AppMenu,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let selected = app.app_menu() == Some(menu);
    div()
        .id(match menu {
            AppMenu::File => "app-menu-file",
            AppMenu::Edit => "app-menu-edit",
            AppMenu::View => "app-menu-view",
            AppMenu::Help => "app-menu-help",
        })
        .h(px(26.0))
        .px(px(7.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| app.toggle_app_menu(menu, cx)))
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
        )
}

fn menu_item(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> gpui::AnyElement {
    let colors = theme::colors();
    div()
        .id(id)
        .h(px(30.0))
        .px(px(9.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
        })
        .when(!enabled, |this| this.opacity(0.38))
        .child(
            div()
                .text_sm()
                .text_color(colors.text_secondary)
                .child(label),
        )
        .into_any_element()
}

fn menu_separator() -> gpui::AnyElement {
    let colors = theme::colors();
    div()
        .h(px(1.0))
        .mx(px(5.0))
        .my(px(3.0))
        .bg(colors.border_heavy)
        .into_any_element()
}
