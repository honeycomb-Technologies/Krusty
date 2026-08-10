//! Scheduled tasks destination with a read-only Mitsuro Hive schedule catalog.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, SurfaceDataState};
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
    let live_tasks = app.scheduled_tasks().map(|tasks| tasks.to_vec());
    let state = app.scheduled_state();

    div()
        .id("scheduled-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(header(show_tasks, state, cx))
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
                .child(match state {
                    SurfaceDataState::Live => {
                        live_tasks_section(live_tasks.as_deref().unwrap_or(&[])).into_any_element()
                    }
                    SurfaceDataState::Fixture if show_tasks => {
                        tasks_section(&enabled, cx).into_any_element()
                    }
                    SurfaceDataState::Fixture => suggestions_section(false, cx).into_any_element(),
                    SurfaceDataState::Loading => state_notice(
                        "Loading scheduled tasks",
                        "Waiting for the selected backend to finish connecting.",
                    )
                    .into_any_element(),
                    SurfaceDataState::Unsupported => state_notice(
                        "Scheduled tasks are not supported",
                        "The ChatGPT / Codex app-server does not expose Mitsuro Hive schedules.",
                    )
                    .into_any_element(),
                    SurfaceDataState::Error => state_notice(
                        "Scheduled tasks unavailable",
                        "The selected backend could not provide a schedule catalog.",
                    )
                    .into_any_element(),
                }),
        )
}

fn header(
    show_tasks: bool,
    state: SurfaceDataState,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let subtitle = match state {
        SurfaceDataState::Live => "Mitsuro Hive schedules · live catalog · read-only",
        SurfaceDataState::Fixture if show_tasks => "Explicit fixture tasks",
        SurfaceDataState::Fixture => "Explicit fixture suggestions",
        SurfaceDataState::Loading => "Waiting for backend data",
        SurfaceDataState::Unsupported => "Unavailable from this backend",
        SurfaceDataState::Error => "Backend error",
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
                        .when(state == SurfaceDataState::Fixture, |this| {
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
                .bg(if state == SurfaceDataState::Fixture {
                    colors.bg_button_primary
                } else {
                    colors.bg_button_secondary
                })
                .when(state == SurfaceDataState::Fixture, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(colors.bg_button_primary_hover))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.request_schedule_creation(None, cx);
                        }))
                })
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if state == SurfaceDataState::Fixture {
                            colors.fg_button_primary
                        } else {
                            colors.text_tertiary
                        })
                        .child(if state == SurfaceDataState::Fixture {
                            "Create"
                        } else {
                            "Unavailable"
                        }),
                )
                .child(
                    Icon::new(IconName::ChevronDown)
                        .with_size(px(12.0))
                        .text_color(colors.fg_button_primary),
                ),
        )
}

fn state_notice(title: &str, detail: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .px(px(16.0))
        .py(px(18.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_sm()
                .text_color(colors.text_tertiary)
                .child(detail.to_string()),
        )
}

fn live_tasks_section(tasks: &[mitsuro_desktop_backend::ProductSchedule]) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("scheduled-live-tasks")
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(format!("Your Hive schedules · {}", tasks.len())),
        )
        .when(tasks.is_empty(), |this| {
            this.child(
                div()
                    .px(px(14.0))
                    .py(px(16.0))
                    .rounded(px(12.0))
                    .bg(colors.bg_elevated)
                    .border_1()
                    .border_color(colors.border)
                    .text_sm()
                    .text_color(colors.text_tertiary)
                    .child("No Hive schedules are currently available."),
            )
        })
        .children(tasks.iter().enumerate().map(|(index, schedule)| {
            let timing = schedule
                .next_fire_at
                .as_deref()
                .map(|next| format!("next {next}"))
                .unwrap_or_else(|| "no next occurrence".to_owned());
            let detail = if schedule.summary.is_empty() {
                schedule.objective.clone()
            } else {
                schedule.summary.clone()
            };
            div()
                .id(("live-schedule", index))
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
                        .flex_col()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .child(schedule.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!("{} · {} · {detail}", schedule.status, timing)),
                        ),
                )
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(999.0))
                        .bg(theme::hex_alpha(0xffffff, 0.06))
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .child("read-only"),
                )
        }))
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

fn suggestions_section(live: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
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
                .map(|(i, s)| suggestion_row(i as u64, s, live, cx).into_any_element())
                .collect::<Vec<_>>(),
        )
}

fn suggestion_row(
    index: u64,
    item: &ScheduleItem,
    live: bool,
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
        .when(!live, |this| {
            this.cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.request_schedule_creation(Some(name), cx);
                }))
        })
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
