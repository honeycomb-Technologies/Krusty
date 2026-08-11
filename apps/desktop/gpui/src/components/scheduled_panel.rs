//! Scheduled tasks destination backed by the Mitsuro Hive schedule control plane.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{
    schedule_toggle_action, MitsuroApp, ScheduleEditorInputs, ScheduleEditorMode,
    ScheduleEditorState, ScheduleRecurrenceKind, SurfaceDataState,
};
use crate::theme;
use mitsuro_desktop_backend::{
    ProductDstFoldPolicy, ProductDstGapPolicy, ProductMisfirePolicy, ProductMonthlyDayPolicy,
    ProductOverlapPolicy, ProductRetryJitter, ProductSchedule, ProductScheduleAction,
    ProductScheduleWeekday,
};

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
    let mutations_available = app.schedule_mutations_available();
    let editor = app.schedule_editor().cloned();
    let editor_inputs = editor.as_ref().map(|_| app.schedule_editor_inputs());
    let editor_element = editor
        .zip(editor_inputs)
        .map(|(editor, inputs)| schedule_editor(editor, inputs, cx).into_any_element());

    div()
        .id("scheduled-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(header(show_tasks, state, mutations_available, cx))
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
                .when_some(editor_element, |this, editor| this.child(editor))
                .child(match state {
                    SurfaceDataState::Live => {
                        live_tasks_section(live_tasks.as_deref().unwrap_or(&[]), app, cx)
                            .into_any_element()
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
    mutations_available: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let subtitle = match state {
        SurfaceDataState::Live => "Mitsuro Hive schedules · live catalog and controls",
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
                .bg(
                    if state == SurfaceDataState::Fixture || mutations_available {
                        colors.bg_button_primary
                    } else {
                        colors.bg_button_secondary
                    },
                )
                .when(state == SurfaceDataState::Fixture, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(colors.bg_button_primary_hover))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.request_schedule_creation(None, cx);
                        }))
                })
                .when(mutations_available, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(colors.bg_button_primary_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.open_schedule_creation(window, cx);
                        }))
                })
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(
                            if state == SurfaceDataState::Fixture || mutations_available {
                                colors.fg_button_primary
                            } else {
                                colors.text_tertiary
                            },
                        )
                        .child(
                            if state == SurfaceDataState::Fixture || mutations_available {
                                "Create"
                            } else {
                                "Unavailable"
                            },
                        ),
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

