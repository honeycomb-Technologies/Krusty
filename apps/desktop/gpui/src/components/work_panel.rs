//! Work mode — explicit fixture goals or authoritative Mitsuro Hive runs.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};
use mitsuro_desktop_backend::{
    ProductHivePriority, ProductHiveSessionDetail, ProductHiveSnapshot, ProductHiveTask,
};

use crate::app::{
    hive_goal_status, hive_session_toggle_action, HiveDispatchEditorState, HiveWorkInputs,
    MitsuroApp, SurfaceDataState,
};
use crate::components::codex_button;
use crate::demo::{DemoGoal, DemoGoalStatus, DemoPlanItem};
use crate::theme;

/// Full-height Work panel: Hive catalog + selected session controls, or explicit fixtures.
pub fn work_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let goals = app.goals().to_vec();
    let selected = app.selected_goal_id().map(str::to_string);
    let selected_goal = app.selected_goal().cloned();
    let state = app.work_state();
    let live_hive = app.goals_are_live_hive();
    let snapshot = app.hive_snapshot().cloned();
    let detail = app.hive_session_detail().cloned();
    let detail_state = app.hive_detail_state();
    let mutations_available = app.hive_mutations_available();
    let mutation_in_progress = app.hive_mutation_id().is_some();
    let cancel_confirmation = app.hive_cancel_confirmation().map(str::to_owned);
    let editor = app.hive_dispatch_editor().cloned();
    let inputs = app.hive_work_inputs();
    let model = app.selected_model_slug();
    let active = goals
        .iter()
        .filter(|goal| goal.status == DemoGoalStatus::Active)
        .count();

    div()
        .id("work-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(work_title_bar(
            goals.len(),
            active,
            state,
            mutations_available,
            app.status_line().as_ref(),
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .when_some(editor, |this, editor| {
                    this.child(dispatch_editor(editor, inputs.clone(), model, cx))
                })
                .child(if goals.is_empty() {
                    work_empty_state(state, mutations_available, cx).into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h_0()
                        .child(goal_list(
                            &goals,
                            selected.as_deref(),
                            state,
                            snapshot.as_ref(),
                            cx,
                        ))
                        .child(goal_detail(
                            selected_goal.as_ref(),
                            state,
                            live_hive,
                            detail.as_ref(),
                            detail_state,
                            &inputs,
                            mutations_available,
                            mutation_in_progress,
                            cancel_confirmation.as_deref(),
                            cx,
                        ))
                        .into_any_element()
                }),
        )
}

