//! Native interaction strip for Codex request_user_input and MCP elicitation.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::{Input, InputState};
use mitsuro_desktop_backend::{McpElicitationMode, PendingMcpElicitation, PendingUserInput};

use crate::app::MitsuroApp;
use crate::theme;

pub fn user_input_bar(
    pending: &PendingUserInput,
    index: usize,
    normal_input: &Entity<InputState>,
    secret_input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let count = pending.questions.len();
    let question = pending.questions.get(index).cloned();
    let Some(question) = question else {
        return interaction_shell(
            "empty-input-request",
            "Input requested".to_owned(),
            "The server did not include a question.".to_owned(),
            div(),
            action_button("Decline", false, cx, |app, window, cx| {
                app.decline_user_input(window, cx);
            }),
        )
        .into_any_element();
    };
    let options = question.options.clone().unwrap_or_default();
    let show_text = options.is_empty() || question.is_other;
    let input = if question.is_secret {
        secret_input.clone()
    } else {
        normal_input.clone()
    };

    interaction_shell(
        "input-request",
        format!("{} · {} of {}", question.header, index + 1, count),
        question.question,
        div()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .children(
                options
                    .into_iter()
                    .enumerate()
                    .map(|(option_index, option)| {
                        let answer = option.label.clone();
                        div()
                            .id(("request-option", option_index))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(colors.border)
                            .px(px(10.0))
                            .py(px(7.0))
                            .cursor_pointer()
                            .hover(|style| style.bg(colors.bg_hover))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.answer_user_input_option(answer.clone(), window, cx);
                            }))
                            .child(div().text_sm().text_color(colors.text).child(option.label))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors.text_tertiary)
                                    .child(option.description),
                            )
                    }),
            )
            .when(show_text, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(colors.border)
                        .px(px(9.0))
                        .py(px(5.0))
                        .child(
                            Input::new(&input)
                                .appearance(false)
                                .when(question.is_secret, |input| input.mask_toggle())
                                .h(px(28.0)),
                        )
                        .child(action_button("Submit", true, cx, |app, window, cx| {
                            app.submit_user_input_text(window, cx);
                        })),
                )
            }),
        action_button("Decline", false, cx, |app, window, cx| {
            app.decline_user_input(window, cx);
        }),
    )
    .into_any_element()
}

pub fn mcp_elicitation_bar(
    pending: &PendingMcpElicitation,
    index: usize,
    current_field: Option<(String, serde_json::Value, usize)>,
    input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = format!("{} · MCP request", pending.server_name);
    match &pending.mode {
        McpElicitationMode::Url { url, .. } => interaction_shell(
            "mcp-url-request",
            title,
            pending.message.clone(),
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(url.clone()),
            div()
                .flex()
                .gap(px(7.0))
                .child(action_button("Open link", false, cx, |app, window, cx| {
                    app.open_mcp_elicitation_url(window, cx);
                }))
                .child(action_button("Done", true, cx, |app, window, cx| {
                    app.accept_mcp_url_elicitation(window, cx);
                })),
        )
        .into_any_element(),
        McpElicitationMode::OpenAiForm { .. } => interaction_shell(
            "mcp-openai-form-request",
            title,
            pending.message.clone(),
            div().text_xs().text_color(colors.text_tertiary).child(
                "This server requested an OpenAI form extension the desktop did not advertise.",
            ),
            action_button("Decline", false, cx, |app, window, cx| {
                app.decline_mcp_elicitation(window, cx);
            }),
        )
        .into_any_element(),
        McpElicitationMode::Form { .. } => {
            let Some((name, schema, count)) = current_field else {
                return interaction_shell(
                    "mcp-empty-form-request",
                    title,
                    pending.message.clone(),
                    div().text_xs().child("No fields were requested."),
                    div()
                        .flex()
                        .gap(px(7.0))
                        .child(action_button("Decline", false, cx, |app, window, cx| {
                            app.decline_mcp_elicitation(window, cx);
                        }))
                        .child(action_button("Continue", true, cx, |app, window, cx| {
                            app.accept_empty_mcp_form(window, cx);
                        })),
                )
                .into_any_element();
            };
            let field_title = schema
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&name)
                .to_owned();
            let description = schema
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            let mut choices: Vec<(String, serde_json::Value)> = schema
                .get("enum")
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| {
                            value
                                .as_str()
                                .map(|label| (label.to_owned(), value.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if choices.is_empty() {
                choices = schema
                    .get("oneOf")
                    .and_then(serde_json::Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| {
                                let actual = value.get("const")?.clone();
                                let label = value
                                    .get("title")
                                    .and_then(serde_json::Value::as_str)
                                    .or_else(|| actual.as_str())?
                                    .to_owned();
                                Some((label, actual))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            if schema.get("type").and_then(serde_json::Value::as_str) == Some("boolean") {
                choices = vec![
                    ("Yes".to_owned(), serde_json::Value::Bool(true)),
                    ("No".to_owned(), serde_json::Value::Bool(false)),
                ];
            }
            let show_text = choices.is_empty();
            let input = input.clone();
            interaction_shell(
                "mcp-form-request",
                format!("{title} · field {} of {count}", index + 1),
                pending.message.clone(),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(colors.text)
                            .child(field_title),
                    )
                    .when(!description.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(description),
                        )
                    })
                    .children(choices.into_iter().enumerate().map(
                        |(choice_index, (label, value))| {
                            div()
                                .id(("mcp-form-choice", choice_index))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(colors.border)
                                .px(px(10.0))
                                .py(px(7.0))
                                .text_sm()
                                .text_color(colors.text)
                                .cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.answer_mcp_form_option(value.clone(), window, cx);
                                }))
                                .child(label)
                        },
                    ))
                    .when(show_text, |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(colors.border)
                                .px(px(9.0))
                                .py(px(5.0))
                                .child(Input::new(&input).appearance(false).h(px(28.0)))
                                .child(action_button("Next", true, cx, |app, window, cx| {
                                    app.submit_mcp_form_text(window, cx);
                                })),
                        )
                    }),
                action_button("Decline", false, cx, |app, window, cx| {
                    app.decline_mcp_elicitation(window, cx);
                }),
            )
            .into_any_element()
        }
    }
}

fn interaction_shell(
    id: &'static str,
    title: String,
    prompt: String,
    body: impl IntoElement,
    trailing: impl IntoElement,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .w_full()
        .px(px(16.0))
        .pt(px(4.0))
        .pb(px(8.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(9.0))
                .border_t_1()
                .border_color(colors.border_heavy)
                .pt(px(11.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .child(trailing),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_secondary)
                        .child(prompt),
                )
                .child(body),
        )
}

fn action_button(
    label: &'static str,
    primary: bool,
    cx: &mut Context<MitsuroApp>,
    handler: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(label)
        .px(px(11.0))
        .py(px(5.0))
        .rounded(px(999.0))
        .bg(if primary {
            colors.bg_button_primary
        } else {
            colors.bg_button_secondary
        })
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if primary {
            colors.fg_button_primary
        } else {
            colors.text_secondary
        })
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| handler(app, window, cx)))
        .child(label)
}
