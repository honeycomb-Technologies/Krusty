//! Home left sidebar matching the current reference desktop density.
//!
//! Structure:
//! - Mode switcher pill (Chat / Codex) + search / bell icons
//! - Nav: New chat · Pull requests · Sites · Scheduled · Plugins
//! - Native-host Projects (real workspace roots; live thread membership)
//! - Pinned/Recents or Priority/day-grouped activity from real thread state
//! - Profile row → Settings

use std::collections::BTreeMap;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    deferred, div, point, px, AnyElement, BoxShadow, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::{Input, InputState};
use gpui_component::spinner::Spinner;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Sizable as _};
use mitsuro_desktop_backend::BackendKind;

use crate::app::{
    ActionAvailability, MitsuroApp, ProductMode, ThreadAction, UiConnectionState,
    UiConnectionSummary, UiConnectionThreadPreview, UiThreadActivity,
};
use crate::demo::{self, DemoThread, ThreadSurface};
use crate::theme;

pub fn sidebar(
    app: &MitsuroApp,
    search: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let connections_expanded = app.sidebar_group_expanded("connections");
    let projects_expanded = app.sidebar_group_expanded("projects");
    let selected = app.selected_thread_id().map(str::to_string);
    let filter = app.search_query().to_lowercase();
    let threads = app.visible_threads();
    let connection_summaries = app.connection_summaries();
    let mut connection_items = Vec::new();
    for connection in connection_summaries {
        let previews = app.inactive_connection_thread_previews(&connection.connection_id, 3);
        connection_items.push(connection_row(connection, cx));
        connection_items.extend(
            previews
                .into_iter()
                .map(|preview| inactive_connection_thread_row(preview, cx)),
        );
    }
    let mode = app.active_mode();
    let chat_mode = matches!(mode, ProductMode::Chat);
    let sidebar_width = if chat_mode {
        theme::metrics().chat_sidebar_width
    } else {
        theme::metrics().sidebar_width
    };
    let menu_open = app.mode_menu_open();
    let search_open = app.sidebar_search_open();
    let activity_view = app.sidebar_activity_view();
    let has_priority_activity = app.sidebar_has_priority_activity();
    let surface = app.active_thread_surface();
    // Product switcher mirrors the reference's ChatGPT / Codex label.
    let switcher_label = mode.mode_switcher_label();
    let new_label = "New chat";
    let profile_name = app.profile_display_name().to_string();
    let profile_name_visible = app.profile_name_visible_in_sidebar();
    let profile_plan = app.profile_plan_label().map(|p| p.to_string());
    let projects_available = app.can_manage_local_projects();
    let selected_project = app.selected_project_id().map(str::to_owned);
    let project_items = app
        .local_projects()
        .iter()
        .cloned()
        .map(|project| {
            let is_selected = selected_project.as_deref() == Some(project.id.as_str());
            let remove_armed = app.project_remove_armed(&project.id);
            project_row(project, is_selected, remove_armed, projects_available, cx)
        })
        .collect::<Vec<_>>();
    let mut thread_items = if activity_view {
        activity_thread_items(app, threads, selected.as_deref(), cx)
    } else {
        standard_thread_items(app, threads, selected.as_deref(), cx)
    };
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
        .w(px(sidebar_width))
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
                            search_open,
                            false,
                            cx,
                            |app, _, window, cx| {
                                app.toggle_sidebar_search(window, cx);
                            },
                        ))
                        // Bar: trailing control is notification bell (New chat is a nav row).
                        .child(header_icon_btn(
                            "sidebar-bell",
                            IconName::Bell,
                            activity_view,
                            has_priority_activity,
                            cx,
                            |app, _, _, cx| {
                                app.toggle_sidebar_activity_view(cx);
                            },
                        )),
                ),
        )
        .when(search_open, |this| {
            this.child(
                div()
                    .id("sidebar-search-field")
                    .mx(px(8.0))
                    .mb(px(6.0))
                    .h(px(34.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .rounded(px(8.0))
                    .bg(colors.bg_elevated)
                    .border_1()
                    .border_color(colors.border)
                    .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            app.close_sidebar_search(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        Icon::new(IconName::Search)
                            .with_size(px(13.0))
                            .text_color(colors.text_tertiary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(search).appearance(false).h(px(28.0))),
                    )
                    .child(
                        div()
                            .id("sidebar-search-close")
                            .w(px(22.0))
                            .h(px(22.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|style| style.bg(colors.bg_hover))
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.close_sidebar_search(window, cx);
                            }))
                            .child(
                                Icon::new(IconName::Close)
                                    .with_size(px(12.0))
                                    .text_color(colors.text_tertiary),
                            ),
                    ),
            )
        })
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
        // Both real providers and a bounded inactive-session preview remain
        // navigable while the selected provider owns the full Recents list.
        .child(
            div()
                .flex()
                .flex_col()
                .px(px(14.0))
                .pt(px(8.0))
                .pb(px(4.0))
                .gap(px(2.0))
                .child(
                    div()
                        .id("connections-disclosure")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .px(px(2.0))
                        .pb(px(4.0))
                        .tab_index(0)
                        .cursor_pointer()
                        .focus(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.toggle_sidebar_group("connections", cx);
                        }))
                        .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                app.toggle_sidebar_group("connections", cx);
                                cx.stop_propagation();
                            }
                        }))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_tertiary)
                        .child(if connections_expanded { "▾" } else { "▸" })
                        .child("Connections"),
                )
                .when(connections_expanded, |section| {
                    section.children(connection_items)
                }),
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
                                    .id("projects-disclosure")
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(5.0))
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .focus(|style| style.bg(colors.bg_hover))
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.toggle_sidebar_group("projects", cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        |app, event: &gpui::KeyDownEvent, _, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                app.toggle_sidebar_group("projects", cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(colors.text_tertiary)
                                    .child(if projects_expanded { "▾" } else { "▸" })
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
                                    .opacity(if projects_available { 1.0 } else { 0.4 })
                                    .when(projects_available, |this| {
                                        this.cursor_pointer()
                                            .hover(|style| {
                                                style.bg(theme::hex_alpha(0xffffff, 0.08))
                                            })
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.create_local_project(cx);
                                            }))
                                    })
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .with_size(px(12.0))
                                            .text_color(colors.text_tertiary),
                                    ),
                            ),
                    )
                    .when(projects_expanded, |this| {
                        this.when(project_items.is_empty(), |this| {
                            this.child(
                                div()
                                    .px(px(8.0))
                                    .py(px(4.0))
                                    .text_xs()
                                    .text_color(colors.text_tertiary)
                                    .child(if projects_available {
                                        "No projects yet."
                                    } else {
                                        "Projects unavailable for this backend."
                                    }),
                            )
                        })
                        .children(project_items)
                    }),
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
        // ── Profile footer (bar: avatar · name · plan · circular ? → Settings) ──
        .child(profile_footer(
            &profile_name,
            profile_plan.as_deref(),
            profile_name_visible,
            cx,
        ))
        // Paint the mode menu after all sidebar content so later nav rows
        // cannot bleed through an absolute overlay.
        .when(menu_open, |this| {
            this.child(
                deferred(
                    div()
                        .id("mode-dropdown-backdrop")
                        .occlude()
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.close_mode_menu(cx);
                            cx.stop_propagation();
                        }))
                        .child(mode_dropdown(surface, cx)),
                )
                .with_priority(50),
            )
        })
}