fn schedule_editor(
    editor: ScheduleEditorState,
    inputs: ScheduleEditorInputs,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let is_replace = matches!(editor.mode, ScheduleEditorMode::Replace { .. });
    let submitting = editor.submitting;
    let session_value = inputs.session.read(cx).value().to_string();
    div()
        .id("schedule-editor")
        .flex()
        .flex_col()
        .gap(px(14.0))
        .p(px(16.0))
        .rounded(px(14.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_base()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(if is_replace {
                                    "Edit schedule"
                                } else {
                                    "Create schedule"
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("Saved directly to the Mitsuro Hive control plane"),
                        ),
                )
                .child(
                    div()
                        .id("schedule-editor-close")
                        .px(px(9.0))
                        .py(px(5.0))
                        .rounded(px(7.0))
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .when(!submitting, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.close_schedule_editor(cx);
                                }))
                        })
                        .child("Close"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(10.0))
                .child(if is_replace {
                    editor_readonly_field(
                        "schedule-editor-session-fixed",
                        "Hive session (fixed)",
                        session_value,
                    )
                    .into_any_element()
                } else {
                    editor_input(
                        "schedule-editor-session",
                        "Hive session",
                        &inputs.session,
                        false,
                        false,
                    )
                    .into_any_element()
                })
                .child(editor_input(
                    "schedule-editor-title",
                    "Title",
                    &inputs.title,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-summary",
                    "Summary",
                    &inputs.summary,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-timezone",
                    "Timezone",
                    &inputs.timezone,
                    false,
                    false,
                )),
        )
        .child(editor_input(
            "schedule-editor-objective",
            "Objective",
            &inputs.objective,
            true,
            true,
        ))
        .child(editor_section_label("Recurrence"))
        .child(
            div().flex().flex_row().flex_wrap().gap(px(6.0)).children(
                [
                    ScheduleRecurrenceKind::Once,
                    ScheduleRecurrenceKind::Daily,
                    ScheduleRecurrenceKind::Weekdays,
                    ScheduleRecurrenceKind::Weekly,
                    ScheduleRecurrenceKind::Monthly,
                ]
                .into_iter()
                .enumerate()
                .map(|(index, kind)| {
                    editor_chip(
                        ("schedule-recurrence", index),
                        kind.label(),
                        editor.recurrence_kind == kind,
                        cx,
                        move |app, _, _, cx| app.set_schedule_recurrence_kind(kind, cx),
                    )
                }),
            ),
        )
        .when(
            editor.recurrence_kind == ScheduleRecurrenceKind::Once,
            |this| {
                this.child(editor_input(
                    "schedule-editor-once-at",
                    "Run once at (RFC3339)",
                    &inputs.once_at,
                    true,
                    false,
                ))
            },
        )
        .when(
            editor.recurrence_kind != ScheduleRecurrenceKind::Once,
            |this| {
                this.child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(10.0))
                        .child(editor_input(
                            "schedule-editor-start-date",
                            "Start date",
                            &inputs.start_date,
                            false,
                            false,
                        ))
                        .child(editor_input(
                            "schedule-editor-time",
                            "Local time",
                            &inputs.time,
                            false,
                            false,
                        )),
                )
            },
        )
        .when(
            editor.recurrence_kind == ScheduleRecurrenceKind::Weekly,
            |this| {
                this.child(editor_section_label("Run on")).child(
                    div().flex().flex_row().flex_wrap().gap(px(6.0)).children(
                        [
                            (ProductScheduleWeekday::Sunday, "Sun"),
                            (ProductScheduleWeekday::Monday, "Mon"),
                            (ProductScheduleWeekday::Tuesday, "Tue"),
                            (ProductScheduleWeekday::Wednesday, "Wed"),
                            (ProductScheduleWeekday::Thursday, "Thu"),
                            (ProductScheduleWeekday::Friday, "Fri"),
                            (ProductScheduleWeekday::Saturday, "Sat"),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (weekday, label))| {
                            editor_chip(
                                ("schedule-weekday", index),
                                label,
                                editor.weekdays.contains(&weekday),
                                cx,
                                move |app, _, _, cx| {
                                    app.toggle_schedule_weekday(weekday, cx);
                                },
                            )
                        }),
                    ),
                )
            },
        )
        .when(
            editor.recurrence_kind == ScheduleRecurrenceKind::Monthly,
            |this| {
                this.child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_end()
                        .gap(px(10.0))
                        .child(editor_input(
                            "schedule-editor-monthly-day",
                            "Day of month",
                            &inputs.monthly_day,
                            false,
                            false,
                        ))
                        .child(editor_chip(
                            "schedule-monthly-skip",
                            "Skip short months",
                            editor.monthly_day_policy == ProductMonthlyDayPolicy::Skip,
                            cx,
                            |app, _, _, cx| {
                                app.set_schedule_monthly_policy(ProductMonthlyDayPolicy::Skip, cx);
                            },
                        ))
                        .child(editor_chip(
                            "schedule-monthly-last",
                            "Use last day",
                            editor.monthly_day_policy == ProductMonthlyDayPolicy::LastDay,
                            cx,
                            |app, _, _, cx| {
                                app.set_schedule_monthly_policy(
                                    ProductMonthlyDayPolicy::LastDay,
                                    cx,
                                );
                            },
                        )),
                )
            },
        )
        .child(editor_section_label("Execution"))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(10.0))
                .child(editor_input(
                    "schedule-editor-project",
                    "Workspace",
                    &inputs.project_dir,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-model",
                    "Model",
                    &inputs.model,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-crew",
                    "Crew slug",
                    &inputs.crew_slug,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-priority",
                    "Priority",
                    &inputs.priority,
                    false,
                    false,
                )),
        )
        .child(
            div()
                .id("schedule-editor-advanced-toggle")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.accent)
                .cursor_pointer()
                .on_click(cx.listener(|app, _, _, cx| {
                    app.toggle_schedule_advanced(cx);
                }))
                .child(if editor.advanced_open {
                    "Hide advanced policy"
                } else {
                    "Show advanced policy"
                }),
        )
        .when(editor.advanced_open, |this| {
            this.child(advanced_schedule_policy(&editor, &inputs, cx))
        })
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(px(8.0))
                .child(
                    div()
                        .id("schedule-editor-cancel")
                        .h(px(32.0))
                        .px(px(13.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .bg(colors.bg_button_secondary)
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .when(!submitting, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.close_schedule_editor(cx);
                                }))
                        })
                        .child("Cancel"),
                )
                .child(
                    div()
                        .id("schedule-editor-submit")
                        .h(px(32.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .bg(colors.bg_button_primary)
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.fg_button_primary)
                        .when(!submitting, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.bg(colors.bg_button_primary_hover))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.submit_schedule_editor(cx);
                                }))
                        })
                        .when(submitting, |this| this.opacity(0.6))
                        .child(if submitting {
                            "Saving…"
                        } else if is_replace {
                            "Save changes"
                        } else {
                            "Create schedule"
                        }),
                ),
        )
}

