//! Reference `/approve` surface for recent denied Codex auto-reviews.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};

use crate::app::{GuardianDeniedAction, MitsuroApp};
use crate::theme;

pub fn guardian_dialog(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let denials = app.selected_guardian_denials().to_vec();
    let approving = app.guardian_approval_in_progress().map(ToOwned::to_owned);
    let busy = approving.is_some();
    let approval_error = app
        .status_line()
        .as_ref()
        .strip_prefix("Could not record auto-review approval · ")
        .map(str::to_owned);

    div()
        .id("guardian-dialog-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::hex_alpha(0x000000, 0.66))
        .child(
            div()
                .id("guardian-dialog")
                .w(px(560.0))
                .max_w_full()
                .mx(px(24.0))
                .rounded(px(16.0))
                .border_1()
                .border_color(colors.border_heavy)
                .bg(colors.bg_elevated)
                .p(px(20.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("Approve a recent denial"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .line_height(gpui::relative(1.4))
                                .text_color(colors.text_secondary)
                                .child("Select an auto-review denial to approve one retry. The retry will still go through auto-review."),
                        ),
                )
                .child(
                    div()
                        .id("guardian-denial-list")
                        .flex()
                        .flex_col()
                        .border_t_1()
                        .border_color(colors.border)
                        .children(denials.into_iter().enumerate().map(|(index, denial)| {
                            denial_row(index, denial, approving.as_deref(), busy, cx)
                        })),
                )
                .when_some(approval_error, |this, error| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(colors.status_error)
                            .child(error),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .child(
                            div()
                                .id("guardian-cancel")
                                .h(px(34.0))
                                .px(px(14.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.bg_button_secondary)
                                .flex()
                                .items_center()
                                .text_sm()
                                .text_color(colors.text)
                                .when(!busy, |this| {
                                    this.cursor_pointer()
                                        .hover(|style| style.bg(colors.bg_hover))
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.close_guardian_dialog(cx);
                                        }))
                                })
                                .when(busy, |this| this.opacity(0.5))
                                .child("Cancel"),
                        ),
                ),
        )
}

fn denial_row(
    index: usize,
    denial: GuardianDeniedAction,
    approving: Option<&str>,
    busy: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let is_approving = approving == Some(denial.id.as_str());
    let review_id = denial.id.clone();
    div()
        .id(("guardian-denial", index))
        .min_h(px(62.0))
        .w_full()
        .py(px(10.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .border_b_1()
        .border_color(theme::hex_alpha(0xffffff, 0.055))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.0))
                .child(
                    div()
                        .w_full()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(denial.title),
                )
                .child(
                    div()
                        .w_full()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(denial.rationale.unwrap_or_else(|| {
                            "Auto-review did not include a rationale".to_owned()
                        })),
                ),
        )
        .child(
            div()
                .id(("guardian-approve", index))
                .h(px(30.0))
                .px(px(11.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(colors.border)
                .bg(colors.bg_button_secondary)
                .flex()
                .items_center()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .when(!busy, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.approve_guardian_denial(review_id.clone(), cx);
                        }))
                })
                .when(busy, |this| this.opacity(0.5))
                .child(if is_approving {
                    "Recording…"
                } else {
                    "Approve one retry"
                }),
        )
}
