//! Home left sidebar (~260px) matching Codex/ChatGPT desktop bar density.
//!
//! Structure:
//! - Mode switcher pill (Chat / Codex) + search / bell icons
//! - Nav: New chat · Pull requests · Sites · Scheduled · Plugins
//! - Projects (empty)
//! - Pinned + Recents (scrollable live thread titles)
//! - Profile row → Settings

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, ProductMode};
use crate::demo::{self, DemoThread, ThreadSurface};
use crate::theme;

const SIDEBAR_WIDTH: f32 = 260.0;

pub fn sidebar(
    app: &MitsuroApp,
    search: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let selected = app.selected_thread_id().map(str::to_string);
    let filter = app.search_query().to_lowercase();
    let threads = app.visible_threads();
    let mode = app.active_mode();
    let chat_mode = matches!(mode, ProductMode::Chat);
    let menu_open = app.mode_menu_open();
    let surface = app.active_thread_surface();
    // Chat mode pill: "Chat"; Codex surface: "Codex".
    let switcher_label = mode.mode_switcher_label();
    let new_label = "New chat";
    let profile_name = app.profile_display_name().to_string();
    let profile_name_visible = app.profile_name_visible_in_sidebar();
    let profile_plan = app.profile_plan_label().map(|p| p.to_string());
    let _ = search; // search field still wired via InputState; icon opens focus

    let (mut pinned_threads, recent_threads): (Vec<_>, Vec<_>) = threads
        .into_iter()
        .partition(|thread| thread.summary.is_pinned.unwrap_or(false));
    pinned_threads.sort_by_key(|thread| {
        app.pinned_thread_rank(&thread.summary.id)
            .unwrap_or(usize::MAX)
    });
    let mut thread_items: Vec<AnyElement> = Vec::new();
    if !pinned_threads.is_empty() {
        thread_items.push(thread_section_heading("Pinned", false));
        thread_items.extend(pinned_threads.into_iter().map(|thread| {
            let is_selected = selected.as_deref() == Some(thread.summary.id.as_str());
            thread_row(thread, is_selected, cx)
        }));
    }
    if !recent_threads.is_empty() {
        thread_items.push(thread_section_heading("Recents", !thread_items.is_empty()));
        thread_items.extend(recent_threads.into_iter().map(|thread| {
            let is_selected = selected.as_deref() == Some(thread.summary.id.as_str());
            thread_row(thread, is_selected, cx)
        }));
    }
    if thread_items.is_empty() {
        thread_items.push(
            div()
                .px(px(10.0))
                .py(px(10.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(if filter.is_empty() {
                    "No recents yet.".to_string()
                } else {
                    "No matches.".to_string()
                })
                .into_any_element(),
        );
    }

    div()
        .id("thread-sidebar")
        .relative()
        .flex()
        .flex_col()
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .bg(colors.bg_sidebar)
        .border_r_1()
        .border_color(colors.border_subtle)
        // ── Header: mode pill + search / secondary action ─────────────
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(12.0))
                .pt(px(10.0))
                .pb(px(6.0))
                .gap(px(6.0))
                .child(mode_switcher_pill(switcher_label, menu_open, cx))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2.0))
                        .flex_shrink_0()
                        .child(header_icon_btn(
                            "sidebar-search",
                            IconName::Search,
                            cx,
                            |app, _, _, cx| {
                                app.set_status_line("Search recents · type to filter", cx);
                            },
                        ))
                        // Bar: trailing control is notification bell (New chat is a nav row).
                        .child(header_icon_btn(
                            "sidebar-bell",
                            IconName::Bell,
                            cx,
                            |app, _, _, cx| {
                                app.set_status_line("Notifications · no new alerts", cx);
                            },
                        )),
                ),
        )
        // Mode dropdown overlay (Chat / Codex)
        .when(menu_open, |this| this.child(mode_dropdown(surface, cx)))
        // ── Primary nav ───────────────────────────────────────────────
        // Chat mode: New chat · Projects(+) · Sites · Scheduled · Plugins (hide PRs).
        // Codex mode: New chat · Pull requests · Sites · Scheduled · Plugins.
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(0.0))
                .px(px(8.0))
                .pb(px(6.0))
                .child(nav_item(
                    "nav-new-chat",
                    IconName::SquareTerminal,
                    new_label,
                    false,
                    Some("icons/square-pen.svg"),
                    cx,
                    |app, window, cx| {
                        app.close_mode_menu(cx);
                        app.go_home(window, cx);
                    },
                ))
                .when(chat_mode, |this| {
                    this.child(nav_item_with_trailing(
                        "nav-projects",
                        IconName::FolderOpen,
                        "Projects",
                        false,
                        Some("icons/folder.svg"),
                        cx,
                        |app, _, _, cx| {
                            app.close_mode_menu(cx);
                            app.set_status_line(
                                "Projects are not exposed by the selected backend.",
                                cx,
                            );
                        },
                        |app, _, _, cx| {
                            app.set_status_line(
                                "Project creation is unavailable for the selected backend.",
                                cx,
                            );
                        },
                    ))
                })
                .when(!chat_mode, |this| {
                    this.child(nav_item(
                        "nav-prs",
                        IconName::GitHub,
                        "Pull requests",
                        mode == ProductMode::PullRequests,
                        None,
                        cx,
                        |app, window, cx| {
                            app.close_mode_menu(cx);
                            app.set_mode(ProductMode::PullRequests, window, cx);
                        },
                    ))
                })
                .child(nav_item(
                    "nav-sites",
                    IconName::LayoutDashboard,
                    "Sites",
                    mode == ProductMode::Sites,
                    Some("icons/layout-dashboard.svg"),
                    cx,
                    |app, window, cx| {
                        app.close_mode_menu(cx);
                        app.set_mode(ProductMode::Sites, window, cx);
                    },
                ))
                .child(nav_item(
                    "nav-scheduled",
                    IconName::Calendar,
                    "Scheduled",
                    mode == ProductMode::Scheduled,
                    Some("icons/clock.svg"),
                    cx,
                    |app, window, cx| {
                        app.close_mode_menu(cx);
                        app.set_mode(ProductMode::Scheduled, window, cx);
                    },
                ))
                .child(nav_item(
                    "nav-plugins",
                    IconName::Asterisk,
                    "Plugins",
                    mode == ProductMode::Extensions,
                    Some("icons/at-sign.svg"),
                    cx,
                    |app, window, cx| {
                        app.close_mode_menu(cx);
                        app.set_mode(ProductMode::Extensions, window, cx);
                    },
                )),
        )
        // ── Projects section (Codex mode only — Chat surfaces Projects in nav) ─
        .when(!chat_mode, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(14.0))
                    .pt(px(10.0))
                    .pb(px(6.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(colors.text_tertiary)
                                    .child("Projects"),
                            )
                            .child(
                                div()
                                    .id("projects-add")
                                    .w(px(22.0))
                                    .h(px(22.0))
                                    .rounded(px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .with_size(px(12.0))
                                            .text_color(colors.text_tertiary),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.text_tertiary)
                            .child("Unavailable from backend"),
                    ),
            )
        })
        // ── Pinned + Recents (dense list · native host pin order) ─────
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .pt(px(6.0))
                .child(
                    div()
                        .id("thread-list")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .px(px(6.0))
                        .gap(px(0.0))
                        .overflow_y_scroll()
                        .children(thread_items),
                ),
        )
        // Hidden search field (filter still works via InputState when focused programmatically)
        .child(
            div()
                .h(px(0.0))
                .overflow_hidden()
                .opacity(0.0)
                .child(Input::new(search).appearance(false)),
        )
        // ── Profile footer (bar: avatar · name · plan · circular ? → Settings) ──
        .child(profile_footer(
            &profile_name,
            profile_plan.as_deref(),
            profile_name_visible,
            cx,
        ))
}