fn work_title_bar(
    count: usize,
    active: usize,
    state: SurfaceDataState,
    mutations_available: bool,
    status: &str,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let subtitle = match state {
        SurfaceDataState::Live => format!("{count} Hive run(s) · {active} running · live"),
        _ => format!("{count} goal(s) · {active} active · {}", state.label()),
    };
    div()
        .id("work-title")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(16.0))
        .py(px(12.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_w(px(180.0))
                .items_center()
                .gap(px(10.0))
                .child(
                    Icon::new(IconName::LayoutDashboard)
                        .with_size(px(16.0))
                        .text_color(colors.text),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("Work"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(subtitle),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .max_w(px(180.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(status.to_string()),
                )
                .child(
                    codex_button::primary_with_icon(
                        "work-header-create",
                        if state == SurfaceDataState::Fixture {
                            "New fixture goal"
                        } else {
                            "New Hive run"
                        },
                        Icon::new(IconName::Plus).with_size(px(12.0)),
                        cx,
                    )
                    .rounded(px(8.0))
                    .when(state == SurfaceDataState::Fixture, |this| {
                        this.on_click(
                            cx.listener(|app, _, window, cx| app.start_new_goal(window, cx)),
                        )
                    })
                    .when(mutations_available, |this| {
                        this.on_click(cx.listener(|app, _, window, cx| {
                            app.open_hive_dispatch_editor(window, cx);
                        }))
                    })
                    .when(
                        state != SurfaceDataState::Fixture && !mutations_available,
                        |this| this.opacity(0.45),
                    ),
                ),
        )
}

fn work_empty_state(
    state: SurfaceDataState,
    mutations_available: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let (title, detail) = match state {
        SurfaceDataState::Live => (
            "No Hive runs",
            "Dispatch a run to begin durable autonomous work on the connected Mitsuro server.",
        ),
        SurfaceDataState::Fixture => (
            "No fixture goals yet",
            "Explicit fixture mode can create local sample goals and plan items.",
        ),
        SurfaceDataState::Loading => (
            "Loading Work",
            "Waiting for the selected backend to return its Hive catalog.",
        ),
        SurfaceDataState::Error => (
            "Work unavailable",
            "The selected backend could not provide an authoritative Hive catalog.",
        ),
        SurfaceDataState::Unsupported => (
            "Work is not supported",
            "The ChatGPT / Codex app-server does not expose Mitsuro Hive runs.",
        ),
    };
    div()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .px(px(24.0))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(14.0))
                .max_w(px(440.0))
                .child(
                    Icon::new(IconName::LayoutDashboard)
                        .with_size(px(28.0))
                        .text_color(colors.text_tertiary),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .text_center()
                        .child(detail),
                )
                .when(state == SurfaceDataState::Live, |this| {
                    this.child(
                        codex_button::primary_with_icon(
                            "work-empty-create",
                            "New Hive run",
                            Icon::new(IconName::Plus).with_size(px(14.0)),
                            cx,
                        )
                        .rounded(px(10.0))
                        .when(mutations_available, |button| {
                            button.on_click(cx.listener(|app, _, window, cx| {
                                app.open_hive_dispatch_editor(window, cx);
                            }))
                        })
                        .when(!mutations_available, |button| button.opacity(0.45)),
                    )
                })
                .when(state == SurfaceDataState::Fixture, |this| {
                    this.child(
                        codex_button::primary_with_icon(
                            "work-empty-fixture-create",
                            "Create fixture goal",
                            Icon::new(IconName::Plus).with_size(px(14.0)),
                            cx,
                        )
                        .rounded(px(10.0))
                        .on_click(cx.listener(|app, _, window, cx| app.start_new_goal(window, cx))),
                    )
                }),
        )
}

fn goal_list(
    goals: &[DemoGoal],
    selected: Option<&str>,
    state: SurfaceDataState,
    snapshot: Option<&ProductHiveSnapshot>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("work-goal-list")
        .flex()
        .flex_col()
        .w(px(300.0))
        .h_full()
        .bg(colors.bg_sidebar)
        .border_r_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(12.0))
                .py(px(10.0))
                .border_b_1()
                .border_color(colors.border)
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text_secondary)
                        .child(if state == SurfaceDataState::Live {
                            "Hive runs"
                        } else {
                            "Goals"
                        }),
                )
                .child(
                    div()
                        .id("work-refresh")
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .when(state == SurfaceDataState::Live, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.text_color(colors.text))
                                .on_click(cx.listener(|app, _, _, cx| app.refresh_hive_now(cx)))
                        })
                        .when(state == SurfaceDataState::Fixture, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.text_color(colors.text))
                                .on_click(
                                    cx.listener(|app, _, window, cx| {
                                        app.start_new_goal(window, cx)
                                    }),
                                )
                        })
                        .child(if state == SurfaceDataState::Live {
                            "Refresh"
                        } else if state == SurfaceDataState::Fixture {
                            "Create"
                        } else {
                            "Unavailable"
                        }),
                ),
        )
        .child(
            div()
                .id("work-goal-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .py(px(6.0))
                .gap(px(2.0))
                .children(goals.iter().enumerate().map(|(index, goal)| {
                    let id = goal.id.clone();
                    let selected = selected == Some(goal.id.as_str());
                    let objective = goal.objective.clone();
                    let live_run = snapshot.and_then(|snapshot| {
                        snapshot.runs.iter().find(|run| run.session_id == goal.id)
                    });
                    let (status_label, done, total) = live_run.map_or_else(
                        || {
                            (
                                goal.status.label().to_owned(),
                                goal.plan_items.iter().filter(|item| item.done).count(),
                                goal.plan_items.len(),
                            )
                        },
                        |run| {
                            (
                                run.runtime_status
                                    .clone()
                                    .unwrap_or_else(|| run.agent_state.clone()),
                                run.completed_tasks,
                                run.pending_tasks
                                    + run.in_progress_tasks
                                    + run.completed_tasks
                                    + run.failed_tasks
                                    + run.blocked_tasks,
                            )
                        },
                    );
                    div()
                        .id(("work-goal-row", index as u64))
                        .mx(px(8.0))
                        .px(px(12.0))
                        .py(px(12.0))
                        .rounded(px(10.0))
                        .cursor_pointer()
                        .bg(if selected {
                            colors.bg_selected
                        } else {
                            colors.bg_sidebar
                        })
                        .border_1()
                        .border_color(if selected {
                            colors.border_heavy
                        } else {
                            colors.border
                        })
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.select_goal(id.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_start()
                                .gap(px(8.0))
                                .child(status_dot(goal.status))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(colors.text)
                                        .child(objective),
                                ),
                        )
                        .child(
                            div()
                                .mt(px(8.0))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(status_chip(&status_label, goal.status))
                                .child(div().text_xs().text_color(colors.text_tertiary).child(
                                    if state == SurfaceDataState::Live {
                                        format!("{done}/{total} tasks")
                                    } else {
                                        format!("{done}/{total} plan")
                                    },
                                )),
                        )
                        .child(progress_bar(done, total))
                })),
        )
}

