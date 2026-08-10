//! Transport-neutral chat composer.
//!
//! Implemented controls are interactive: text entry, backend-scoped model cycling,
//! Send, and Stop. Voice, attachments, speed presets, and project selection are
//! deliberately absent until their backend contracts exist.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::MitsuroApp;
use crate::theme;

pub fn composer(
    app: &MitsuroApp,
    input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let chat_mode = app.active_mode() == crate::app::ProductMode::Chat;
    let calm = app.is_calm_stage();
    // Chat empty home: slim single-row composer (no promo / project stack).
    if chat_mode && (calm || app.is_empty_conversation()) {
        return chat_slim_composer(app, input, cx).into_any_element();
    }
    // Chat open thread: still slim-ish, but full-width max 720 with Instant + send.
    if chat_mode {
        return chat_thread_composer(app, input, cx).into_any_element();
    }
    codex_composer(app, input, cx).into_any_element()
}

/// Codex / agent composer: promo stack on calm home, full chrome when thread open.
fn codex_composer(
    app: &MitsuroApp,
    input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let model = app.model_label().to_string();
    let input_entity = input.clone();
    let streaming = app.turn_in_progress();
    let steerable = app.can_steer_active_turn();
    let calm = app.is_calm_stage();
    let show_usage = app.usage_card_visible();
    let draft_empty = input.read(cx).value().trim().is_empty();

    div()
        .id("composer-wrap")
        .w_full()
        .max_w(px(720.0))
        .px(if calm { px(24.0) } else { px(16.0) })
        .pb(if calm { px(24.0) } else { px(16.0) })
        .pt(px(4.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .when(show_usage, |this| this.child(usage_card(cx)))
        // Composer shell — full chrome on open thread
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(16.0))
                .bg(if calm {
                    theme::hex_alpha(0x1a1a1a, 0.78)
                } else {
                    colors.bg_elevated
                })
                .border_1()
                .border_color(if calm {
                    colors.border_subtle
                } else {
                    colors.border
                })
                .px(px(14.0))
                .pt(px(12.0))
                .pb(px(10.0))
                .gap(px(10.0))
                // Multi-line input
                .child(
                    div()
                        .min_h(if calm { px(40.0) } else { px(52.0) })
                        .max_h(px(160.0))
                        .w_full()
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(input).appearance(false).h(if calm {
                            px(44.0)
                        } else {
                            px(64.0)
                        })),
                )
                // Bottom toolbar: current model | send/stop.
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(model_chip(&model, cx))
                        .child(div().flex_1())
                        .child(if streaming {
                            streaming_actions(
                                "composer-stop",
                                "composer-steer",
                                input,
                                draft_empty,
                                steerable,
                                cx,
                            )
                            .into_any_element()
                        } else if !draft_empty {
                            round_action(
                                "composer-send",
                                IconName::ArrowUp,
                                false,
                                cx,
                                move |app, _, window, cx| {
                                    app.submit_composer(&input_entity, window, cx);
                                },
                            )
                            .into_any_element()
                        } else {
                            disabled_send("composer-send-disabled").into_any_element()
                        }),
                ),
        )
}

/// ChatGPT-mode home: slim centered row — + | Message… | Instant | mic | send.
fn chat_slim_composer(
    app: &MitsuroApp,
    input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let input_entity = input.clone();
    let streaming = app.turn_in_progress();
    let steerable = app.can_steer_active_turn();
    let draft_empty = input.read(cx).value().trim().is_empty();

    div()
        .id("composer-wrap-chat")
        .w_full()
        .max_w(px(640.0))
        .px(px(24.0))
        .pb(px(28.0))
        .pt(px(8.0))
        .flex()
        .flex_col()
        .items_center()
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(999.0))
                .bg(theme::hex_alpha(0x1a1a1a, 0.85))
                .border_1()
                .border_color(colors.border_subtle)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(32.0))
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(input).appearance(false).h(px(30.0))),
                )
                .child(if streaming {
                    streaming_actions("chat-stop", "chat-steer", input, draft_empty, steerable, cx)
                        .into_any_element()
                } else if !draft_empty {
                    round_action(
                        "chat-send",
                        IconName::ArrowUp,
                        false,
                        cx,
                        move |app, _, window, cx| {
                            app.submit_composer(&input_entity, window, cx);
                        },
                    )
                    .into_any_element()
                } else {
                    disabled_send("chat-send-disabled").into_any_element()
                }),
        )
}