fn thread_section_heading(label: &'static str, separated: bool) -> AnyElement {
    let colors = theme::colors();
    div()
        .px(px(8.0))
        .pt(if separated { px(10.0) } else { px(0.0) })
        .pb(px(4.0))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text_tertiary)
        .child(label)
        .into_any_element()
}

fn thread_row(thread: DemoThread, is_selected: bool, cx: &mut Context<MitsuroApp>) -> AnyElement {
    let colors = theme::colors();
    let id = thread.summary.id.clone();
    let open_id = id.clone();
    let pin_id = id.clone();
    let title = thread.summary.display_title();
    let show_branch = thread.summary.cwd.is_some();
    let is_pinned = thread.summary.is_pinned.unwrap_or(false);
    let can_pin = thread.backend_session_id.is_some();
    let group_name = SharedString::from(format!("thread-row-group-{id}"));

    div()
        .id(SharedString::from(format!("thread-row-{id}")))
        .group(group_name.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(8.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(if is_selected {
            theme::hex_alpha(0xffffff, 0.06)
        } else {
            theme::transparent()
        })
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| {
            app.close_mode_menu(cx);
            let surface = app
                .threads()
                .iter()
                .find(|candidate| candidate.summary.id == open_id)
                .map(|candidate| candidate.surface)
                .unwrap_or(ThreadSurface::Codex);
            let mode = match surface {
                ThreadSurface::Chat => ProductMode::Chat,
                ThreadSurface::Codex => ProductMode::Codex,
            };
            if app.active_mode() != mode {
                app.set_mode(mode, window, cx);
            }
            app.select_thread_with_window(open_id.clone(), window, cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(if is_selected {
                    colors.text
                } else {
                    colors.text_secondary
                })
                .whitespace_nowrap()
                .overflow_hidden()
                .child(title),
        )
        .when(show_branch, |this| {
            this.child(
                Icon::empty()
                    .path("icons/git-branch.svg")
                    .with_size(px(12.0))
                    .text_color(theme::hex_alpha(0xa78bfa, 0.95)),
            )
        })
        .when(can_pin, |this| {
            this.child(
                div()
                    .id(SharedString::from(format!("thread-pin-{pin_id}")))
                    .w(px(22.0))
                    .h(px(22.0))
                    .flex_shrink_0()
                    .rounded(px(6.0))
                    .opacity(if is_pinned { 1.0 } else { 0.0 })
                    .group_hover(group_name, |style| style.opacity(1.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|style| style.bg(theme::hex_alpha(0xffffff, 0.07)))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        cx.stop_propagation();
                        app.set_thread_pinned(pin_id.clone(), !is_pinned, cx);
                    }))
                    .child(
                        Icon::empty()
                            .path("icons/pin.svg")
                            .with_size(px(13.0))
                            .text_color(if is_pinned {
                                colors.text_secondary
                            } else {
                                colors.text_tertiary
                            }),
                    ),
            )
        })
        .into_any_element()
}

/// Dense account row: initials avatar · name + plan · circular help `?`.
/// Whole row opens Settings (bar help opens help; ours maps to Settings for demos).
fn profile_footer(
    profile_name: &str,
    plan_label: Option<&str>,
    show_name: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let plan = plan_label.map(str::to_string);
    div()
        .id("sidebar-profile")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(8.0))
        .border_t_1()
        .border_color(colors.border_subtle)
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, window, cx| {
            app.close_mode_menu(cx);
            app.set_mode(ProductMode::Settings, window, cx);
        }))
        .child(
            div()
                .w(px(26.0))
                .h(px(26.0))
                .rounded_full()
                .border_1()
                .border_color(theme::hex_alpha(0xffe8d8, 0.16))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .text_color(colors.text_secondary)
                .child(crate::app::profile_initials_from_name(profile_name)),
        )
        .when(show_name, |this| {
            this.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(colors.text)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(profile_name.to_string()),
                    )
                    .when_some(plan, |this, plan| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(plan),
                        )
                    }),
            )
        })
        .when(!show_name, |this| this.child(div().flex_1()))
        .child(
            // Bar silhouette: circular outlined help `?` (not chevron square).
            div()
                .id("sidebar-profile-help")
                .w(px(22.0))
                .h(px(22.0))
                .rounded_full()
                .border_1()
                .border_color(theme::hex_alpha(0xffffff, 0.14))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_tertiary)
                        .child("?"),
                ),
        )
}