#[allow(clippy::too_many_arguments)]
fn goal_detail(
    goal: Option<&DemoGoal>,
    state: SurfaceDataState,
    live_hive: bool,
    detail: Option<&ProductHiveSessionDetail>,
    detail_state: SurfaceDataState,
    inputs: &HiveWorkInputs,
    mutations_available: bool,
    mutation_in_progress: bool,
    cancel_confirmation: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("work-goal-detail")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(match goal {
            Some(goal) if live_hive => match detail {
                Some(detail) if detail.session_id == goal.id => live_goal_detail(
                    detail,
                    inputs,
                    mutations_available,
                    mutation_in_progress,
                    cancel_confirmation,
                    cx,
                )
                .into_any_element(),
                _ => detail_notice(detail_state).into_any_element(),
            },
            Some(goal) => fixture_goal_detail(goal, state, cx).into_any_element(),
            None => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("Select a run")
                .into_any_element(),
        })
}

fn detail_notice(state: SurfaceDataState) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(colors.text_tertiary)
        .child(match state {
            SurfaceDataState::Loading => "Loading authoritative session detail…",
            SurfaceDataState::Error => "Could not load this Hive session.",
            SurfaceDataState::Unsupported => "Session details are unavailable on this backend.",
            SurfaceDataState::Fixture => "Fixture detail is not a Hive session.",
            SurfaceDataState::Live => "No session detail returned.",
        })
}

