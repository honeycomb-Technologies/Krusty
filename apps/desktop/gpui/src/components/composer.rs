//! Bottom composer matching Codex desktop bar density.
//!
//! Home stack (above window edge, centered ~max 720):
//! - Optional dismissible promo cards (voice / usage)
//! - "Choose project" chip row
//! - Composer shell:
//!   - Multi-line placeholder ("Do anything" / …)
//!   - Toolbar: + | Full access (orange) | spacer | model | mic | voice/send (round)
//!
//! Empty / ready: trailing control is a quiet voice disc (audio-lines), not white ↑ send.
//! Non-empty: high-contrast primary send (↑). Streaming: stop.

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
    let calm = app.is_calm_stage();
    // Home stack only on calm empty stage — hide once a thread is open.
    let show_home_stack = calm;
    let show_voice = app.voice_promo_visible();
    let show_usage = app.usage_card_visible();
    // Empty draft → voice disc (bar empty-home); non-empty → send.
    // Open thread: prefer send when non-empty; voice disc when empty draft.
    let draft_empty = input.read(cx).value().trim().is_empty();
    let open_thread = !calm;

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
        // Dismissible promo cards (calm home only — never on open thread)
        .when(show_voice, |this| this.child(voice_promo_card(cx)))
        .when(show_usage, |this| this.child(usage_card(cx)))
        // Choose project chip (calm home only)
        .when(show_home_stack, |this| {
            this.child(
                div()
                    .id("choose-project")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(12.0))
                    .bg(theme::hex_alpha(0xffffff, 0.04))
                    .border_1()
                    .border_color(colors.border_subtle)
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.bg_hover))
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.set_status_line("Choose project · stub", cx);
                    }))
                    .child(
                        Icon::new(IconName::FolderOpen)
                            .with_size(px(14.0))
                            .text_color(colors.text_tertiary),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.text_secondary)
                            .child("Choose project"),
                    ),
            )
        })
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
                // Bottom toolbar: + | Full access | spacer | model | mic | send
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(toolbar_icon(
                            "composer-attach",
                            IconName::Plus,
                            cx,
                            |app, _, _, cx| {
                                app.set_status_line("Attach · stub", cx);
                            },
                        ))
                        .child(full_access_chip(cx))
                        .child(div().flex_1())
                        .child(model_chip(&model, cx))
                        .child(toolbar_icon_path(
                            "composer-mic",
                            "icons/mic.svg",
                            cx,
                            |app, _, _, cx| {
                                app.set_status_line("Voice input · stub", cx);
                            },
                        ))
                        .child(if streaming {
                            round_action(
                                "composer-stop",
                                IconName::CircleX,
                                true,
                                cx,
                                |app, _, _, cx| app.interrupt_turn(cx),
                            )
                            .into_any_element()
                        } else if !draft_empty || open_thread {
                            // Open thread: always show ↑ send affordance (full chrome).
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
                            quiet_voice_disc(cx).into_any_element()
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
                .child(toolbar_icon(
                    "chat-attach",
                    IconName::Plus,
                    cx,
                    |app, _, _, cx| {
                        app.set_status_line("Attach · stub", cx);
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(32.0))
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(input).appearance(false).h(px(30.0))),
                )
                .child(instant_chip(cx))
                .child(toolbar_icon_path(
                    "chat-mic",
                    "icons/mic.svg",
                    cx,
                    |app, _, _, cx| {
                        app.set_status_line("Voice input · stub", cx);
                    },
                ))
                .child(if streaming {
                    round_action("chat-stop", IconName::CircleX, true, cx, |app, _, _, cx| {
                        app.interrupt_turn(cx)
                    })
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
                    quiet_voice_disc(cx).into_any_element()
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
                        .child(toolbar_icon(
                            "chat-thread-attach",
                            IconName::Plus,
                            cx,
                            |app, _, _, cx| {
                                app.set_status_line("Attach · stub", cx);
                            },
                        ))
                        .child(div().flex_1())
                        .child(instant_chip(cx))
                        .child(toolbar_icon_path(
                            "chat-thread-mic",
                            "icons/mic.svg",
                            cx,
                            |app, _, _, cx| {
                                app.set_status_line("Voice input · stub", cx);
                            },
                        ))
                        .child(if streaming {
                            round_action(
                                "chat-thread-stop",
                                IconName::CircleX,
                                true,
                                cx,
                                |app, _, _, cx| app.interrupt_turn(cx),
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
                            quiet_voice_disc(cx).into_any_element()
                        }),
                ),
        )
}

fn instant_chip(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("instant-chip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| {
            app.set_status_line("Instant · speed preset · stub", cx);
        }))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_secondary)
                .child("Instant"),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(px(12.0))
                .text_color(colors.text_tertiary),
        )
}