fn mode_switcher_pill(
    label: &'static str,
    open: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("mode-switcher")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(999.0))
        .bg(if open {
            colors.bg_elevated
        } else {
            theme::hex_alpha(0xffffff, 0.06)
        })
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| app.toggle_mode_menu(cx)))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
        .child(
            Icon::new(if open {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            })
            .with_size(px(13.0))
            .text_color(colors.text_tertiary),
        )
}

fn mode_dropdown(surface: ThreadSurface, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("mode-dropdown")
        .absolute()
        .top(px(48.0))
        .left(px(10.0))
        .w(px(220.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        // elevate above list
        .child(mode_option(
            "mode-opt-chat",
            "Chat",
            "Create, learn, and explore",
            surface == ThreadSurface::Chat,
            cx,
            |app, window, cx| {
                app.switch_thread_surface(ThreadSurface::Chat, window, cx);
            },
        ))
        .child(mode_option(
            "mode-opt-codex",
            "Codex",
            "Build, debug, and ship",
            surface == ThreadSurface::Codex,
            cx,
            |app, window, cx| {
                app.switch_thread_surface(ThreadSurface::Codex, window, cx);
            },
        ))
}

fn mode_option(
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .when(selected, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .with_size(px(14.0))
                    .text_color(colors.text_secondary),
            )
        })
}