fn live_goal_detail(
    detail: &ProductHiveSessionDetail,
    inputs: &HiveWorkInputs,
    mutations_available: bool,
    mutation_in_progress: bool,
    cancel_confirmation: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let runtime_label = detail
        .runtime_status
        .as_deref()
        .unwrap_or(detail.agent_state.as_str());
    let visual_status = hive_goal_status(detail.runtime_status.as_deref(), &detail.agent_state);
    let toggle = hive_session_toggle_action(detail.runtime_status.as_deref());
    let toggle_label = toggle.as_ref().map(|action| {
        if matches!(
            action,
            mitsuro_desktop_backend::ProductHiveSessionAction::Pause
        ) {
            "Pause"
        } else {
            "Resume"
        }
    });
    let cancel_armed = cancel_confirmation == Some(detail.session_id.as_str());
    let crew = detail.crew_slug.as_deref().unwrap_or("unassigned");
    let priority = detail.priority;

    div()
        .id("work-live-detail-scroll")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(28.0))
        .py(px(20.0))
        .gap(px(16.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(16.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(status_chip(runtime_label, visual_status))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.text_tertiary)
                                        .child(format!("priority · {}", priority_label(priority))),
                                ),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(detail.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!("{} · crew {crew}", detail.session_id)),
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
                                .id("hive-refresh-detail")
                                .h(px(32.0))
                                .px(px(12.0))
                                .flex()
                                .items_center()
                                .rounded(px(8.0))
                                .bg(colors.bg_button_secondary)
                                .text_xs()
                                .text_color(colors.text_secondary)
                                .cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(|app, _, _, cx| app.refresh_hive_now(cx)))
                                .child("Refresh"),
                        )
                        .when_some(toggle_label, |this, label| {
                            this.child(
                                div()
                                    .id("hive-toggle-session")
                                    .h(px(32.0))
                                    .px(px(12.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(8.0))
                                    .bg(colors.bg_button_secondary)
                                    .text_xs()
                                    .text_color(colors.text_secondary)
                                    .when(mutations_available && !mutation_in_progress, |button| {
                                        button
                                            .cursor_pointer()
                                            .hover(|style| style.bg(colors.bg_hover))
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.toggle_selected_hive_session(cx);
                                            }))
                                    })
                                    .when(!mutations_available || mutation_in_progress, |button| {
                                        button.opacity(0.45)
                                    })
                                    .child(label),
                            )
                        }),
                ),
        )
        .child(runtime_metadata(detail))
        .when_some(detail.last_error.as_ref(), |this, error| {
            this.child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded(px(8.0))
                    .bg(theme::hex_alpha(0xef4444, 0.08))
                    .border_1()
                    .border_color(theme::hex_alpha(0xef4444, 0.3))
                    .text_sm()
                    .text_color(colors.status_error)
                    .child(error.clone()),
            )
        })
        .child(section_header(
            "Tasks",
            &format!("{} authoritative row(s)", detail.tasks.len()),
        ))
        .child(if detail.tasks.is_empty() {
            div()
                .py(px(18.0))
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("This session has no autonomous task rows yet.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(
                    detail
                        .tasks
                        .iter()
                        .cloned()
                        .enumerate()
                        .map(|(index, task)| live_task_row(index as u64, task)),
                )
                .into_any_element()
        })
        .child(section_header(
            "Direction",
            "Messages are persisted and wake the selected Hive session",
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .h(px(72.0))
                        .px(px(10.0))
                        .rounded(px(8.0))
                        .bg(colors.bg_sidebar)
                        .border_1()
                        .border_color(colors.border)
                        .child(Input::new(&inputs.message).appearance(false).h(px(68.0))),
                )
                .child(
                    div()
                        .id("hive-send-message")
                        .h(px(34.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .bg(colors.bg_button_primary)
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.fg_button_primary)
                        .when(mutations_available && !mutation_in_progress, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.bg(colors.bg_button_primary_hover))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.submit_hive_message(cx);
                                }))
                        })
                        .when(!mutations_available || mutation_in_progress, |button| {
                            button.opacity(0.45)
                        })
                        .child(if mutation_in_progress {
                            "Applying…"
                        } else {
                            "Send"
                        }),
                ),
        )
        .child(section_header(
            "Run policy",
            "Priority and crew changes are written to the Mitsuro control plane",
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("Priority"),
                        )
                        .child(priority_controls(
                            "hive-detail-priority",
                            priority,
                            mutations_available && !mutation_in_progress,
                            false,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("Crew"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .flex()
                                        .w(px(250.0))
                                        .h(px(34.0))
                                        .px(px(10.0))
                                        .rounded(px(8.0))
                                        .bg(colors.bg_sidebar)
                                        .border_1()
                                        .border_color(colors.border)
                                        .child(
                                            Input::new(&inputs.crew_update)
                                                .appearance(false)
                                                .h(px(30.0)),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("hive-apply-crew")
                                        .h(px(34.0))
                                        .px(px(12.0))
                                        .flex()
                                        .items_center()
                                        .rounded(px(8.0))
                                        .bg(colors.bg_button_secondary)
                                        .text_xs()
                                        .text_color(colors.text_secondary)
                                        .when(
                                            mutations_available && !mutation_in_progress,
                                            |button| {
                                                button
                                                    .cursor_pointer()
                                                    .hover(|style| style.bg(colors.bg_hover))
                                                    .on_click(cx.listener(|app, _, _, cx| {
                                                        app.submit_selected_hive_crew(cx);
                                                    }))
                                            },
                                        )
                                        .when(
                                            !mutations_available || mutation_in_progress,
                                            |button| button.opacity(0.45),
                                        )
                                        .child("Apply"),
                                ),
                        ),
                ),
        )
        .child(
            div()
                .pt(px(4.0))
                .border_t_1()
                .border_color(colors.border)
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child("Cancellation permanently deletes the Hive session."),
                )
                .child(
                    div()
                        .id("hive-cancel-session")
                        .h(px(32.0))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .bg(if cancel_armed {
                            theme::hex_alpha(0xef4444, 0.16)
                        } else {
                            colors.bg_button_secondary
                        })
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.status_error)
                        .when(mutations_available && !mutation_in_progress, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.bg(theme::hex_alpha(0xef4444, 0.22)))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.cancel_selected_hive_session(cx);
                                }))
                        })
                        .when(!mutations_available || mutation_in_progress, |button| {
                            button.opacity(0.45)
                        })
                        .child(if cancel_armed {
                            "Confirm cancel run"
                        } else {
                            "Cancel run"
                        }),
                ),
        )
}

