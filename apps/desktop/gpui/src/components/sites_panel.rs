//! Sites destination — empty publish CTA (bar parity) + optional site cards.
//!
//! No sites/* app-server methods. Default is product empty when live; Create densifies
//! with [`SAMPLE_SITES`] fixture cards (explicit "fixture demo" badge).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, UiConnection};
use crate::theme;

#[derive(Clone, Copy)]
struct SiteCard {
    name: &'static str,
    url: &'static str,
    status: &'static str,
    updated: &'static str,
}

const SAMPLE_SITES: &[SiteCard] = &[
    SiteCard {
        name: "mitsuro-docs",
        url: "docs.mitsuro.local",
        status: "Published",
        updated: "Yesterday",
    },
    SiteCard {
        name: "landing",
        url: "landing.local",
        status: "Draft",
        updated: "3d ago",
    },
];

/// Full-height Sites panel (sidebar nav destination).
pub fn sites_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let show_sites = app.sites_show_fixtures();
    let live = matches!(app.connection(), UiConnection::Ready { .. });

    div()
        .id("sites-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(header(show_sites, live, cx))
        .child(search_placeholder())
        .child(if show_sites {
            sites_grid(cx).into_any_element()
        } else {
            empty_state(live, cx).into_any_element()
        })
}

fn header(show_fixtures: bool, live: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let subtitle = if show_fixtures {
        "Fixture demo cards · no sites/* in app-server"
    } else if live {
        "No sites yet · product empty while connected (no sites protocol)"
    } else {
        "Turn your ideas into live websites"
    };
    div()
        .id("sites-header")
        .flex()
        .flex_row()
        .items_start()
        .justify_between()
        .px(px(28.0))
        .pt(px(28.0))
        .pb(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("Sites"),
                        )
                        .when(show_fixtures, |this| {
                            this.child(
                                div()
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded(px(999.0))
                                    .bg(theme::hex_alpha(0xf59e0b, 0.14))
                                    .border_1()
                                    .border_color(theme::hex_alpha(0xf59e0b, 0.35))
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme::hex(0xfbbf24))
                                    .child("Fixture demo"),
                            )
                        }),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .id("sites-refresh")
                        .w(px(32.0))
                        .h(px(32.0))
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.set_status_line("Sites · refreshed", cx);
                        }))
                        .child(
                            Icon::empty()
                                .path("icons/refresh-cw.svg")
                                .with_size(px(14.0))
                                .text_color(colors.text_secondary),
                        ),
                )
                .child(
                    div()
                        .id("sites-create")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .h(px(32.0))
                        .px(px(14.0))
                        .rounded(px(999.0))
                        .bg(colors.bg_button_primary)
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.bg_button_primary_hover))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.set_sites_show_fixtures(true, cx);
                            app.set_status_line("Sites · fixture demo cards", cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.fg_button_primary)
                                .child("Create"),
                        )
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .with_size(px(12.0))
                                .text_color(colors.fg_button_primary),
                        ),
                ),
        )
}

fn search_placeholder() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("sites-search")
        .mx(px(28.0))
        .mb(px(8.0))
        .h(px(36.0))
        .px(px(12.0))
        .rounded(px(10.0))
        .bg(theme::hex_alpha(0xffffff, 0.04))
        .border_1()
        .border_color(colors.border)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
            Icon::new(IconName::Search)
                .with_size(px(14.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("Search sites"),
        )
}

fn empty_state(live: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let caption = if live {
        "Connected · no sites/* methods on app-server yet"
    } else {
        "Create densifies local fixture cards only"
    };
    // Fill remaining column under header/search and center CTA both axes (bar empty).
    div()
        .id("sites-empty")
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .px(px(24.0))
        .pb(px(48.0))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(14.0))
                .child(
                    Icon::new(IconName::LayoutDashboard)
                        .with_size(px(28.0))
                        .text_color(colors.text_tertiary),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("No sites yet"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child(caption),
                )
                .child(
                    div()
                        .id("sites-create-empty")
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .h(px(34.0))
                        .px(px(16.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_button_secondary)
                        .border_1()
                        .border_color(colors.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.set_sites_show_fixtures(true, cx);
                            app.set_status_line("Sites · fixture demo cards", cx);
                        }))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .child("Create new site"),
                        ),
                ),
        )
}

fn sites_grid(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("sites-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(28.0))
        .pb(px(28.0))
        .pt(px(12.0))
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!(
                            "{} fixture site(s) · demo only",
                            SAMPLE_SITES.len()
                        )),
                )
                .child(
                    div()
                        .id("sites-hide-demo")
                        .cursor_pointer()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.set_sites_show_fixtures(false, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("Show empty"),
                        ),
                ),
        )
        .child(
            div().flex().flex_row().flex_wrap().gap(px(12.0)).children(
                SAMPLE_SITES
                    .iter()
                    .enumerate()
                    .map(|(i, site)| site_card(i as u64, site).into_any_element()),
            ),
        )
}

fn site_card(index: u64, site: &SiteCard) -> impl IntoElement {
    let colors = theme::colors();
    let published = site.status == "Published";
    div()
        .id(("site-card", index))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .w(px(260.0))
        .px(px(14.0))
        .py(px(14.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .w_full()
                .h(px(96.0))
                .rounded(px(8.0))
                .bg(theme::hex_alpha(0xffffff, 0.04))
                .border_1()
                .border_color(colors.border_subtle)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(IconName::LayoutDashboard)
                        .with_size(px(22.0))
                        .text_color(colors.text_tertiary),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(site.name),
                )
                .child(
                    div()
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(6.0))
                        .bg(if published {
                            theme::hex_alpha(0x04b84c, 0.14)
                        } else {
                            colors.bg_button_secondary
                        })
                        .text_xs()
                        .text_color(if published {
                            colors.status_ready
                        } else {
                            colors.text_secondary
                        })
                        .child(site.status),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(format!("{} · {}", site.url, site.updated)),
        )
}
