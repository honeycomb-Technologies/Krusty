//! Work mode — long-running goals / plans (fixture + optional thread/goal/* later).

use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::MitsuroApp;
use crate::components::codex_button;
use crate::demo::{DemoGoal, DemoGoalStatus, DemoPlanItem};
use crate::theme;

/// Full-height Work panel: goal list + plan items + empty CTA.
pub fn work_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let goals = app.goals().to_vec();
    let selected = app.selected_goal_id().map(str::to_string);
    let selected_goal = app.selected_goal().cloned();
    let empty = goals.is_empty();
    let live_hive = app.work_is_live_hive();
    let active = goals
        .iter()
        .filter(|g| g.status == DemoGoalStatus::Active)
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
            live_hive,
            app.status_line().as_ref(),
            cx,
        ))
        .child(if empty {
            work_empty_state(live_hive, cx).into_any_element()
        } else {
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .child(goal_list(&goals, selected.as_deref(), cx))
                .child(goal_detail(selected_goal.as_ref(), cx))
                .into_any_element()
        })
}

fn work_title_bar(
    count: usize,
    active: usize,
    live_hive: bool,
    status: &str,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
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
                        .child(div().text_xs().text_color(colors.text_tertiary).child(
                            if live_hive {
                                format!("{count} Hive run(s) · {active} active · read-only")
                            } else {
                                format!("{count} goal(s) · {active} active · long-running plans")
                            },
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .max_w(px(240.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(status.to_string()),
                )
                .child(
                    codex_button::primary_with_icon(
                        "work-header-create",
                        if live_hive { "Read-only" } else { "New goal" },
                        Icon::new(IconName::Plus).with_size(px(12.0)),
                        cx,
                    )
                    .rounded(px(8.0))
                    .on_click(cx.listener(|app, _, _, cx| app.start_new_goal(cx))),
                ),
        )
}

fn work_empty_state(live_hive: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
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
                .max_w(px(420.0))
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
                        .child(if live_hive {
                            "No Hive runs"
                        } else {
                            "No goals yet"
                        }),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .text_center()
                        .child(if live_hive {
                            "No Mitsuro Hive runs are currently available in this view. Dispatch remains intentionally unavailable from this client."
                        } else {
                            "Work mode tracks long-running goals with a plan tracker. Create a goal to get a checklist of steps; selecting a goal loads plan items and wires thread/goal/* offline."
                        }),
                )
                .child(
                    codex_button::primary_with_icon(
                        "start-goal",
                        if live_hive { "Read-only" } else { "Create goal" },
                        Icon::new(IconName::Plus).with_size(px(14.0)),
                        cx,
                    )
                    .rounded(px(10.0))
                    .on_click(cx.listener(|app, _, _, cx| app.start_new_goal(cx))),
                ),
        )
}

fn goal_list(
    goals: &[DemoGoal],
    selected: Option<&str>,
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
                        .child("Goals"),
                )
                .child(
                    codex_button::primary_with_icon(
                        "work-new-goal",
                        "Create",
                        Icon::new(IconName::Plus).with_size(px(12.0)),
                        cx,
                    )
                    .rounded(px(8.0))
                    .on_click(cx.listener(|app, _, _, cx| app.start_new_goal(cx))),
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
                .children(goals.iter().enumerate().map(|(i, goal)| {
                    let id = goal.id.clone();
                    let selected = selected == Some(goal.id.as_str());
                    let objective = goal.objective.clone();
                    let status = goal.status.label().to_string();
                    let done = goal.plan_items.iter().filter(|p| p.done).count();
                    let total = goal.plan_items.len();
                    div()
                        .id(("work-goal-row", i as u64))
                        .mx(px(8.0))
                        .px(px(12.0))
                        .py(px(12.0))
                        .rounded(px(12.0))
                        .cursor_pointer()
                        .bg(if selected {
                            colors.bg_selected
                        } else {
                            colors.bg_elevated
                        })
                        .border_1()
                        .border_color(if selected {
                            colors.border_heavy
                        } else {
                            colors.border
                        })
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.select_goal(id.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_start()
                                .gap(px(8.0))
                                .child(div().mt(px(2.0)).w(px(8.0)).h(px(8.0)).rounded_full().bg(
                                    match goal.status {
                                        DemoGoalStatus::Active => colors.status_ready,
                                        DemoGoalStatus::Paused => colors.status_connecting,
                                        DemoGoalStatus::Blocked => colors.status_error,
                                        DemoGoalStatus::Complete => colors.accent,
                                    },
                                ))
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
                                .child(status_chip(&status, goal.status))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.text_tertiary)
                                        .child(format!("{done}/{total} plan")),
                                ),
                        )
                        // Mini progress bar
                        .child(
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
                                ),
                        )
                })),
        )
}

fn goal_detail(goal: Option<&DemoGoal>, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
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
            Some(g) => goal_detail_body(g, cx).into_any_element(),
            None => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("Select a goal")
                .into_any_element(),
        })
}

fn goal_detail_body(goal: &DemoGoal, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let goal_id = goal.id.clone();
    let plan = goal.plan_items.clone();
    let linked = goal
        .thread_id
        .as_deref()
        .map(|t| format!("Linked thread · {t}"))
        .unwrap_or_else(|| "No linked thread".into());
    let updated = goal
        .updated_at
        .map(|t| format!("updated {t}"))
        .unwrap_or_else(|| "local".into());

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .px(px(28.0))
        .py(px(20.0))
        .gap(px(16.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(status_chip(goal.status.label(), goal.status))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(linked),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(updated),
                ),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(goal.objective.clone()),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text_secondary)
                        .child("Plan tracker"),
                )
                .child(div().text_xs().text_color(colors.text_tertiary).child({
                    let done = plan.iter().filter(|p| p.done).count();
                    let total = plan.len();
                    format!("{done}/{total} complete")
                })),
        )
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
                        .map(|(i, item)| plan_row(goal_id.clone(), i as u64, item, cx)),
                ),
        )
        .child(
            div()
                .mt(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    codex_button::primary_with_icon(
                        "work-clear-goal",
                        "Clear goal",
                        Icon::new(IconName::Delete).with_size(px(12.0)),
                        cx,
                    )
                    .rounded(px(8.0))
                    .on_click(cx.listener(|app, _, _, cx| app.clear_selected_goal(cx))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child("thread/goal/get · set · clear"),
                ),
        )
}

fn plan_row(
    goal_id: String,
    idx: u64,
    item: DemoPlanItem,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let item_id = item.id.clone();
    let done = item.done;
    div()
        .id(("work-plan-item", idx))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
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
                .child(if done {
                    Icon::new(IconName::Check)
                        .with_size(px(12.0))
                        .text_color(colors.accent)
                        .into_any_element()
                } else {
                    div().into_any_element()
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

fn status_chip(label: &str, status: DemoGoalStatus) -> impl IntoElement {
    let colors = theme::colors();
    let fg = match status {
        DemoGoalStatus::Active => colors.status_ready,
        DemoGoalStatus::Paused => colors.status_connecting,
        DemoGoalStatus::Blocked => colors.status_error,
        DemoGoalStatus::Complete => colors.accent,
    };
    div()
        .text_xs()
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(999.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .text_color(fg)
        .child(label.to_string())
}