fn runtime_metadata(detail: &ProductHiveSessionDetail) -> impl IntoElement {
    let colors = theme::colors();
    let next_wake = detail.next_wake_at.as_deref().unwrap_or("—");
    let sleep_reason = detail.sleep_reason.as_deref().unwrap_or("—");
    let current_run = detail.current_run_id.as_deref().unwrap_or("—");
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(18.0))
        .py(px(10.0))
        .border_y_1()
        .border_color(colors.border)
        .child(metadata_item("Current run", current_run))
        .child(metadata_item("Next wake", next_wake))
        .child(metadata_item("Sleep reason", sleep_reason))
        .child(metadata_item(
            "Cadence",
            &format!("{}s · max {}", detail.tick_interval_secs, detail.max_ticks),
        ))
}

fn metadata_item(label: &str, value: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .max_w(px(250.0))
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label.to_owned()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_secondary)
                .overflow_hidden()
                .whitespace_nowrap()
                .child(value.to_owned()),
        )
}

fn live_task_row(index: u64, task: ProductHiveTask) -> impl IntoElement {
    let colors = theme::colors();
    let done = task.status == "completed";
    let failed = task.status == "failed" || task.status == "blocked";
    div()
        .id(("hive-task-row", index))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(8.0))
        .bg(colors.bg_sidebar)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(if failed {
                    colors.status_error
                } else if done {
                    colors.accent
                } else {
                    colors.status_ready
                }))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(task.subject),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if failed {
                            colors.status_error
                        } else {
                            colors.text_tertiary
                        })
                        .child(task.status),
                ),
        )
        .when(!task.description.is_empty(), |this| {
            this.child(
                div()
                    .pl(px(16.0))
                    .text_xs()
                    .text_color(colors.text_secondary)
                    .child(task.description),
            )
        })
        .when_some(task.owner, |this, owner| {
            this.child(
                div()
                    .pl(px(16.0))
                    .text_xs()
                    .text_color(colors.text_tertiary)
                    .child(format!("owner · {owner}")),
            )
        })
        .when(!task.blocked_by.is_empty(), |this| {
            this.child(
                div()
                    .pl(px(16.0))
                    .text_xs()
                    .text_color(colors.status_error)
                    .child(format!("blocked by · {}", task.blocked_by.join(", "))),
            )
        })
        .when_some(task.result, |this, result| {
            this.child(
                div()
                    .pl(px(16.0))
                    .text_xs()
                    .text_color(colors.text_secondary)
                    .child(result),
            )
        })
}

