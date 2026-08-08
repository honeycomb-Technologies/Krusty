//! Pull requests destination.
//!
//! Neither Mitsuro HTTP nor Codex app-server currently exposes a typed pull-request
//! contract. Keep the ChatGPT desktop navigation destination for parity, but render
//! an honest capability state instead of fixture repositories and review controls.

use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, UiConnection};
use crate::theme;

pub fn pull_requests_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let backend = app
        .active_backend_kind()
        .map(MitsuroApp::backend_display_name)
        .unwrap_or("No backend selected");
    let connection = connection_label(app.connection());
    let github = app
        .mcp_github_server()
        .map(|server| server.display_title().to_string());

    div()
        .id("pull-requests-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .px(px(28.0))
                .pt(px(28.0))
                .pb(px(18.0))
                .border_b_1()
                .border_color(colors.border_subtle)
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("Pull requests"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child("Review work connected to the active coding backend."),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .px(px(28.0))
                .pb(px(56.0))
                .child(
                    div()
                        .id("pull-requests-unavailable")
                        .flex()
                        .flex_col()
                        .w_full()
                        .max_w(px(520.0))
                        .items_center()
                        .child(
                            Icon::new(IconName::GitHub)
                                .with_size(px(30.0))
                                .text_color(colors.text_tertiary),
                        )
                        .child(
                            div()
                                .mt(px(16.0))
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("Pull requests are not available yet"),
                        )
                        .child(
                            div()
                                .mt(px(7.0))
                                .max_w(px(430.0))
                                .text_sm()
                                .text_center()
                                .text_color(colors.text_tertiary)
                                .child(
                                    "This backend does not expose pull-request listing or review methods. No sample repositories are shown.",
                                ),
                        )
                        .child(
                            div()
                                .mt(px(24.0))
                                .w_full()
                                .border_t_1()
                                .border_color(colors.border_subtle)
                                .child(status_row("Backend", backend))
                                .child(status_row("Connection", connection))
                                .child(status_row(
                                    "GitHub connection",
                                    github.as_deref().unwrap_or("Not detected"),
                                ))
                                .child(status_row("Pull-request API", "Unavailable")),
                        )
                        .child(
                            div()
                                .id("pull-requests-reconnect")
                                .mt(px(20.0))
                                .h(px(34.0))
                                .px(px(14.0))
                                .rounded(px(9.0))
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.bg_button_secondary)
                                .cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.reconnect_backend(cx);
                                }))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(colors.text)
                                        .child("Reconnect backend"),
                                ),
                        ),
                ),
        )
}

fn status_row(label: &'static str, value: impl Into<String>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
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
                .text_sm()
                .text_color(colors.text_tertiary)
                .child(value.into()),
        )
}

fn connection_label(connection: &UiConnection) -> &'static str {
    match connection {
        UiConnection::Ready { .. } => "Ready",
        UiConnection::Connecting => "Connecting",
        UiConnection::Error { .. } => "Error",
        UiConnection::Fixture => "Offline fixtures",
        UiConnection::Demo => "Demo",
    }
}
