//! Native `/feedback` dialog backed by Codex `feedback/upload`.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{FeedbackCategory, MitsuroApp};
use crate::theme;

pub fn feedback_dialog(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let selected = app.feedback_category();
    let include_logs = app.feedback_include_logs();
    let uploading = app.feedback_upload_in_progress();
    let submit_enabled = app.feedback_submit_enabled(cx);
    let details_input = app.feedback_details_input().clone();
    let upload_error = app
        .status_line()
        .as_ref()
        .strip_prefix("Feedback could not be uploaded · ")
        .map(str::to_owned);

    div()
        .id("feedback-dialog-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::hex_alpha(0x000000, 0.66))
        .child(
            div()
                .id("feedback-dialog")
                .w(px(520.0))
                .max_w_full()
                .mx(px(24.0))
                .rounded(px(16.0))
                .border_1()
                .border_color(colors.border_heavy)
                .bg(colors.bg_elevated)
                .p(px(20.0))
                .flex()
                .flex_col()
                .gap(px(16.0))
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
                                .child("Send feedback"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.text_secondary)
                                .child("Tell us what worked or what got in your way."),
                        ),
                )
                .when_some(upload_error, |this, error| {
                    this.child(div().text_xs().text_color(colors.status_error).child(error))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(field_label("Category", "Required"))
                        .child(div().flex().flex_row().flex_wrap().gap(px(7.0)).children(
                            FeedbackCategory::ALL.into_iter().map(|category| {
                                category_button(category, selected == Some(category), uploading, cx)
                            }),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(field_label("Details", "Required"))
                        .child(
                            div()
                                .h(px(132.0))
                                .w_full()
                                .rounded(px(10.0))
                                .border_1()
                                .border_color(colors.border)
                                .bg(theme::hex_alpha(0xffffff, 0.025))
                                .px(px(11.0))
                                .py(px(8.0))
                                .text_sm()
                                .text_color(colors.text)
                                .child(Input::new(&details_input).appearance(false).h(px(112.0))),
                        ),
                )
                .child(
                    div()
                        .id("feedback-include-logs")
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(px(10.0))
                        .when(!uploading, |this| {
                            this.cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| app.toggle_feedback_logs(cx)))
                        })
                        .when(uploading, |this| this.opacity(0.6))
                        .child(
                            div()
                                .mt(px(1.0))
                                .w(px(18.0))
                                .h(px(18.0))
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(if include_logs {
                                    colors.accent
                                } else {
                                    colors.border_heavy
                                })
                                .bg(if include_logs {
                                    colors.accent
                                } else {
                                    theme::hex_alpha(0xffffff, 0.02)
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(include_logs, |this| {
                                    this.child(
                                        Icon::new(IconName::Check)
                                            .xsmall()
                                            .text_color(colors.fg_button_primary),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.text)
                                        .child("Include current Codex session logs"),
                                )
                                .child(div().text_xs().text_color(colors.text_tertiary).child(
                                    "Logs can contain prompts, responses, and tool activity.",
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.0))
                        .pt(px(2.0))
                        .child(
                            div()
                                .id("feedback-cancel")
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
                                .when(!uploading, |this| {
                                    this.cursor_pointer()
                                        .hover(|style| style.bg(colors.bg_hover))
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.close_feedback_dialog(cx);
                                        }))
                                })
                                .when(uploading, |this| this.opacity(0.5))
                                .child("Cancel"),
                        )
                        .child(
                            div()
                                .id("feedback-submit")
                                .h(px(34.0))
                                .px(px(15.0))
                                .rounded(px(8.0))
                                .bg(colors.accent)
                                .flex()
                                .items_center()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.fg_button_primary)
                                .when(submit_enabled, |this| {
                                    this.cursor_pointer()
                                        .hover(|style| style.opacity(0.9))
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.submit_feedback(cx);
                                        }))
                                })
                                .when(!submit_enabled, |this| this.opacity(0.45))
                                .child(if uploading { "Uploading…" } else { "Submit" }),
                        ),
                ),
        )
}

fn field_label(label: &'static str, requirement: &'static str) -> impl IntoElement {
    let colors = theme::colors();
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
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(requirement),
        )
}

fn category_button(
    category: FeedbackCategory,
    selected: bool,
    disabled: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(category.wire_value())
        .h(px(30.0))
        .px(px(11.0))
        .rounded(px(999.0))
        .border_1()
        .border_color(if selected {
            colors.accent
        } else {
            colors.border
        })
        .bg(if selected {
            colors.accent_soft
        } else {
            theme::hex_alpha(0xffffff, 0.02)
        })
        .flex()
        .items_center()
        .text_xs()
        .font_weight(if selected {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(if selected {
            colors.text
        } else {
            colors.text_secondary
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.select_feedback_category(category, cx);
                }))
        })
        .when(disabled, |this| this.opacity(0.6))
        .child(category.label())
}