fn dispatch_editor(
    editor: HiveDispatchEditorState,
    inputs: HiveWorkInputs,
    model: Option<String>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("hive-dispatch-editor")
        .flex()
        .flex_col()
        .gap(px(12.0))
        .px(px(24.0))
        .py(px(16.0))
        .bg(colors.bg_main)
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("New Hive run"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!(
                                    "Runs on the connected Mitsuro server · model {}",
                                    model.as_deref().unwrap_or("server default")
                                )),
                        ),
                )
                .child(
                    div()
                        .id("hive-dispatch-close")
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .when(!editor.submitting, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.text_color(colors.text))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.close_hive_dispatch_editor(cx);
                                }))
                        })
                        .child("Close"),
                ),
        )
        .child(work_input(
            "hive-dispatch-task",
            "Task",
            &inputs.task,
            true,
            true,
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(10.0))
                .child(work_input(
                    "hive-dispatch-workspace",
                    "Workspace",
                    &inputs.project_dir,
                    false,
                    false,
                ))
                .child(work_input(
                    "hive-dispatch-start",
                    "Start at · optional RFC3339",
                    &inputs.start_at,
                    false,
                    false,
                ))
                .child(work_input(
                    "hive-dispatch-crew",
                    "Crew · optional",
                    &inputs.crew_slug,
                    false,
                    false,
                )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_end()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("Priority"),
                        )
                        .child(priority_controls(
                            "hive-dispatch-priority",
                            editor.priority,
                            !editor.submitting,
                            true,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.0))
                        .child(
                            div()
                                .id("hive-dispatch-cancel")
                                .h(px(32.0))
                                .px(px(13.0))
                                .flex()
                                .items_center()
                                .rounded(px(8.0))
                                .bg(colors.bg_button_secondary)
                                .text_xs()
                                .text_color(colors.text_secondary)
                                .when(!editor.submitting, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(|style| style.bg(colors.bg_hover))
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.close_hive_dispatch_editor(cx);
                                        }))
                                })
                                .when(editor.submitting, |button| button.opacity(0.45))
                                .child("Cancel"),
                        )
                        .child(
                            div()
                                .id("hive-dispatch-submit")
                                .h(px(32.0))
                                .px(px(14.0))
                                .flex()
                                .items_center()
                                .rounded(px(8.0))
                                .bg(colors.bg_button_primary)
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.fg_button_primary)
                                .when(!editor.submitting, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(|style| style.bg(colors.bg_button_primary_hover))
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.submit_hive_dispatch(cx);
                                        }))
                                })
                                .when(editor.submitting, |button| button.opacity(0.6))
                                .child(if editor.submitting {
                                    "Dispatching…"
                                } else {
                                    "Dispatch run"
                                }),
                        ),
                ),
        )
}

fn priority_controls(
    id: &'static str,
    selected: ProductHivePriority,
    enabled: bool,
    dispatch_editor: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div().id(id).flex().flex_row().gap(px(5.0)).children(
        [
            ("Low", ProductHivePriority::Low),
            ("Normal", ProductHivePriority::Normal),
            ("High", ProductHivePriority::High),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, priority))| {
            div()
                .id((id, index as u64))
                .h(px(30.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .bg(if selected == priority {
                    colors.bg_selected
                } else {
                    colors.bg_button_secondary
                })
                .border_1()
                .border_color(if selected == priority {
                    colors.border_heavy
                } else {
                    colors.border
                })
                .text_xs()
                .text_color(if selected == priority {
                    colors.text
                } else {
                    colors.text_secondary
                })
                .when(enabled, |button| {
                    button
                        .cursor_pointer()
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if dispatch_editor {
                                app.set_hive_dispatch_priority(priority, cx);
                            } else {
                                app.set_selected_hive_priority(priority, cx);
                            }
                        }))
                })
                .when(!enabled, |button| button.opacity(0.45))
                .child(label)
        }),
    )
}

fn work_input(
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
                .h(if tall { px(72.0) } else { px(34.0) })
                .px(px(10.0))
                .rounded(px(8.0))
                .bg(colors.bg_sidebar)
                .border_1()
                .border_color(colors.border)
                .child(Input::new(input).appearance(false).h(if tall {
                    px(68.0)
                } else {
                    px(30.0)
                })),
        )
}