fn voice_promo_card(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("voice-promo")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(14.0))
        .bg(theme::hex_alpha(0xffffff, 0.05))
        .border_1()
        .border_color(colors.border_subtle)
        .child(
            div()
                .w(px(36.0))
                .h(px(36.0))
                .rounded_full()
                .bg(theme::hex_alpha(0x7c9cff, 0.35))
                .border_1()
                .border_color(theme::hex_alpha(0xffffff, 0.08)),
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
                        .child("Try Mitsuro Voice"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child("Orchestrate tasks, connect tools, and explore code"),
                ),
        )
        .child(
            div()
                .id("voice-start")
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(999.0))
                .bg(theme::hex_alpha(0xffffff, 0.10))
                .cursor_pointer()
                .hover(|s| s.bg(theme::hex_alpha(0xffffff, 0.16)))
                .on_click(cx.listener(|app, _, _, cx| {
                    app.set_status_line("Voice · stub", cx);
                }))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child("Start Voice"),
                ),
        )
        .child(
            div()
                .id("voice-dismiss")
                .w(px(24.0))
                .h(px(24.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, _, cx| app.dismiss_voice_promo(cx)))
                .child(
                    Icon::new(IconName::Close)
                        .with_size(px(12.0))
                        .text_color(colors.text_tertiary),
                ),
        )
}

fn usage_card(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
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
                        .child("You're out of Codex and Work usage"),
                )
                .child(div().text_xs().text_color(colors.text_tertiary).child(
                    "Add credits to keep going now, or wait for usage to reset on Aug 7, 10:30 PM",
                )),
        )
        .child(
            div()
                .id("usage-credits")
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(999.0))
                .bg(theme::hex_alpha(0xffffff, 0.08))
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, _, cx| {
                    app.set_status_line("Add credits · stub", cx);
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .child("Add Credits"),
                ),
        )
}

fn full_access_chip(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("full-access")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, window, cx| {
            app.set_mode(crate::app::ProductMode::Settings, window, cx);
            app.set_status_line("Full access · see Settings", cx);
        }))
        .child(
            // Pointed shield mark (bar density) — not a plain hollow circle.
            Icon::empty()
                .path("icons/shield.svg")
                .with_size(px(14.0))
                .text_color(colors.accent_orange),
        )
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.accent_orange)
                .child("Full access"),
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
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, window, cx| app.open_model_picker(window, cx)))
        .child(
            // Small sparkle mark like bar model chip
            div().text_xs().text_color(colors.text_tertiary).child("✧"),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(px(12.0))
                .text_color(colors.text_tertiary),
        )
}

fn toolbar_icon(
    id: &'static str,
    icon: IconName,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(on_click))
        .child(
            Icon::new(icon)
                .with_size(px(15.0))
                .text_color(colors.text_tertiary),
        )
}

fn toolbar_icon_path(
    id: &'static str,
    path: &'static str,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(on_click))
        .child(
            Icon::empty()
                .path(path)
                .with_size(px(15.0))
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

/// Empty-home trailing control: quiet dark circular voice disc with audio-lines.
///
/// Critic/bar hierarchy: empty draft must **not** end in a high-contrast white ↑
/// send. Soft elevated disc + waveform reads as voice beside mic; promote to
/// primary white ↑ send only when the draft is non-empty.
fn quiet_voice_disc(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("composer-voice")
        .w(px(32.0))
        .h(px(32.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        // Quiet dark elevated disc (not primary white send).
        .bg(theme::hex_alpha(0xffffff, 0.10))
        .border_1()
        .border_color(theme::hex_alpha(0xffffff, 0.08))
        .hover(|s| s.bg(theme::hex_alpha(0xffffff, 0.16)))
        .on_click(cx.listener(|app, _, _, cx| {
            app.set_status_line("Voice mode · stub", cx);
        }))
        .child(
            Icon::empty()
                .path("icons/audio-lines.svg")
                .with_size(px(15.0))
                .text_color(colors.text_secondary),
        )
}
