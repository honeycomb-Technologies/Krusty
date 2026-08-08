//! Atlas / browser-use surface (Codex-like browser chrome).
//!
//! # Host
//!
//! Navigation is driven by [`crate::browser::BrowserHost`] (`DesktopBrowserHost`):
//! history stack, mock page bodies, optional wry/WebKitGTK link (`browser-native`).
//!
//! # Bridge (P12)
//!
//! True wry child embed into the GPUI Atlas panel is **not production-ready** on this
//! tree (Wayland + Blade compositor; wry child is X11-only and needs a GTK loop).
//! Atlas instead:
//!
//! 1. Probes `HasWindowHandle` on Atlas open (`NativeWebViewHost::attach_after_window_open`)
//! 2. Keeps an improved in-panel mock driven by live history
//! 3. **Open external** → `xdg-open` / Chromium `--app=` sibling (`browser-external`)
//! 4. Optional auto-bridge: `MITSURO_ATLAS_EXTERNAL=1` / `MITSURO_ATLAS_SIBLING=1`
//! 5. Optional embed probe: `MITSURO_ATLAS_EMBED=1` (X11 only, fail soft)

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{BrowserSessionStatus, MitsuroApp};
use crate::theme;

/// Full-height Atlas browser panel: toolbar + mock page + session status.
pub fn browser_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let session = app.browser_session();
    let url = session.url.as_ref();
    let is_blank = url.is_empty()
        || url == "about:blank"
        || url == "mitsuro://atlas"
        || url.starts_with("about:");

    div()
        .id("browser-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(atlas_title_strip())
        .child(browser_toolbar(app, cx))
        .child(
            div()
                .id("browser-content")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .px(px(20.0))
                .py(px(16.0))
                .gap(px(12.0))
                .child(if is_blank {
                    atlas_empty_state().into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .child(page_title_row(session.title.as_ref(), session.url.as_ref()))
                        .child(mock_page_card(session))
                        .into_any_element()
                })
                .child(div().flex_1())
                .child(session_status_bar(session)),
        )
}

fn atlas_title_strip() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("atlas-title")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(16.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
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
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child("· browser-use"),
                ),
        )
        .child(
            div()
                .text_xs()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_color(colors.text_secondary)
                .child("Ready"),
        )
}

fn atlas_empty_state() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("atlas-empty")
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .flex_1()
        .min_h(px(280.0))
        .w_full()
        .rounded(px(14.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .gap(px(14.0))
        .px(px(28.0))
        .py(px(40.0))
        .child(
            div()
                .w(px(56.0))
                .h(px(56.0))
                .rounded(px(16.0))
                .bg(colors.accent_soft)
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(colors.border)
                .child(
                    Icon::new(IconName::Globe)
                        .with_size(px(24.0))
                        .text_color(colors.accent),
                ),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("Browse the web"),
        )
        .child(
            div()
                .text_sm()
                .text_color(colors.text_secondary)
                .text_center()
                .max_w(px(400.0))
                .child(
                    "Enter a URL above and press Go. Use Open external to launch your \
                     system browser when needed.",
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .mt(px(4.0))
                .child(
                    div()
                        .text_xs()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(999.0))
                        .bg(colors.bg_sidebar)
                        .border_1()
                        .border_color(colors.border)
                        .text_color(colors.text_tertiary)
                        .child("https://…"),
                )
                .child(
                    div()
                        .text_xs()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(999.0))
                        .bg(colors.bg_sidebar)
                        .border_1()
                        .border_color(colors.border)
                        .text_color(colors.text_tertiary)
                        .child("Back / Forward"),
                ),
        )
}

fn browser_toolbar(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let session = app.browser_session();
    let can_back = session.can_go_back;
    let can_forward = session.can_go_forward;
    let url_input = app.browser_url_input().clone();

    div()
        .id("browser-toolbar")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        // Back
        .child(nav_btn(
            "browser-back",
            IconName::ArrowLeft,
            "Back",
            can_back,
            cx,
            |app, window, cx| app.browser_go_back(window, cx),
        ))
        // Forward
        .child(nav_btn(
            "browser-forward",
            IconName::ArrowRight,
            "Forward",
            can_forward,
            cx,
            |app, window, cx| app.browser_go_forward(window, cx),
        ))
        // Editable URL bar
        .child(
            div()
                .id("browser-url-bar")
                .flex()
                .flex_row()
                .items_center()
                .flex_1()
                .min_w_0()
                .h(px(34.0))
                .px(px(12.0))
                .gap(px(8.0))
                .rounded(px(10.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border_heavy)
                .child(
                    Icon::new(IconName::Globe)
                        .with_size(px(14.0))
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
        // Go
        .child(
            div()
                .id("browser-go")
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(34.0))
                .px(px(14.0))
                .rounded(px(10.0))
                .bg(colors.accent_soft)
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, window, cx| {
                    app.browser_navigate(window, cx);
                }))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.accent)
                        .child("Go"),
                ),
        )
        // Open external / sibling bridge
        .child(
            div()
                .id("browser-open-external")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(34.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, _, cx| {
                    app.browser_open_external(cx);
                }))
                .child(
                    Icon::new(IconName::ExternalLink)
                        .with_size(px(14.0))
                        .text_color(colors.text_secondary),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_secondary)
                        .child("Open external"),
                ),
        )
        // Import profile (discovery stub)
        .child(
            div()
                .id("browser-import-profile")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(34.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, _, cx| {
                    app.browser_import_profile_stub(cx);
                }))
                .child(
                    Icon::new(IconName::FolderOpen)
                        .with_size(px(14.0))
                        .text_color(colors.text_secondary),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_secondary)
                        .child("Import Chrome/Atlas profile"),
                ),
        )
}