/// Chat mode with an open conversation: compact shell, Instant + mic + send.
fn chat_thread_composer(
    app: &MitsuroApp,
    input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let input_entity = input.clone();
    let streaming = app.turn_in_progress();
    let steerable = app.can_steer_active_turn();
    let draft_empty = input.read(cx).value().trim().is_empty();

    div()
        .id("composer-wrap-chat-thread")
        .w_full()
        .max_w(px(720.0))
        .px(px(16.0))
        .pb(px(16.0))
        .pt(px(4.0))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(16.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .px(px(12.0))
                .pt(px(10.0))
                .pb(px(8.0))
                .gap(px(8.0))
                .child(
                    div()
                        .min_h(px(40.0))
                        .max_h(px(140.0))
                        .w_full()
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(input).appearance(false).h(px(48.0))),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().flex_1())
                        .child(if streaming {
                            streaming_actions(
                                "chat-thread-stop",
                                "chat-thread-steer",
                                input,
                                draft_empty,
                                steerable,
                                cx,
                            )
                            .into_any_element()
                        } else if !draft_empty {
                            round_action(
                                "chat-thread-send",
                                IconName::ArrowUp,
                                false,
                                cx,
                                move |app, _, window, cx| {
                                    app.submit_composer(&input_entity, window, cx);
                                },
                            )
                            .into_any_element()
                        } else {
                            disabled_send("chat-thread-send-disabled").into_any_element()
                        }),
                ),
        )
}

fn streaming_actions(
    stop_id: &'static str,
    steer_id: &'static str,
    input: &Entity<InputState>,
    draft_empty: bool,
    steerable: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let steer_input = input.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .when(steerable && !draft_empty, |this| {
            this.child(round_action(
                steer_id,
                IconName::ArrowUp,
                false,
                cx,
                move |app, _, window, cx| {
                    app.submit_composer(&steer_input, window, cx);
                },
            ))
        })
        .child(round_action(
            stop_id,
            IconName::CircleX,
            true,
            cx,
            |app, _, _, cx| app.interrupt_turn(cx),
        ))
}

fn usage_card(_cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    // Bar: refresh glyph + real-style reset copy; no dismiss X (Voice keeps X).
    div()
        .id("usage-card")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(14.0))
        .bg(theme::hex_alpha(0xffffff, 0.04))
        .border_1()
        .border_color(colors.border_subtle)
        .child(
            Icon::empty()
                .path("icons/refresh-cw.svg")
                .with_size(px(16.0))
                .text_color(colors.text_tertiary),
        )
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
                        .child("Usage limit reached"),
                )
                .child(div().text_xs().text_color(colors.text_tertiary).child(
                    "Check account settings or wait for the current limit window to reset.",
                )),
        )
}

fn model_chip(label: &str, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let label = label.to_string();
    div()
        .id("model-chip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| app.cycle_model(cx)))
        .child(div().text_xs().text_color(colors.text_tertiary).child("✧"))
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(px(11.0))
                .text_color(colors.text_tertiary),
        )
}

fn round_action(
    id: &'static str,
    icon: IconName,
    danger: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .w(px(32.0))
        .h(px(32.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .bg(if danger {
            theme::hex_alpha(0xfa423e, 0.9)
        } else {
            colors.bg_button_primary
        })
        .hover(|s| {
            if danger {
                s.bg(theme::hex_alpha(0xfa423e, 0.75))
            } else {
                s.bg(colors.bg_button_primary_hover)
            }
        })
        .on_click(cx.listener(on_click))
        .child(Icon::new(icon).with_size(px(15.0)).text_color(if danger {
            colors.text
        } else {
            colors.fg_button_primary
        }))
}

fn disabled_send(id: &'static str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .w(px(32.0))
        .h(px(32.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::hex_alpha(0xffffff, 0.07))
        .border_1()
        .border_color(theme::hex_alpha(0xffffff, 0.08))
        .child(
            Icon::new(IconName::ArrowUp)
                .with_size(px(15.0))
                .text_color(colors.text_tertiary),
        )
}