fn advanced_schedule_policy(
    editor: &ScheduleEditorState,
    inputs: &ScheduleEditorInputs,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("schedule-editor-advanced")
        .flex()
        .flex_col()
        .gap(px(12.0))
        .p(px(12.0))
        .rounded(px(10.0))
        .bg(colors.bg_sidebar)
        .border_1()
        .border_color(colors.border)
        .child(editor_policy_row(
            "DST gap",
            vec![
                editor_chip(
                    "schedule-dst-gap-shift",
                    "Shift forward",
                    editor.dst_gap_policy == ProductDstGapPolicy::ShiftForward,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_dst_gap(ProductDstGapPolicy::ShiftForward, cx);
                    },
                )
                .into_any_element(),
                editor_chip(
                    "schedule-dst-gap-skip",
                    "Skip",
                    editor.dst_gap_policy == ProductDstGapPolicy::Skip,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_dst_gap(ProductDstGapPolicy::Skip, cx);
                    },
                )
                .into_any_element(),
            ],
        ))
        .child(editor_policy_row(
            "DST fold",
            vec![
                editor_chip(
                    "schedule-dst-fold-first",
                    "First",
                    editor.dst_fold_policy == ProductDstFoldPolicy::First,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_dst_fold(ProductDstFoldPolicy::First, cx);
                    },
                )
                .into_any_element(),
                editor_chip(
                    "schedule-dst-fold-second",
                    "Second",
                    editor.dst_fold_policy == ProductDstFoldPolicy::Second,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_dst_fold(ProductDstFoldPolicy::Second, cx);
                    },
                )
                .into_any_element(),
            ],
        ))
        .child(editor_policy_row(
            "Misfire",
            vec![
                editor_chip(
                    "schedule-misfire-skip",
                    "Skip",
                    editor.misfire_policy == ProductMisfirePolicy::Skip,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_misfire_policy(ProductMisfirePolicy::Skip, cx);
                    },
                )
                .into_any_element(),
                editor_chip(
                    "schedule-misfire-once",
                    "Fire once",
                    editor.misfire_policy == ProductMisfirePolicy::FireOnce,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_misfire_policy(ProductMisfirePolicy::FireOnce, cx);
                    },
                )
                .into_any_element(),
                editor_chip(
                    "schedule-misfire-catch-up",
                    "Catch up",
                    editor.misfire_policy == ProductMisfirePolicy::CatchUp,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_misfire_policy(ProductMisfirePolicy::CatchUp, cx);
                    },
                )
                .into_any_element(),
            ],
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(10.0))
                .child(editor_input(
                    "schedule-editor-grace",
                    "Misfire grace (seconds)",
                    &inputs.misfire_grace,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-catch-up",
                    "Catch-up limit",
                    &inputs.catch_up_limit,
                    false,
                    false,
                )),
        )
        .child(editor_policy_row(
            "Overlap",
            vec![
                editor_chip(
                    "schedule-overlap-skip",
                    "Skip",
                    editor.overlap_policy == ProductOverlapPolicy::Skip,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_overlap_policy(ProductOverlapPolicy::Skip, cx);
                    },
                )
                .into_any_element(),
                editor_chip(
                    "schedule-overlap-queue",
                    "Queue one",
                    editor.overlap_policy == ProductOverlapPolicy::QueueOne,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_overlap_policy(ProductOverlapPolicy::QueueOne, cx);
                    },
                )
                .into_any_element(),
                editor_chip(
                    "schedule-overlap-allow",
                    "Allow",
                    editor.overlap_policy == ProductOverlapPolicy::Allow,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_overlap_policy(ProductOverlapPolicy::Allow, cx);
                    },
                )
                .into_any_element(),
            ],
        ))
        .child(editor_policy_row(
            "Retry jitter",
            vec![
                editor_chip(
                    "schedule-retry-none",
                    "None",
                    editor.retry_jitter == ProductRetryJitter::None,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_retry_jitter(ProductRetryJitter::None, cx);
                    },
                )
                .into_any_element(),
                editor_chip(
                    "schedule-retry-full",
                    "Full",
                    editor.retry_jitter == ProductRetryJitter::Full,
                    cx,
                    |app, _, _, cx| {
                        app.set_schedule_retry_jitter(ProductRetryJitter::Full, cx);
                    },
                )
                .into_any_element(),
            ],
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(10.0))
                .child(editor_input(
                    "schedule-editor-attempts",
                    "Retry attempts",
                    &inputs.retry_attempts,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-retry-base",
                    "Base delay (seconds)",
                    &inputs.retry_base,
                    false,
                    false,
                ))
                .child(editor_input(
                    "schedule-editor-retry-max",
                    "Max delay (seconds)",
                    &inputs.retry_max,
                    false,
                    false,
                )),
        )
}

