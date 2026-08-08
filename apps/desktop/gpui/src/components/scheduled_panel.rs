//! Scheduled tasks destination — suggestions-first (bar parity), optional Your tasks.
//!
//! No schedule/* app-server methods. Suggestions are product chrome (bar-like).
//! "Your tasks" densifies fixture rows after Create (explicit fixture demo badge).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, UiConnection};
use crate::theme;

#[derive(Clone, Copy)]
struct ScheduleItem {
    name: &'static str,
    when: &'static str,
    detail: &'static str,
    icon_path: &'static str,
}

/// Bar-aligned suggestion cards (primary chrome when no user tasks).
const SUGGESTIONS: &[ScheduleItem] = &[
    ScheduleItem {
        name: "Daily brief",
        when: "Weekdays at 8:00 AM",
        detail: "Start each weekday with a summary of your calendar, unread email, and priorities",
        icon_path: "icons/clock.svg",
    },
    ScheduleItem {
        name: "Weekly review",
        when: "Fridays at 4:00 PM",
        detail: "Turn your recent work into a concise status update every Friday",
        icon_path: "icons/calendar.svg",
    },
    ScheduleItem {
        name: "Follow-up monitor",
        when: "Weekdays at 9:00 AM",
        detail:
            "Review recent email and calendar activity and flag anything that needs your attention",
        icon_path: "icons/bell.svg",
    },
];

/// Optional secondary list after Create / suggestion pick (not always-on).
const YOUR_TASKS: &[ScheduleItem] = &[
    ScheduleItem {
        name: "Nightly catalog sweep",
        when: "Daily · 2:00 AM",
        detail: "Refresh local catalog overnight",
        icon_path: "icons/clock.svg",
    },
    ScheduleItem {
        name: "PR digest",
        when: "Mon–Fri · 9:00 AM",
        detail: "Open pull requests summary",
        icon_path: "icons/git-pull-request.svg",
    },
];

/// Full-height Scheduled panel (sidebar nav destination).
pub fn scheduled_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let enabled = app.scheduled_enabled().to_vec();
    let show_tasks = app.scheduled_show_tasks();
    let live = matches!(app.connection(), UiConnection::Ready { .. });

    div()
        .id("scheduled-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(header(show_tasks, live, cx))
        .child(search_placeholder())
        .child(
            div()
                .id("scheduled-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(28.0))
                .pb(px(28.0))
                .gap(px(18.0))
                // Bar: suggestions-first; Your tasks only when user densifies via Create.
                .when(show_tasks, |this| {
                    this.child(tasks_section(&enabled, cx).into_any_element())
                })
                .child(suggestions_section(cx)),
        )
}

fn header(show_tasks: bool, live: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let subtitle = if show_tasks {
        "Fixture demo tasks · no schedule/* in app-server"
    } else if live {
        "Suggestions only · product empty tasks while connected (no schedule protocol)"
    } else {
        "Ask Mitsuro to schedule tasks, set reminders, or monitor for updates"
    };
    div()
        .id("scheduled-header")
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
                                .child("Scheduled tasks"),
                        )
                        .when(show_tasks, |this| {
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
                .id("scheduled-create")
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
                    app.set_scheduled_show_tasks(true, cx);
                    app.set_status_line("Scheduled · fixture demo tasks", cx);
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
        )
}

fn search_placeholder() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("scheduled-search")
        .mx(px(28.0))
        .mb(px(4.0))
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
                .child("Search scheduled tasks"),
        )
}

fn tasks_section(enabled: &[bool], cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("scheduled-tasks")
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("Your tasks"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("fixture densify"),
                        ),
                )
                .child(
                    div()
                        .id("scheduled-clear")
                        .cursor_pointer()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.set_scheduled_show_tasks(false, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("Clear"),
                        ),
                ),
        )
        .children(
            YOUR_TASKS
                .iter()
                .enumerate()
                .map(|(i, task)| {
                    let on = enabled.get(i).copied().unwrap_or(true);
                    task_row(i as u64, task, on, true, cx).into_any_element()
                })
                .collect::<Vec<_>>(),
        )
}

fn suggestions_section(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("scheduled-suggestions")
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("Suggestions"),
        )
        .children(
            SUGGESTIONS
                .iter()
                .enumerate()
                .map(|(i, s)| suggestion_row(i as u64, s, cx).into_any_element())
                .collect::<Vec<_>>(),
        )
}

fn suggestion_row(
    index: u64,
    item: &ScheduleItem,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let name = item.name;
    div()
        .id(("sched-suggest", index))
        .flex()
        .flex_row()
        .items_start()
        .gap(px(12.0))
        .px(px(4.0))
        .py(px(8.0))
        .rounded(px(10.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_scheduled_show_tasks(true, cx);
            app.set_status_line(format!("Scheduled · added suggestion “{name}”"), cx);
        }))
        .child(
            Icon::empty()
                .path(item.icon_path)
                .with_size(px(16.0))
                .text_color(colors.accent),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .whitespace_nowrap()
                                .child(item.name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .whitespace_nowrap()
                                .child(item.when),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .child(item.detail),
                ),
        )
}

fn task_row(
    index: u64,
    task: &ScheduleItem,
    enabled: bool,
    toggleable: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(("sched-task", index))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(10.0))
                .min_w_0()
                .flex_1()
                .child(
                    Icon::empty()
                        .path(task.icon_path)
                        .with_size(px(16.0))
                        .text_color(if enabled {
                            colors.accent
                        } else {
                            colors.text_tertiary
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(task.name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(format!("{} · {}", task.when, task.detail)),
                        ),
                ),
        )
        .when(toggleable, |this| this.child(toggle(index, enabled, cx)))
}

fn toggle(index: u64, on: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(("sched-toggle", index))
        .w(px(40.0))
        .h(px(22.0))
        .rounded(px(999.0))
        .bg(if on {
            colors.accent
        } else {
            theme::hex_alpha(0xffffff, 0.12)
        })
        .flex()
        .items_center()
        .px(px(2.0))
        .cursor_pointer()
        .on_click(cx.listener(move |app, _, _, cx| {
            app.toggle_scheduled_enabled(index as usize, cx);
        }))
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .rounded_full()
                .bg(colors.text)
                .when(on, |this| this.ml(px(18.0)))
                .when(!on, |this| this.ml(px(0.0))),
        )
}
