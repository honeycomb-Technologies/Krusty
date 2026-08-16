//! Client-decorated application header matching the reference desktop menu bar.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    deferred, div, point, px, BoxShadow, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _, TitleBar};

use crate::app::{AppMenu, MitsuroApp, ProductMode};
use crate::components::ui_button::{self, ButtonSize, ButtonState, ButtonTone};
use crate::theme;

pub fn app_header(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    TitleBar::new()
        .h(px(theme::metrics().title_bar_height))
        .bg(colors.bg_under)
        .border_color(colors.border_subtle)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .h_full()
                .gap(px(theme::spacing().xxs))
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
        .shadow(vec![BoxShadow {
            color: colors.shadow,
            offset: point(px(0.0), px(theme::spacing().xs)),
            blur_radius: px(theme::shape().shadow_blur),
            spread_radius: px(0.0),
        }])
        .flex()
        .flex_col()
        .gap(px(2.0))
        .children(menu_items(menu, app, cx));

    deferred(
        div()
            .id("app-menu-backdrop")
            .occlude()
            .absolute()
            // Leave the title bar interactive so an open menu can switch directly
            // to another File/Edit/View/Help menu in one click.
            .top(px(theme::metrics().title_bar_height))
            .bottom_0()
            .left_0()
            .right_0()
            .on_click(cx.listener(|app, _, _, cx| {
                app.close_app_menu(cx);
                cx.stop_propagation();
            }))
            .child(popup),
    )
    .with_priority(100)
}

fn menu_items(
    menu: AppMenu,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> Vec<gpui::AnyElement> {
    match menu {
        AppMenu::File => vec![
            menu_item(
                "menu-new-chat",
                "New chat",
                Some("Ctrl+N"),
                true,
                cx,
                |app, window, cx| app.new_conversation_from_menu(window, cx),
            ),
            menu_item(
                "menu-settings",
                "Settings",
                Some("Ctrl+,"),
                true,
                cx,
                |app, window, cx| app.set_mode(ProductMode::Settings, window, cx),
            ),
            menu_separator(),
            menu_item(
                "menu-close-window",
                "Close window",
                None,
                true,
                cx,
                |_, window, _| window.remove_window(),
            ),
        ],
        AppMenu::Edit => vec![menu_item(
            "menu-find-conversation",
            "Find in conversation",
            None,
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
                Some("Ctrl+B"),
                app.thread_sidebar_toggle_available(),
                cx,
                |app, _, cx| app.toggle_thread_sidebar(cx),
            ),
            menu_separator(),
            menu_item(
                "menu-work",
                "Work",
                Some("Ctrl+2"),
                true,
                cx,
                |app, window, cx| app.set_mode(ProductMode::Work, window, cx),
            ),
            menu_item(
                "menu-terminal",
                "Terminal",
                Some("Ctrl+`"),
                true,
                cx,
                |app, window, cx| app.set_mode(ProductMode::Terminal, window, cx),
            ),
            menu_item("menu-files", "Files", None, true, cx, |app, window, cx| {
                app.set_mode(ProductMode::Files, window, cx)
            }),
        ],
        AppMenu::Help => vec![
            menu_item(
                "menu-feedback",
                "Send feedback",
                None,
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
                None,
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
    ui_button::icon_button(
        id,
        Icon::new(icon).with_size(px(theme::shape().icon_sm)),
        tooltip,
        ButtonTone::Ghost,
        ButtonSize::Medium,
        ButtonState {
            disabled: !enabled,
            ..ButtonState::default()
        },
        cx,
    )
    .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
}

fn header_menu_button(
    label: &'static str,
    menu: AppMenu,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let selected = app.app_menu() == Some(menu);
    ui_button::button(
        match menu {
            AppMenu::File => "app-menu-file",
            AppMenu::Edit => "app-menu-edit",
            AppMenu::View => "app-menu-view",
            AppMenu::Help => "app-menu-help",
        },
        label,
        ButtonTone::Ghost,
        ButtonSize::Small,
        ButtonState {
            selected,
            ..ButtonState::default()
        },
        cx,
    )
    .on_click(cx.listener(move |app, _, _, cx| app.toggle_app_menu(menu, cx)))
}

fn menu_item(
    id: &'static str,
    label: &'static str,
    shortcut: Option<&'static str>,
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
        .justify_between()
        .gap(px(14.0))
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| {
                    on_click(app, window, cx);
                    app.close_app_menu(cx);
                    cx.stop_propagation();
                }))
        })
        .when(!enabled, |this| this.opacity(0.38))
        .child(
            div()
                .text_sm()
                .text_color(colors.text_secondary)
                .child(label),
        )
        .when_some(shortcut, |this, shortcut| {
            this.child(
                div()
                    .text_xs()
                    .font_family("monospace")
                    .text_color(colors.text_tertiary)
                    .child(shortcut),
            )
        })
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