fn editor_policy_row(title: &'static str, controls: Vec<gpui::AnyElement>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(px(6.0))
        .child(
            div()
                .w(px(110.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(title),
        )
        .children(controls)
}

fn editor_section_label(label: &'static str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text_secondary)
        .child(label)
}

fn editor_input(
    id: &'static str,
    label: &'static str,
    input: &gpui::Entity<gpui_component::input::InputState>,
    full_width: bool,
    tall: bool,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(5.0))
        .when(full_width, |this| this.w_full())
        .when(!full_width, |this| this.w(px(250.0)))
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
        )
        .child(
            div()
                .flex()
                .w_full()
                .h(if tall { px(76.0) } else { px(34.0) })
                .px(px(10.0))
                .rounded(px(8.0))
                .bg(colors.bg_sidebar)
                .border_1()
                .border_color(colors.border)
                .child(Input::new(input).appearance(false).h(if tall {
                    px(72.0)
                } else {
                    px(30.0)
                })),
        )
}

fn editor_readonly_field(id: &'static str, label: &'static str, value: String) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(5.0))
        .w(px(250.0))
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_center()
                .w_full()
                .h(px(34.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .bg(theme::hex_alpha(0xffffff, 0.025))
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text_secondary)
                .child(value),
        )
}

fn editor_chip(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(7.0))
        .bg(if selected {
            colors.accent_soft
        } else {
            colors.bg_button_secondary
        })
        .border_1()
        .border_color(if selected {
            colors.accent
        } else {
            colors.border
        })
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if selected {
            colors.accent
        } else {
            colors.text_secondary
        })
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(on_click))
        .child(label)
}

fn live_tasks_section(
    tasks: &[ProductSchedule],
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let mutations_available = app.schedule_mutations_available();
    let active_mutation = app.schedule_mutation_id().map(str::to_owned);
    let cancel_confirmation = app.schedule_cancel_confirmation().map(str::to_owned);
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
            let schedule_for_toggle = schedule.clone();
            let schedule_for_cancel = schedule.clone();
            let schedule_for_edit = schedule.clone();
            let toggle_action = schedule_toggle_action(&schedule.status);
            let is_terminal = toggle_action.is_none();
            let any_mutation = active_mutation.is_some();
            let is_mutating = active_mutation.as_deref() == Some(schedule.id.as_str());
            let confirming_cancel = cancel_confirmation.as_deref() == Some(schedule.id.as_str());
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
                        .flex_1()
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
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .when(is_mutating, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(colors.text_tertiary)
                                    .child("Updating…"),
                            )
                        })
                        .when(mutations_available && !is_terminal, |this| {
                            let action =
                                toggle_action.expect("non-terminal schedule has a toggle action");
                            this.child(schedule_action_button(
                                ("schedule-edit", index),
                                "Edit",
                                !any_mutation,
                                false,
                                cx,
                                move |app, _, window, cx| {
                                    app.open_schedule_replacement(
                                        schedule_for_edit.clone(),
                                        window,
                                        cx,
                                    );
                                },
                            ))
                            .child(schedule_action_button(
                                ("schedule-toggle", index),
                                if action == ProductScheduleAction::Resume {
                                    "Resume"
                                } else {
                                    "Pause"
                                },
                                !any_mutation,
                                false,
                                cx,
                                move |app, _, _, cx| {
                                    app.mutate_schedule(schedule_for_toggle.clone(), action, cx);
                                },
                            ))
                            .child(schedule_action_button(
                                ("schedule-cancel", index),
                                if confirming_cancel {
                                    "Confirm cancel"
                                } else {
                                    "Cancel"
                                },
                                !any_mutation,
                                confirming_cancel,
                                cx,
                                move |app, _, _, cx| {
                                    app.mutate_schedule(
                                        schedule_for_cancel.clone(),
                                        ProductScheduleAction::Cancel,
                                        cx,
                                    );
                                },
                            ))
                        })
                        .when(!mutations_available || is_terminal, |this| {
                            this.child(
                                div()
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded(px(999.0))
                                    .bg(theme::hex_alpha(0xffffff, 0.06))
                                    .text_xs()
                                    .text_color(colors.text_secondary)
                                    .child(if is_terminal {
                                        schedule.status.clone()
                                    } else {
                                        "Unavailable".to_owned()
                                    }),
                            )
                        }),
                )
        }))
}

fn schedule_action_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    enabled: bool,
    destructive: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(7.0))
        .bg(if destructive {
            theme::hex_alpha(0xef4444, 0.14)
        } else {
            colors.bg_button_secondary
        })
        .border_1()
        .border_color(if destructive {
            theme::hex_alpha(0xef4444, 0.38)
        } else {
            colors.border
        })
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if destructive {
            theme::hex(0xfca5a5)
        } else if enabled {
            colors.text_secondary
        } else {
            colors.text_tertiary
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(on_click))
        })
        .child(label)
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
