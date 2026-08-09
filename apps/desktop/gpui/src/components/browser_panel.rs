//! Atlas browser surface.
//!
//! The default GPUI build does not embed a web renderer. Atlas therefore acts as an
//! explicit system-browser bridge with local URL history; it never renders invented
//! page content or implies that an agent can inspect a page opened elsewhere.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{BrowserSessionStatus, MitsuroApp};
use crate::theme;

pub fn browser_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let session = app.browser_session();
    let is_blank = is_blank_url(session.url.as_ref());

    div()
        .id("browser-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(title_strip(session.status))
        .child(browser_toolbar(app, cx))
        .child(
            div()
                .id("browser-content")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .px(px(28.0))
                .pb(px(28.0))
                .child(if is_blank {
                    empty_state().into_any_element()
                } else {
                    current_url_state(session).into_any_element()
                })
                .child(bridge_status(session)),
        )
}

fn title_strip(status: BrowserSessionStatus) -> impl IntoElement {
    let colors = theme::colors();
    let label = match status {
        BrowserSessionStatus::Error => "Unavailable",
        BrowserSessionStatus::NoNativeHost => "System browser",
        _ => "Browser bridge",
    };
    div()
        .id("atlas-title")
        .flex()
        .items_center()
        .justify_between()
        .px(px(20.0))
        .py(px(12.0))
        .border_b_1()
        .border_color(colors.border_subtle)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(9.0))
                .child(
                    Icon::new(IconName::Globe)
                        .with_size(px(16.0))
                        .text_color(colors.text),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("Atlas"),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
        )
}

fn browser_toolbar(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let session = app.browser_session();
    let url_input = app.browser_url_input().clone();

    div()
        .id("browser-toolbar")
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(colors.border_subtle)
        .child(nav_button(
            "browser-back",
            IconName::ArrowLeft,
            session.can_go_back,
            cx,
            |app, window, cx| app.browser_go_back(window, cx),
        ))
        .child(nav_button(
            "browser-forward",
            IconName::ArrowRight,
            session.can_go_forward,
            cx,
            |app, window, cx| app.browser_go_forward(window, cx),
        ))
        .child(
            div()
                .id("browser-url-bar")
                .flex()
                .items_center()
                .flex_1()
                .min_w_0()
                .h(px(34.0))
                .px(px(11.0))
                .gap(px(7.0))
                .rounded(px(9.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .child(
                    Icon::new(IconName::Globe)
                        .with_size(px(13.0))
                        .text_color(colors.text_tertiary),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(&url_input).appearance(false).h(px(28.0))),
                ),
        )
        .child(
            div()
                .id("browser-open")
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(34.0))
                .px(px(13.0))
                .rounded(px(9.0))
                .bg(colors.bg_button_primary)
                .cursor_pointer()
                .hover(|style| style.bg(colors.bg_button_primary_hover))
                .on_click(cx.listener(|app, _, window, cx| {
                    app.browser_navigate(window, cx);
                    app.browser_open_external(cx);
                }))
                .child(
                    Icon::new(IconName::ExternalLink)
                        .with_size(px(13.0))
                        .text_color(colors.fg_button_primary),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.fg_button_primary)
                        .child("Open in browser"),
                ),
        )
}

fn nav_button(
    id: &'static str,
    icon: IconName,
    enabled: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .size(px(34.0))
        .rounded(px(9.0))
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
        })
        .when(!enabled, |this| this.opacity(0.35))
        .child(
            Icon::new(icon)
                .with_size(px(15.0))
                .text_color(colors.text_secondary),
        )
}

fn empty_state() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("atlas-empty")
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h(px(320.0))
        .pb(px(36.0))
        .child(
            Icon::new(IconName::Globe)
                .with_size(px(30.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .mt(px(16.0))
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("Open a page in your browser"),
        )
        .child(
            div()
                .mt(px(7.0))
                .max_w(px(430.0))
                .text_sm()
                .text_center()
                .text_color(colors.text_tertiary)
                .child(
                    "This build records URL history here and opens the real page in your system browser. Page content is not available inside Mitsuro.",
                ),
        )
}

fn current_url_state(session: &crate::app::BrowserSession) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("atlas-current-url")
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h(px(320.0))
        .pb(px(36.0))
        .child(
            Icon::new(IconName::ExternalLink)
                .with_size(px(28.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .mt(px(16.0))
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(session.title.to_string()),
        )
        .child(
            div()
                .mt(px(6.0))
                .max_w(px(560.0))
                .text_sm()
                .text_center()
                .text_color(colors.text_tertiary)
                .child(session.url.to_string()),
        )
        .child(
            div()
                .mt(px(18.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(session.page_body.to_string()),
        )
}

fn bridge_status(session: &crate::app::BrowserSession) -> impl IntoElement {
    let colors = theme::colors();
    let state = match session.status {
        BrowserSessionStatus::Error => "Error".to_string(),
        _ => session.bridge_mode.to_string(),
    };
    let detail = session.bridge_detail.to_string();
    let host = session
        .engine_version
        .as_ref()
        .map(|version| format!("{} · WebKit {version}", session.host_kind))
        .unwrap_or_else(|| session.host_kind.to_string());
    div()
        .id("browser-session-status")
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(colors.border_subtle)
        .child(status_row("Browser surface", host))
        .child(status_row("Bridge", format!("{state} · {detail}")))
}

fn status_row(label: &'static str, value: String) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(18.0))
        .min_h(px(42.0))
        .border_b_1()
        .border_color(colors.border_subtle)
        .child(
            div()
                .text_sm()
                .text_color(colors.text_secondary)
                .child(label),
        )
        .child(
            div()
                .max_w(px(520.0))
                .text_sm()
                .text_color(colors.text_tertiary)
                .child(value),
        )
}

fn is_blank_url(url: &str) -> bool {
    url.is_empty() || url == "about:blank" || url.starts_with("about:")
}
