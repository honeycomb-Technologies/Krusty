//! Atlas browser surface.
//!
//! The default Linux GPUI build renders a real WebKitGTK page offscreen and presents
//! those pixels inside Atlas. The external browser remains an explicit fallback.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, img, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{BrowserSessionStatus, MitsuroApp, ATLAS_RUNTIME_KEY};
use crate::components::ui_button;
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
        .child(title_strip(session))
        .child(browser_toolbar(app, cx))
        .child(
            div()
                .id("browser-content")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(if is_blank {
                    empty_state(app.browser_embedded_available()).into_any_element()
                } else if let Some(frame) = app.browser_frame().cloned() {
                    embedded_page(app, frame, cx).into_any_element()
                } else if session.status == BrowserSessionStatus::Error {
                    error_state(session).into_any_element()
                } else {
                    current_url_state(session).into_any_element()
                }),
        )
}

fn title_strip(session: &crate::app::BrowserSession) -> impl IntoElement {
    let colors = theme::colors();
    let page_title = session.title.trim();
    let page_title = if page_title.is_empty() || page_title == "about:blank" {
        None
    } else {
        Some(page_title.to_owned())
    };
    let status = match session.status {
        BrowserSessionStatus::Connecting => Some(("Loading", colors.status_connecting)),
        BrowserSessionStatus::AgentDriving => Some(("Agent control", colors.accent)),
        BrowserSessionStatus::Error => Some(("Unavailable", colors.status_error)),
        BrowserSessionStatus::NoNativeHost => Some(("Opens externally", colors.text_tertiary)),
        BrowserSessionStatus::Idle | BrowserSessionStatus::Ready => None,
    };
    div()
        .id("atlas-title")
        .flex()
        .items_center()
        .justify_between()
        .px(px(20.0))
        .h(px(64.0))
        .border_b_1()
        .border_color(colors.border_subtle)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(9.0))
                .flex_1()
                .min_w_0()
                .child(
                    Icon::new(IconName::Globe)
                        .with_size(px(16.0))
                        .text_color(colors.text),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_base()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("Atlas"),
                        )
                        .when_some(page_title, |this, page_title| {
                            this.child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(colors.text_tertiary)
                                    .child(page_title),
                            )
                        }),
                ),
        )
        .when_some(status, |this, (label, color)| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_xs()
                    .text_color(color)
                    .child(div().size(px(6.0)).rounded_full().bg(color))
                    .child(label),
            )
        })
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
        .when(app.browser_embedded_available(), |this| {
            this.child(nav_button(
                "browser-reload",
                IconName::Redo2,
                true,
                cx,
                |app, _, cx| app.browser_reload(cx),
            ))
        })
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
                    app.browser_submit_address(window, cx);
                }))
                .child(
                    Icon::new(if app.browser_embedded_available() {
                        IconName::Globe
                    } else {
                        IconName::ExternalLink
                    })
                    .with_size(px(13.0))
                    .text_color(colors.fg_button_primary),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.fg_button_primary)
                        .child(if app.browser_embedded_available() {
                            "Go"
                        } else {
                            "Open in browser"
                        }),
                ),
        )
        .when(app.browser_embedded_available(), |this| {
            this.child(nav_button(
                "browser-open-external",
                IconName::ExternalLink,
                true,
                cx,
                |app, _, cx| app.browser_open_external(cx),
            ))
        })
}

fn nav_button(
    id: &'static str,
    icon: IconName,
    enabled: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let tooltip = match &icon {
        IconName::ArrowLeft => "Back",
        IconName::ArrowRight => "Forward",
        IconName::Redo2 => "Reload",
        IconName::ExternalLink => "Open in system browser",
        _ => "Browser action",
    };
    ui_button::icon_button(
        id,
        Icon::new(icon).with_size(px(theme::shape().icon_sm)),
        tooltip,
        ui_button::ButtonTone::Ghost,
        ui_button::ButtonSize::Medium,
        ui_button::ButtonState {
            disabled: !enabled,
            ..Default::default()
        },
        cx,
    )
    .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
}