fn connection_row(connection: UiConnectionSummary, cx: &mut Context<MitsuroApp>) -> AnyElement {
    let colors = theme::colors();
    let kind = connection.kind;
    let connection_id = connection.connection_id.clone();
    let keyboard_connection_id = connection_id.clone();
    let buffered_updates = connection.buffered_updates;
    let running_sessions = connection.running_sessions;
    let needs_attention = connection.needs_attention;
    let (status_label, status_color, session_count) = match &connection.state {
        UiConnectionState::Connecting => ("Connecting", colors.status_connecting, None),
        UiConnectionState::Online { session_count } => {
            ("Online", colors.status_ready, Some(*session_count))
        }
        UiConnectionState::Degraded { session_count, .. } => {
            ("Degraded", colors.status_connecting, Some(*session_count))
        }
        UiConnectionState::Offline { .. } => ("Offline", colors.status_offline, None),
        UiConnectionState::Reconnecting => ("Reconnecting", colors.status_connecting, None),
        UiConnectionState::Unsupported { .. } => ("Unsupported", colors.status_error, None),
    };
    let state_reason = match &connection.state {
        UiConnectionState::Degraded { reason, .. } | UiConnectionState::Unsupported { reason } => {
            Some(reason.as_str())
        }
        UiConnectionState::Offline {
            reason: Some(reason),
        } => Some(reason.as_str()),
        _ => None,
    };
    let mut tooltip_lines = Vec::new();
    if let Some(provenance) = &connection.provenance {
        tooltip_lines.push(provenance.clone());
    }
    if let Some(reason) = state_reason {
        tooltip_lines.push(reason.to_owned());
    }
    if let Some(error) = &connection.last_error {
        if state_reason != Some(error.as_str()) {
            tooltip_lines.push(format!("Last error: {error}"));
        }
    }
    tooltip_lines.insert(
        0,
        connection_status_detail(
            status_label,
            session_count,
            running_sessions,
            buffered_updates,
            needs_attention,
        ),
    );
    let tooltip = tooltip_lines.join("\n");
    let icon = if kind == BackendKind::MitsuroHttp {
        IconName::Building2
    } else {
        IconName::SquareTerminal
    };

    div()
        .id(SharedString::from(format!(
            "connection-row-{connection_id}"
        )))
        .flex()
        .flex_row()
        .items_center()
        .h(px(32.0))
        .gap(px(8.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .bg(theme::transparent())
        .tab_index(0)
        .cursor_pointer()
        .focus(|style| style.bg(colors.bg_hover))
        .hover(|style| style.bg(colors.bg_hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.switch_connection(connection_id.clone(), cx);
        }))
        .on_key_down(cx.listener(move |app, event: &gpui::KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                app.switch_connection(keyboard_connection_id.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .child(
            Icon::new(icon)
                .with_size(px(15.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(if connection.selected {
                    gpui::FontWeight::MEDIUM
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if connection.selected {
                    colors.text
                } else {
                    colors.text_secondary
                })
                .whitespace_nowrap()
                .overflow_hidden()
                .child(connection.label),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .whitespace_nowrap()
                .child(status_label),
        )
        .child(
            div()
                .size(px(7.0))
                .rounded_full()
                .bg(status_color)
                .flex_shrink_0(),
        )
        .into_any_element()
}

fn connection_status_detail(
    status: &str,
    session_count: Option<usize>,
    running_sessions: usize,
    buffered_updates: usize,
    needs_attention: usize,
) -> String {
    let mut parts = vec![status.to_owned()];
    if let Some(count) = session_count {
        parts.push(format!("{count} sessions"));
    }
    if needs_attention > 0 {
        parts.push(format!("{needs_attention} need attention"));
    } else {
        if running_sessions > 0 {
            parts.push(format!("{running_sessions} running"));
        }
        if buffered_updates > 0 {
            parts.push(format!("{buffered_updates} updates"));
        }
    }
    parts.join(" · ")
}

fn standard_thread_items(
    app: &MitsuroApp,
    threads: Vec<DemoThread>,
    selected: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> Vec<AnyElement> {
    let (mut pinned_threads, recent_threads): (Vec<_>, Vec<_>) = threads
        .into_iter()
        .partition(|thread| thread.summary.is_pinned.unwrap_or(false));
    pinned_threads.sort_by_key(|thread| {
        app.pinned_thread_rank(&thread.summary.id)
            .unwrap_or(usize::MAX)
    });
    let mut items = Vec::new();
    if !pinned_threads.is_empty() {
        let expanded = app.sidebar_group_expanded("pinned");
        items.push(collapsible_thread_section_heading(
            "Pinned", false, "pinned", expanded, cx,
        ));
        if expanded {
            for thread in pinned_threads {
                let is_selected = selected == Some(thread.summary.id.as_str());
                let activity = app.thread_activity(&thread.summary.id);
                items.push(thread_row(app, thread, is_selected, activity, cx));
            }
        }
    }
    if !recent_threads.is_empty() {
        let expanded = app.sidebar_group_expanded("recents");
        items.push(collapsible_thread_section_heading(
            "Recents",
            !items.is_empty(),
            "recents",
            expanded,
            cx,
        ));
        if expanded {
            for thread in recent_threads {
                let is_selected = selected == Some(thread.summary.id.as_str());
                let activity = app.thread_activity(&thread.summary.id);
                items.push(thread_row(app, thread, is_selected, activity, cx));
            }
        }
    }
    items
}

fn activity_thread_items(
    app: &MitsuroApp,
    threads: Vec<DemoThread>,
    selected: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> Vec<AnyElement> {
    let colors = theme::colors();
    let mut priority = Vec::new();
    let mut pinned = Vec::new();
    let mut recent = Vec::new();
    for thread in threads {
        if app.thread_has_priority_activity(&thread.summary.id) {
            priority.push(thread);
        } else if thread.summary.is_pinned.unwrap_or(false) {
            pinned.push(thread);
        } else {
            recent.push(thread);
        }
    }
    pinned.sort_by_key(|thread| {
        app.pinned_thread_rank(&thread.summary.id)
            .unwrap_or(usize::MAX)
    });

    let priority_expanded = app.sidebar_group_expanded("priority");
    let mut items = vec![collapsible_thread_section_heading(
        "Priority",
        false,
        "priority",
        priority_expanded,
        cx,
    )];
    if priority_expanded && priority.is_empty() {
        items.push(
            div()
                .px(px(10.0))
                .py(px(7.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("Nothing needs attention")
                .into_any_element(),
        );
    } else if priority_expanded {
        for thread in priority {
            let is_selected = selected == Some(thread.summary.id.as_str());
            let activity = app.thread_activity(&thread.summary.id);
            items.push(thread_row(app, thread, is_selected, activity, cx));
        }
    }

    if !pinned.is_empty() {
        let expanded = app.sidebar_group_expanded("pinned");
        items.push(collapsible_thread_section_heading(
            "Pinned", true, "pinned", expanded, cx,
        ));
        if expanded {
            for thread in pinned {
                let is_selected = selected == Some(thread.summary.id.as_str());
                let activity = app.thread_activity(&thread.summary.id);
                items.push(thread_row(app, thread, is_selected, activity, cx));
            }
        }
    }

    let today = Local::now().date_naive();
    let mut by_day: BTreeMap<NaiveDate, Vec<DemoThread>> = BTreeMap::new();
    let mut unknown_time = Vec::new();
    for thread in recent {
        match thread
            .summary
            .updated_at
            .and_then(local_date_from_timestamp)
        {
            Some(day) => by_day.entry(day).or_default().push(thread),
            None => unknown_time.push(thread),
        }
    }
    if !by_day.is_empty() || !unknown_time.is_empty() {
        let expanded = app.sidebar_group_expanded("recents");
        items.push(collapsible_thread_section_heading(
            "Recents", true, "recents", expanded, cx,
        ));
        if expanded {
            for (day, threads) in by_day.into_iter().rev() {
                items.push(thread_section_heading(
                    activity_day_heading(day, today),
                    true,
                ));
                for thread in threads {
                    let is_selected = selected == Some(thread.summary.id.as_str());
                    let activity = app.thread_activity(&thread.summary.id);
                    items.push(thread_row(app, thread, is_selected, activity, cx));
                }
            }
            if !unknown_time.is_empty() {
                items.push(thread_section_heading("Earlier", true));
                for thread in unknown_time {
                    let is_selected = selected == Some(thread.summary.id.as_str());
                    let activity = app.thread_activity(&thread.summary.id);
                    items.push(thread_row(app, thread, is_selected, activity, cx));
                }
            }
        }
    }
    items
}

fn local_date_from_timestamp(value: i64) -> Option<NaiveDate> {
    let seconds = if value.unsigned_abs() >= 1_000_000_000_000 {
        value / 1_000
    } else {
        value
    };
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|timestamp| timestamp.with_timezone(&Local).date_naive())
}

fn activity_day_heading(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        "Today".to_owned()
    } else if day.succ_opt() == Some(today) {
        "Yesterday".to_owned()
    } else {
        day.format("%A").to_string()
    }
}

fn thread_section_heading(label: impl Into<SharedString>, separated: bool) -> AnyElement {
    let colors = theme::colors();
    let label = label.into();
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

fn collapsible_thread_section_heading(
    label: &'static str,
    separated: bool,
    group: &'static str,
    expanded: bool,
    cx: &mut Context<MitsuroApp>,
) -> AnyElement {
    let colors = theme::colors();
    div()
        .id(SharedString::from(format!("sidebar-group-{group}")))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .px(px(8.0))
        .pt(if separated { px(10.0) } else { px(0.0) })
        .pb(px(4.0))
        .tab_index(0)
        .cursor_pointer()
        .focus(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.toggle_sidebar_group(group, cx);
        }))
        .on_key_down(cx.listener(move |app, event: &gpui::KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                app.toggle_sidebar_group(group, cx);
                cx.stop_propagation();
            }
        }))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text_tertiary)
        .child(if expanded { "▾" } else { "▸" })
        .child(label)
        .into_any_element()
}

fn thread_activity_indicator(activity: UiThreadActivity) -> AnyElement {
    let colors = theme::colors();
    if activity == UiThreadActivity::Running {
        return div()
            .w(px(14.0))
            .h(px(14.0))
            .flex_shrink_0()
            .child(
                Spinner::new()
                    .with_size(px(12.0))
                    .color(colors.text_tertiary),
            )
            .into_any_element();
    }

    let (label, color) = match activity {
        UiThreadActivity::ApprovalNeeded => ("Approval", colors.accent_orange),
        UiThreadActivity::InputNeeded => ("Input", colors.accent),
        UiThreadActivity::Completed => ("Done", colors.status_ready),
        UiThreadActivity::Failed => ("Failed", colors.status_error),
        UiThreadActivity::Running => unreachable!(),
    };
    div()
        .h(px(18.0))
        .px(px(5.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .rounded(px(5.0))
        .bg(theme::hex_alpha(0xffffff, 0.04))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(color)
        .child(label)
        .into_any_element()
}

fn inactive_connection_thread_row(
    preview: UiConnectionThreadPreview,
    cx: &mut Context<MitsuroApp>,
) -> AnyElement {
    let colors = theme::colors();
    let connection_id = preview.connection_id;
    let provider_session_id = preview.provider_session_id;
    let keyboard_connection_id = connection_id.clone();
    let keyboard_session_id = provider_session_id.clone();
    let title = preview.title;
    let activity = preview.activity;
    div()
        .id(SharedString::from(format!(
            "inactive-thread-{}-{}",
            connection_id, provider_session_id
        )))
        .ml(px(22.0))
        .h(px(32.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .tab_index(0)
        .cursor_pointer()
        .focus(|style| style.bg(colors.bg_hover))
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| {
            app.open_connection_thread(
                connection_id.clone(),
                provider_session_id.clone(),
                window,
                cx,
            );
        }))
        .on_key_down(
            cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    app.open_connection_thread(
                        keyboard_connection_id.clone(),
                        keyboard_session_id.clone(),
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }
            }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(colors.text_secondary)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(title),
        )
        .when_some(activity, |row, activity| {
            row.child(thread_activity_indicator(activity))
        })
        .into_any_element()
}

fn project_row(
    project: crate::preferences::DesktopProject,
    is_selected: bool,
    remove_armed: bool,
    enabled: bool,
    cx: &mut Context<MitsuroApp>,
) -> AnyElement {
    let colors = theme::colors();
    let id = project.id.clone();
    let open_id = id.clone();
    let keyboard_open_id = id.clone();
    let remove_id = id.clone();
    let group_name = SharedString::from(format!("project-row-group-{id}"));

    div()
        .id(SharedString::from(format!("project-row-{id}")))
        .group(group_name.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(8.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .opacity(if enabled { 1.0 } else { 0.55 })
        .when(enabled, |this| {
            this.tab_index(0)
                .cursor_pointer()
                .focus(|style| style.bg(colors.bg_hover))
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.close_mode_menu(cx);
                    app.select_local_project(open_id.clone(), window, cx);
                }))
                .on_key_down(
                    cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            app.close_mode_menu(cx);
                            app.select_local_project(keyboard_open_id.clone(), window, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
        })
        .bg(if is_selected {
            theme::hex_alpha(0xffffff, 0.06)
        } else {
            theme::transparent()
        })
        .child(
            Icon::empty()
                .path("icons/folder.svg")
                .with_size(px(14.0))
                .text_color(if is_selected {
                    colors.text_secondary
                } else {
                    colors.text_tertiary
                }),
        )
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
                .child(project.name),
        )
        .when(enabled, |this| {
            this.child(
                div()
                    .id(SharedString::from(format!("project-remove-{remove_id}")))
                    .w(px(22.0))
                    .h(px(22.0))
                    .flex_shrink_0()
                    .rounded(px(6.0))
                    .opacity(if remove_armed { 1.0 } else { 0.0 })
                    .group_hover(group_name, |style| style.opacity(1.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|style| style.bg(theme::hex_alpha(0xffffff, 0.07)))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        cx.stop_propagation();
                        app.request_remove_local_project(remove_id.clone(), cx);
                    }))
                    .child(
                        Icon::empty()
                            .path("icons/delete.svg")
                            .with_size(px(13.0))
                            .text_color(if remove_armed {
                                theme::hex_alpha(0xef4444, 0.95)
                            } else {
                                colors.text_tertiary
                            }),
                    ),
            )
        })
        .into_any_element()
}

fn thread_row(
    app: &MitsuroApp,
    thread: DemoThread,
    is_selected: bool,
    activity: Option<UiThreadActivity>,
    cx: &mut Context<MitsuroApp>,
) -> AnyElement {
    let colors = theme::colors();
    let id = thread.summary.id.clone();
    let open_id = id.clone();
    let keyboard_open_id = id.clone();
    let menu_id = id.clone();
    let keyboard_menu_id = id.clone();
    let title = thread.summary.display_title();
    let project_name = app
        .local_project_for_thread(&thread)
        .map(|project| project.name.clone());
    let context_label = project_name.clone().unwrap_or_else(|| {
        thread
            .backend_session_id
            .as_ref()
            .map(|session| match session.backend {
                BackendKind::MitsuroHttp => "Mitsuro",
                BackendKind::CodexStdio | BackendKind::CodexWebSocket => "Codex",
                BackendKind::Fixture => "Fixture",
            })
            .unwrap_or(match thread.surface {
                ThreadSurface::Chat => "Chat",
                ThreadSurface::Codex => "Draft",
            })
            .to_owned()
    });
    let context_icon = if project_name.is_some() {
        "icons/folder.svg"
    } else {
        "icons/square-terminal.svg"
    };
    let is_pinned = thread.summary.is_pinned.unwrap_or(false);
    let is_archived = thread.summary.archived.unwrap_or(false);
    let can_pin = thread.backend_session_id.is_some();
    let menu_open = app.sidebar_thread_menu_open(&id);
    let rename_availability = app.thread_action_availability(ThreadAction::Rename);
    let archive_availability = app.thread_action_availability(if is_archived {
        ThreadAction::Unarchive
    } else {
        ThreadAction::Archive
    });
    let delete_availability = app.thread_action_availability(ThreadAction::Delete);
    let group_name = SharedString::from(format!("thread-row-group-{id}"));

    div()
        .id(SharedString::from(format!("thread-row-{id}")))
        .relative()
        .group(group_name.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(8.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .tab_index(0)
        .cursor_pointer()
        .focus(|style| style.bg(colors.bg_hover))
        .bg(if is_selected {
            theme::hex_alpha(0xffffff, 0.06)
        } else {
            theme::transparent()
        })
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| {
            app.open_sidebar_thread(open_id.clone(), window, cx);
        }))
        .on_key_down(
            cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    app.open_sidebar_thread(keyboard_open_id.clone(), window, cx);
                    cx.stop_propagation();
                }
            }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
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
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(
                            Icon::empty()
                                .path(context_icon)
                                .with_size(px(10.0))
                                .text_color(colors.text_tertiary),
                        )
                        .child(context_label),
                ),
        )
        .when_some(activity, |this, activity| {
            this.child(thread_activity_indicator(activity))
        })
        .when(is_pinned && activity.is_none(), |this| {
            this.child(
                Icon::empty()
                    .path("icons/pin.svg")
                    .with_size(px(12.0))
                    .text_color(colors.text_tertiary),
            )
        })
        .child(
            div()
                .id(SharedString::from(format!("thread-overflow-{menu_id}")))
                .w(px(22.0))
                .h(px(22.0))
                .flex_shrink_0()
                .rounded(px(6.0))
                .tab_index(0)
                .opacity(if menu_open { 1.0 } else { 0.0 })
                .group_hover(group_name, |style| style.opacity(1.0))
                .focus(|style| style.opacity(1.0).bg(colors.bg_hover))
                .flex()
                .items_center()
                .justify_center()
                .hover(|style| style.bg(theme::hex_alpha(0xffffff, 0.07)))
                .on_click(cx.listener(move |app, _, window, cx| {
                    cx.stop_propagation();
                    app.toggle_sidebar_thread_menu(menu_id.clone(), window, cx);
                }))
                .on_key_down(
                    cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            app.toggle_sidebar_thread_menu(keyboard_menu_id.clone(), window, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
                .child(
                    Icon::empty()
                        .path("icons/ellipsis.svg")
                        .with_size(px(13.0))
                        .text_color(colors.text_tertiary),
                ),
        )
        .when(menu_open, |this| {
            this.child(
                deferred(sidebar_thread_overflow_menu(
                    is_pinned,
                    can_pin,
                    is_archived,
                    rename_availability,
                    archive_availability,
                    delete_availability,
                    cx,
                ))
                .with_priority(20),
            )
        })
        .into_any_element()
}

fn sidebar_thread_overflow_menu(
    is_pinned: bool,
    can_pin: bool,
    is_archived: bool,
    rename_availability: ActionAvailability,
    archive_availability: ActionAvailability,
    delete_availability: ActionAvailability,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let pin_label = if is_pinned { "Unpin" } else { "Pin" };
    let archive_label = if is_archived { "Unarchive" } else { "Archive" };
    div()
        .id("sidebar-thread-overflow-menu")
        .occlude()
        .absolute()
        .top(px(30.0))
        .right(px(2.0))
        .w(px(184.0))
        .tab_group()
        .rounded(px(10.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .shadow(vec![
            BoxShadow {
                color: theme::hex_alpha(0x000000, 0.04),
                offset: point(px(0.0), px(3.0)),
                blur_radius: px(7.5),
                spread_radius: px(0.0),
            },
            BoxShadow {
                color: theme::hex_alpha(0x000000, 0.05),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(20.0),
                spread_radius: px(0.0),
            },
        ])
        .p(px(4.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, _window, cx| {
            if event.keystroke.key == "escape" {
                app.close_sidebar_thread_menu(cx);
                cx.stop_propagation();
            }
        }))
        .when(can_pin, |this| {
            this.child(sidebar_thread_menu_item(
                "sidebar-thread-menu-pin",
                pin_label,
                "icons/pin.svg",
                false,
                ActionAvailability::Available,
                cx,
                move |app, _window, cx| {
                    let thread_id = app.selected_thread_id().unwrap_or_default().to_owned();
                    app.close_sidebar_thread_menu(cx);
                    app.set_thread_pinned(thread_id, !is_pinned, cx);
                },
            ))
        })
        .child(sidebar_thread_menu_item(
            "sidebar-thread-menu-rename",
            "Rename",
            "icons/pen-line.svg",
            false,
            rename_availability,
            cx,
            |app, window, cx| app.open_thread_rename(window, cx),
        ))
        .child(sidebar_thread_menu_item(
            "sidebar-thread-menu-archive",
            archive_label,
            "icons/inbox.svg",
            false,
            archive_availability,
            cx,
            |app, _window, cx| {
                app.close_sidebar_thread_menu(cx);
                app.archive_selected_thread(cx);
            },
        ))
        .child(sidebar_thread_menu_item(
            "sidebar-thread-menu-copy-id",
            "Copy conversation ID",
            "icons/copy.svg",
            false,
            ActionAvailability::Available,
            cx,
            |app, _window, cx| app.copy_selected_thread_id(cx),
        ))
        .child(
            div()
                .h(px(1.0))
                .mx(px(6.0))
                .my(px(3.0))
                .bg(colors.border_subtle),
        )
        .child(sidebar_thread_menu_item(
            "sidebar-thread-menu-delete",
            "Delete",
            "icons/delete.svg",
            true,
            delete_availability,
            cx,
            |app, _window, cx| {
                app.close_sidebar_thread_menu(cx);
                app.delete_selected_thread(cx);
            },
        ))
}

fn sidebar_thread_menu_item(
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    destructive: bool,
    availability: ActionAvailability,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    let enabled = availability.is_available();
    let reason = availability.reason().unwrap_or("Available");
    let on_click: Rc<dyn Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>)> =
        Rc::new(on_click);
    let mouse_click = Rc::clone(&on_click);
    let keyboard_click = Rc::clone(&on_click);
    let foreground = if !enabled {
        colors.text_tertiary
    } else if destructive {
        colors.status_error
    } else {
        colors.text
    };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(7.0))
        .rounded(px(7.0))
        .when(enabled, |row| {
            row.tab_index(0)
                .cursor_pointer()
                .focus(|style| style.bg(colors.bg_hover))
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| {
                    cx.stop_propagation();
                    mouse_click(app, window, cx);
                }))
                .on_key_down(
                    cx.listener(move |app, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            keyboard_click(app, window, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
        })
        .when(!enabled, |row| {
            row.opacity(0.58)
                .tooltip(move |window, cx| Tooltip::new(reason).build(window, cx))
        })
        .child(
            Icon::empty()
                .path(icon)
                .with_size(px(14.0))
                .text_color(if enabled {
                    if destructive {
                        colors.status_error
                    } else {
                        colors.text_tertiary
                    }
                } else {
                    colors.text_tertiary
                }),
        )
        .child(div().text_sm().text_color(foreground).child(label))
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
        .px(px(2.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .bg(theme::transparent())
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| app.toggle_mode_menu(cx)))
        .child(
            div()
                .text_base()
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
        .occlude()
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
        .on_click(cx.listener(move |app, _, window, cx| {
            on_click(app, window, cx);
            cx.stop_propagation();
        }))
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
    selected: bool,
    attention: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .relative()
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(if selected {
            colors.accent_soft
        } else {
            theme::transparent()
        })
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(on_click))
        .child(Icon::new(icon).with_size(px(15.0)).text_color(if selected {
            colors.accent
        } else {
            colors.text_secondary
        }))
        .when(attention && !selected, |this| {
            this.child(
                div()
                    .absolute()
                    .top(px(4.0))
                    .right(px(4.0))
                    .w(px(5.0))
                    .h(px(5.0))
                    .rounded_full()
                    .bg(colors.accent),
            )
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_day_headings_follow_the_reference_relative_day_contract() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 10).expect("valid date");
        assert_eq!(activity_day_heading(today, today), "Today");
        assert_eq!(
            activity_day_heading(
                NaiveDate::from_ymd_opt(2026, 8, 9).expect("valid date"),
                today
            ),
            "Yesterday"
        );
        assert_eq!(
            activity_day_heading(
                NaiveDate::from_ymd_opt(2026, 8, 8).expect("valid date"),
                today
            ),
            "Saturday"
        );
        assert_eq!(
            activity_day_heading(
                NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid date"),
                today
            ),
            "Wednesday"
        );
    }

    #[test]
    fn activity_timestamps_accept_seconds_and_milliseconds() {
        let seconds = 1_786_323_600;
        assert_eq!(
            local_date_from_timestamp(seconds),
            local_date_from_timestamp(seconds * 1_000)
        );
    }

    #[test]
    fn connection_state_is_never_conveyed_by_color_alone() {
        assert_eq!(
            connection_status_detail("Online", Some(40), 2, 3, 0),
            "Online · 40 sessions · 2 running · 3 updates"
        );
        assert_eq!(
            connection_status_detail("Degraded", Some(12), 1, 4, 2),
            "Degraded · 12 sessions · 2 need attention"
        );
        assert_eq!(
            connection_status_detail("Offline", None, 0, 0, 0),
            "Offline"
        );
    }
}