fn nav_btn(
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
        .w(px(34.0))
        .h(px(34.0))
        .rounded(px(10.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(colors.bg_button_secondary)
        .border_1()
        .border_color(colors.border)
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| {
                    let _ = tooltip;
                    on_click(app, window, cx);
                }))
        })
        .when(!enabled, |this| this.opacity(0.45))
        .child(Icon::new(icon).with_size(px(16.0)).text_color(if enabled {
            colors.text
        } else {
            colors.text_tertiary
        }))
}

fn page_title_row(title: &str, url: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("browser-page-title")
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(url.to_string()),
        )
}

/// Mock page card — content from host history; bridge status until embed lands.
fn mock_page_card(session: &crate::app::BrowserSession) -> impl IntoElement {
    let colors = theme::colors();
    let title = session.title.to_string();
    let url = session.url.to_string();
    let profile = session.profile_label.to_string();
    let body = session.page_body.to_string();
    let host_kind = session.host_kind.to_string();
    let bridge_mode = session.bridge_mode.to_string();
    let bridge_detail = session.bridge_detail.to_string();
    let host_chip = if let Some(ver) = session.engine_version.as_ref() {
        format!("{host_kind} · {ver}")
    } else {
        host_kind
    };
    let surface_note = match session.status {
        BrowserSessionStatus::NoNativeHost => "In-panel preview · external browser available",
        _ => "In-panel mock · use Open external for real page",
    };

    div()
        .id("browser-mock-page")
        .flex()
        .flex_col()
        .w_full()
        .max_w(px(720.0))
        .rounded(px(14.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border_heavy)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(12.0))
                .py(px(8.0))
                .bg(colors.bg_sidebar)
                .border_b_1()
                .border_color(colors.border)
                .child(dot(colors.status_error))
                .child(dot(colors.status_connecting))
                .child(dot(colors.status_ready))
                .child(
                    div()
                        .ml(px(8.0))
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(surface_note),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .px(px(20.0))
                .py(px(24.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(40.0))
                                .h(px(40.0))
                                .rounded(px(10.0))
                                .bg(colors.accent_soft)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::Globe)
                                        .with_size(px(20.0))
                                        .text_color(colors.accent),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.text)
                                        .child(title),
                                )
                                .child(div().text_xs().text_color(colors.text_tertiary).child(url)),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_secondary)
                        .child(body),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(meta_chip("Surface", "Atlas"))
                        .child(meta_chip("Host", &host_chip))
                        .child(meta_chip("Bridge", &bridge_mode))
                        .child(meta_chip("Profile", &profile)),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_main)
                        .border_1()
                        .border_color(colors.border)
                        .px(px(14.0))
                        .py(px(12.0))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.accent)
                                .child("Bridge status"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.text_secondary)
                                .child(bridge_detail),
                        )
                        .child(div().text_xs().text_color(colors.text_tertiary).child(
                            "Gap: wry child embed needs X11 + GTK main iteration; GPUI \
                                     Blade/Wayland does not parent WebKit surfaces yet. \
                                     MITSURO_ATLAS_EXTERNAL=1 auto-opens system browser on Go.",
                        )),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_main)
                        .border_1()
                        .border_color(colors.border)
                        .px(px(14.0))
                        .py(px(12.0))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.accent)
                                .child("Agent browser-use"),
                        )
                        .child(div().text_sm().text_color(colors.text_secondary).child(
                            "When the agent opens a browser session, status flips to \
                                     Agent driving and page context (URL · title) is attached \
                                     to the turn. CDP / accessibility hooks track the bridge \
                                     target until an in-panel WebView is feasible.",
                        )),
                ),
        )
}

fn meta_chip(label: &str, value: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(999.0))
        .bg(colors.bg_button_secondary)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(format!("{label}:")),
        )
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_secondary)
                .child(value.to_string()),
        )
}

fn session_status_bar(session: &crate::app::BrowserSession) -> impl IntoElement {
    let colors = theme::colors();
    let engine = session
        .engine_version
        .as_ref()
        .map(|v| format!(" · WebKit {v}"))
        .unwrap_or_default();
    let (dot_color, label, detail) = match session.status {
        BrowserSessionStatus::Idle => (
            colors.status_offline,
            "Agent browser session",
            format!("Idle · {}{engine}", session.host_kind),
        ),
        BrowserSessionStatus::Connecting => (
            colors.status_connecting,
            "Agent browser session",
            "Connecting…".into(),
        ),
        BrowserSessionStatus::Ready => (
            colors.status_ready,
            "Agent browser session",
            format!("Ready · page context available{engine}"),
        ),
        BrowserSessionStatus::AgentDriving => (
            colors.accent,
            "Agent browser session",
            "Agent driving · tools navigating".into(),
        ),
        BrowserSessionStatus::Error => (
            colors.status_error,
            "Agent browser session",
            "Error · see status line".into(),
        ),
        BrowserSessionStatus::NoNativeHost => (
            colors.status_offline,
            "Agent browser session",
            "No native browser · build without browser-native (mock history still works)".into(),
        ),
    };
    let profile = session.profile_label.to_string();

    div()
        .id("browser-session-status")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(10.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(div().w(px(8.0)).h(px(8.0)).rounded(px(999.0)).bg(dot_color))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(detail),
                        ),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(format!("Profile: {profile}")),
        )
}

fn dot(color: gpui::Hsla) -> impl IntoElement {
    div().w(px(10.0)).h(px(10.0)).rounded(px(999.0)).bg(color)
}