fn empty_state(embedded: bool) -> impl IntoElement {
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
                .child(if embedded {
                    "Open a page in Atlas"
                } else {
                    "Open a page in your browser"
                }),
        )
        .child(
            div()
                .mt(px(7.0))
                .max_w(px(430.0))
                .text_sm()
                .text_center()
                .text_color(colors.text_tertiary)
                .child(if embedded {
                    "Enter an address above to load the real page inside Mitsuro's WebKit surface."
                } else {
                    "The embedded renderer is unavailable, so Atlas will open the real page in your system browser."
                }),
        )
}

fn embedded_page(
    app: &MitsuroApp,
    frame: crate::app::McpAppFrame,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let bounds = app.browser_bounds();
    let click_bounds = Arc::clone(&bounds);
    let runtime = app.browser_runtime_handle();
    let focus = app.browser_focus();
    let click_focus = focus.clone();
    let key_focus = focus.clone();
    let width = frame.width;
    let height = frame.height;
    div()
        .on_children_prepainted(move |child_bounds, _window, _cx| {
            if let Some(child) = child_bounds.first().copied() {
                if let Ok(mut stored) = bounds.lock() {
                    *stored = Some(child);
                }
                let target_width = f32::from(child.size.width).round().clamp(320.0, 1920.0) as u32;
                let target_height =
                    f32::from(child.size.height).round().clamp(240.0, 1440.0) as u32;
                if (target_width.abs_diff(width) > 2 || target_height.abs_diff(height) > 2)
                    && runtime.is_some()
                {
                    if let Some(runtime) = runtime.as_ref() {
                        let _ = runtime.resize(
                            ATLAS_RUNTIME_KEY.to_owned(),
                            target_width,
                            target_height,
                        );
                    }
                }
            }
        })
        .id("atlas-embedded-page")
        .key_context("AtlasWebKit")
        .track_focus(&focus)
        .flex()
        .flex_1()
        .min_h(px(320.0))
        .w_full()
        .overflow_hidden()
        .rounded(px(8.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .cursor_pointer()
        .on_click(
            cx.listener(move |app, event: &gpui::ClickEvent, window, cx| {
                let Some(position) = event.mouse_position() else {
                    return;
                };
                let Ok(stored) = click_bounds.lock() else {
                    return;
                };
                let Some(bounds) = *stored else {
                    return;
                };
                let rendered_width = f32::from(bounds.size.width).max(1.0);
                let rendered_height = f32::from(bounds.size.height).max(1.0);
                let x = f32::from(position.x - bounds.origin.x) * width as f32 / rendered_width;
                let y = f32::from(position.y - bounds.origin.y) * height as f32 / rendered_height;
                window.focus(&click_focus);
                app.browser_click(x, y, cx);
            }),
        )
        .on_scroll_wheel(cx.listener(|app, event: &gpui::ScrollWheelEvent, _, cx| {
            let delta = event.delta.pixel_delta(px(20.0));
            app.browser_scroll(-f32::from(delta.x), -f32::from(delta.y), cx);
            cx.stop_propagation();
        }))
        .on_key_down(
            cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                if !key_focus.is_focused(window) {
                    return;
                }
                let value = event
                    .keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| event.keystroke.key.clone());
                app.browser_key(value, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            img(frame.image)
                .w_full()
                .h_full()
                .object_fit(gpui::ObjectFit::Fill),
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

fn error_state(session: &crate::app::BrowserSession) -> impl IntoElement {
    let colors = theme::colors();
    let detail = if session.bridge_detail.trim().is_empty() {
        "The embedded browser renderer is unavailable. You can still open the address in your system browser.".to_owned()
    } else {
        session.bridge_detail.to_string()
    };
    div()
        .id("atlas-error")
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h(px(320.0))
        .px(px(24.0))
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(px(30.0))
                .text_color(colors.status_error),
        )
        .child(
            div()
                .mt(px(16.0))
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("Atlas is unavailable"),
        )
        .child(
            div()
                .mt(px(7.0))
                .max_w(px(520.0))
                .text_sm()
                .text_center()
                .text_color(colors.text_tertiary)
                .child(detail),
        )
}

fn is_blank_url(url: &str) -> bool {
    url.is_empty() || url == "about:blank" || url.starts_with("about:")
}