fn fixture_goal_detail(
    goal: &DemoGoal,
    state: SurfaceDataState,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let goal_id = goal.id.clone();
    let plan = goal.plan_items.clone();
    let updated = goal
        .updated_at
        .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
        .map(|time| format!("updated {}", time.format("%Y-%m-%d %H:%M UTC")))
        .unwrap_or_else(|| "local fixture".to_owned());
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .px(px(28.0))
        .py(px(20.0))
        .gap(px(16.0))
        .child(status_chip(goal.status.label(), goal.status))
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(goal.objective.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(updated),
        )
        .child(section_header(
            "Fixture plan tracker",
            &format!(
                "{}/{} complete",
                plan.iter().filter(|item| item.done).count(),
                plan.len()
            ),
        ))
        .child(
            div()
                .id("work-plan-list")
                .flex()
                .flex_col()
                .gap(px(6.0))
                .overflow_y_scroll()
                .children(
                    plan.into_iter()
                        .enumerate()
                        .map(|(index, item)| plan_row(goal_id.clone(), index as u64, item, cx)),
                ),
        )
        .child(
            codex_button::primary_with_icon(
                "work-clear-goal",
                "Clear fixture goal",
                Icon::new(IconName::Delete).with_size(px(12.0)),
                cx,
            )
            .rounded(px(8.0))
            .when(state == SurfaceDataState::Fixture, |button| {
                button.on_click(cx.listener(|app, _, _, cx| app.clear_selected_goal(cx)))
            }),
        )
}

fn plan_row(
    goal_id: String,
    index: u64,
    item: DemoPlanItem,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let item_id = item.id.clone();
    let done = item.done;
    div()
        .id(("work-plan-item", index))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(8.0))
        .bg(colors.bg_sidebar)
        .border_1()
        .border_color(colors.border)
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.toggle_goal_plan_item(&goal_id, &item_id, cx);
        }))
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .rounded(px(5.0))
                .border_1()
                .border_color(if done {
                    colors.accent
                } else {
                    colors.border_heavy
                })
                .bg(if done {
                    colors.accent_soft
                } else {
                    theme::transparent()
                })
                .flex()
                .items_center()
                .justify_center()
                .when(done, |this| {
                    this.child(
                        Icon::new(IconName::Check)
                            .with_size(px(12.0))
                            .text_color(colors.accent),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .text_sm()
                .text_color(if done {
                    colors.text_tertiary
                } else {
                    colors.text
                })
                .child(item.title),
        )
}

fn section_header(title: &str, detail: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text_secondary)
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(detail.to_owned()),
        )
}

fn status_dot(status: DemoGoalStatus) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .mt(px(4.0))
        .w(px(8.0))
        .h(px(8.0))
        .rounded_full()
        .bg(status_color(status, colors))
}

fn status_chip(label: &str, status: DemoGoalStatus) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .text_xs()
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(999.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .text_color(status_color(status, colors))
        .child(label.replace('_', " "))
}

fn status_color(status: DemoGoalStatus, colors: theme::CodexColors) -> gpui::Hsla {
    match status {
        DemoGoalStatus::Active => colors.status_ready,
        DemoGoalStatus::Paused => colors.status_connecting,
        DemoGoalStatus::Blocked => colors.status_error,
        DemoGoalStatus::Complete => colors.accent,
    }
}

fn progress_bar(done: usize, total: usize) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .mt(px(8.0))
        .h(px(3.0))
        .w_full()
        .rounded(px(999.0))
        .bg(theme::hex_alpha(0xffffff, 0.06))
        .child(
            div()
                .h(px(3.0))
                .w(px(if total == 0 {
                    0.0
                } else {
                    (done as f32 / total as f32) * 220.0
                }))
                .rounded(px(999.0))
                .bg(colors.accent),
        )
}

fn priority_label(priority: ProductHivePriority) -> &'static str {
    match priority {
        ProductHivePriority::Low => "low",
        ProductHivePriority::Normal => "normal",
        ProductHivePriority::High => "high",
    }
}