fn header_icon_btn(
    id: &'static str,
    icon: IconName,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(on_click))
        .child(
            Icon::new(icon)
                .with_size(px(15.0))
                .text_color(colors.text_secondary),
        )
}

fn nav_item(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    selected: bool,
    icon_path: Option<&'static str>,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    let icon_el = match icon_path {
        Some(path) => Icon::empty()
            .path(path)
            .with_size(px(15.0))
            .text_color(if selected {
                colors.text
            } else {
                colors.text_tertiary
            }),
        None => Icon::new(icon).with_size(px(15.0)).text_color(if selected {
            colors.text
        } else {
            colors.text_tertiary
        }),
    };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(8.0))
        .py(px(6.0))
        // Bar density: soft rounded row, not a fat full-width pill.
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if selected {
            theme::hex_alpha(0xffffff, 0.06)
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
        .child(icon_el)
        .child(
            div()
                .text_sm()
                .text_color(if selected {
                    colors.text
                } else {
                    colors.text_secondary
                })
                .child(label),
        )
}

/// Nav row with a trailing + affordance (Chat mode Projects).
fn nav_item_with_trailing(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    selected: bool,
    icon_path: Option<&'static str>,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
    on_trailing: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    let icon_el = match icon_path {
        Some(path) => Icon::empty()
            .path(path)
            .with_size(px(15.0))
            .text_color(if selected {
                colors.text
            } else {
                colors.text_tertiary
            }),
        None => Icon::new(icon).with_size(px(15.0)).text_color(if selected {
            colors.text
        } else {
            colors.text_tertiary
        }),
    };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if selected {
            theme::hex_alpha(0xffffff, 0.06)
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(on_click))
        .child(icon_el)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(if selected {
                    colors.text
                } else {
                    colors.text_secondary
                })
                .child(label),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-plus")))
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(theme::hex_alpha(0xffffff, 0.08)))
                .on_click(cx.listener(on_trailing))
                .child(
                    Icon::new(IconName::Plus)
                        .with_size(px(12.0))
                        .text_color(colors.text_tertiary),
                ),
        )
}

// Silence unused import warning if demo helpers unused later
#[allow(dead_code)]
fn _meta(summary: &mitsuro_desktop_backend::ThreadSummary) -> String {
    demo::meta_line(summary)
}
