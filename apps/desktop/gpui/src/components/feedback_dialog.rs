//! Native `/feedback` dialog backed by Codex `feedback/upload`.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{FeedbackCategory, MitsuroApp};
use crate::components::ui_button::{self, ButtonSize, ButtonState, ButtonTone};
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
        .bg(colors.overlay_scrim)
        .child(
            div()
                .id("feedback-dialog")
                .w(px(520.0))
                .max_w_full()
                .mx(px(theme::spacing().xxl))
                .rounded(px(theme::shape().radius_lg))
                .border_1()
                .border_color(colors.border_heavy)
                .bg(colors.bg_elevated)
                .p(px(theme::spacing().xl))
                .flex()
                .flex_col()
                .gap(px(theme::spacing().xl))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(theme::typography().heading))
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
                        .gap(px(theme::spacing().md))
                        .child(field_label("Category", "Required"))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap(px(theme::spacing().sm))
                                .children(FeedbackCategory::ALL.into_iter().map(|category| {
                                    category_button(
                                        category,
                                        selected == Some(category),
                                        uploading,
                                        cx,
                                    )
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(theme::spacing().md))
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
                        .gap(px(theme::spacing().md))
                        .pt(px(2.0))
                        .child(
                            ui_button::button(
                                "feedback-cancel",
                                "Cancel",
                                ButtonTone::Secondary,
                                ButtonSize::Medium,
                                ButtonState {
                                    disabled: uploading,
                                    ..ButtonState::default()
                                },
                                cx,
                            )
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.close_feedback_dialog(cx);
                            })),
                        )
                        .child(
                            ui_button::button(
                                "feedback-submit",
                                "Submit",
                                ButtonTone::Primary,
                                ButtonSize::Medium,
                                ButtonState {
                                    disabled: !submit_enabled,
                                    loading: uploading,
                                    ..ButtonState::default()
                                },
                                cx,
                            )
                            .on_click(cx.listener(|app, _, _, cx| app.submit_feedback(cx))),
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
    ui_button::button(
        category.wire_value(),
        category.label(),
        ButtonTone::Subtle,
        ButtonSize::Small,
        ButtonState {
            selected,
            disabled,
            loading: false,
        },
        cx,
    )
    .rounded(px(theme::shape().radius_pill))
    .on_click(cx.listener(move |app, _, _, cx| {
        app.select_feedback_category(category, cx);
    }))
}
