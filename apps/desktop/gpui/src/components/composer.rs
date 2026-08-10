//! Transport-neutral chat composer.
//!
//! Implemented controls are interactive: text entry, backend-scoped model and
//! explicit model and reasoning-effort pickers, real model-gated image and audio file
//! attachments, Codex skill and file-mention inputs, native project selection,
//! backend-specific access and response-speed controls, Send, and Stop.
//! Codex realtime voice uses the app-server contract and PipeWire; unsupported
//! backends do not render a microphone control.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{ComposerAttachmentKind, MitsuroApp};
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
    let reasoning = app
        .has_reasoning_effort_control()
        .then(|| app.reasoning_effort_label())
        .flatten();
    let input_entity = input.clone();
    let streaming = app.turn_in_progress();
    let steerable = app.can_steer_active_turn();
    let calm = app.is_calm_stage();
    let show_usage = app.usage_card_visible();
    let draft_empty =
        input.read(cx).value().trim().is_empty() && app.composer_attachments().is_empty();

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
                .when(app.composer_add_menu_open(), |this| {
                    this.child(composer_add_menu(app, cx))
                })
                .when(app.composer_access_menu_open(), |this| {
                    this.child(composer_access_menu(app, cx))
                })
                .when(app.composer_model_menu_open(), |this| {
                    this.child(composer_model_menu(app, cx))
                })
                .when(app.composer_reasoning_menu_open(), |this| {
                    this.child(composer_reasoning_menu(app, cx))
                })
                .when(!app.composer_attachments().is_empty(), |this| {
                    this.child(attachment_chips(app, cx))
                })
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
                        .when(app.can_open_composer_add_menu(), |this| {
                            this.child(round_action(
                                "composer-add-input",
                                IconName::Plus,
                                false,
                                cx,
                                |app, _, _, cx| app.toggle_composer_add_menu(cx),
                            ))
                        })
                        .when(app.can_attach_audio(), |this| {
                            this.child(round_path_action(
                                "composer-attach-audio",
                                "icons/audio-lines.svg",
                                cx,
                                |app, _, _, cx| app.select_composer_audio(cx),
                            ))
                        })
                        .when(app.realtime_voice_available(), |this| {
                            this.child(realtime_voice_action(
                                "composer-realtime-voice",
                                app.realtime_voice_active(),
                                cx,
                            ))
                        })
                        .when(app.show_composer_workspace_control(), |this| {
                            this.child(workspace_chip(app, cx))
                        })
                        .when(app.show_composer_access_control(), |this| {
                            this.child(access_chip(app, cx))
                        })
                        .when(app.work_mode_available(), |this| {
                            this.child(work_mode_chip(app.work_mode_label(), cx))
                        })
                        .child(model_chip(&model, cx))
                        .when_some(reasoning, |this, label| {
                            this.child(reasoning_chip(&label, cx))
                        })
                        .when(app.fast_mode_available(), |this| {
                            this.child(fast_chip(
                                &app.fast_mode_label(),
                                app.fast_mode_enabled(),
                                cx,
                            ))
                        })
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
    let reasoning = app
        .has_reasoning_effort_control()
        .then(|| app.reasoning_effort_label())
        .flatten();
    let input_entity = input.clone();
    let streaming = app.turn_in_progress();
    let steerable = app.can_steer_active_turn();
    let draft_empty =
        input.read(cx).value().trim().is_empty() && app.composer_attachments().is_empty();

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
        .when(!app.composer_attachments().is_empty(), |this| {
            this.child(attachment_chips(app, cx))
        })
        .when(app.composer_add_menu_open(), |this| {
            this.child(composer_add_menu(app, cx))
        })
        .when(app.composer_reasoning_menu_open(), |this| {
            this.child(composer_reasoning_menu(app, cx))
        })
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
                .when(app.can_open_composer_add_menu(), |this| {
                    this.child(round_action(
                        "chat-add-input",
                        IconName::Plus,
                        false,
                        cx,
                        |app, _, _, cx| app.toggle_composer_add_menu(cx),
                    ))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(32.0))
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(input).appearance(false).h(px(30.0))),
                )
                .when_some(reasoning, |this, label| {
                    this.child(reasoning_chip(&label, cx))
                })
                .when(app.fast_mode_available(), |this| {
                    this.child(fast_chip(
                        &app.fast_mode_label(),
                        app.fast_mode_enabled(),
                        cx,
                    ))
                })
                .when(app.work_mode_available(), |this| {
                    this.child(work_mode_chip(app.work_mode_label(), cx))
                })
                .when(app.can_attach_audio(), |this| {
                    this.child(round_path_action(
                        "chat-attach-audio",
                        "icons/audio-lines.svg",
                        cx,
                        |app, _, _, cx| app.select_composer_audio(cx),
                    ))
                })
                .when(app.realtime_voice_available(), |this| {
                    this.child(realtime_voice_action(
                        "chat-realtime-voice",
                        app.realtime_voice_active(),
                        cx,
                    ))
                })
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
    let reasoning = app
        .has_reasoning_effort_control()
        .then(|| app.reasoning_effort_label())
        .flatten();
    let input_entity = input.clone();
    let streaming = app.turn_in_progress();
    let steerable = app.can_steer_active_turn();
    let draft_empty =
        input.read(cx).value().trim().is_empty() && app.composer_attachments().is_empty();

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
                .when(app.composer_add_menu_open(), |this| {
                    this.child(composer_add_menu(app, cx))
                })
                .when(app.composer_reasoning_menu_open(), |this| {
                    this.child(composer_reasoning_menu(app, cx))
                })
                .when(!app.composer_attachments().is_empty(), |this| {
                    this.child(attachment_chips(app, cx))
                })
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
                        .when(app.can_open_composer_add_menu(), |this| {
                            this.child(round_action(
                                "chat-thread-add-input",
                                IconName::Plus,
                                false,
                                cx,
                                |app, _, _, cx| app.toggle_composer_add_menu(cx),
                            ))
                        })
                        .when(app.can_attach_audio(), |this| {
                            this.child(round_path_action(
                                "chat-thread-attach-audio",
                                "icons/audio-lines.svg",
                                cx,
                                |app, _, _, cx| app.select_composer_audio(cx),
                            ))
                        })
                        .when(app.realtime_voice_available(), |this| {
                            this.child(realtime_voice_action(
                                "chat-thread-realtime-voice",
                                app.realtime_voice_active(),
                                cx,
                            ))
                        })
                        .when_some(reasoning, |this, label| {
                            this.child(reasoning_chip(&label, cx))
                        })
                        .when(app.fast_mode_available(), |this| {
                            this.child(fast_chip(
                                &app.fast_mode_label(),
                                app.fast_mode_enabled(),
                                cx,
                            ))
                        })
                        .when(app.work_mode_available(), |this| {
                            this.child(work_mode_chip(app.work_mode_label(), cx))
                        })
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

fn attachment_chips(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("composer-attachments")
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(6.0))
        .children(
            app.composer_attachments()
                .iter()
                .enumerate()
                .map(|(index, attachment)| {
                    let icon = match attachment.kind {
                        ComposerAttachmentKind::Image => "icons/gallery-vertical-end.svg",
                        ComposerAttachmentKind::Audio => "icons/audio-lines.svg",
                        ComposerAttachmentKind::Skill => "icons/puzzle.svg",
                        ComposerAttachmentKind::Mention => "icons/file.svg",
                    };
                    div()
                        .id(("composer-attachment", index))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .max_w(px(220.0))
                        .px(px(9.0))
                        .py(px(5.0))
                        .rounded(px(8.0))
                        .bg(colors.bg_button_secondary)
                        .border_1()
                        .border_color(colors.border)
                        .child(
                            Icon::empty()
                                .path(icon)
                                .with_size(px(13.0))
                                .text_color(colors.text_tertiary),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .text_xs()
                                .text_color(colors.text_secondary)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(attachment.name.clone()),
                        )
                        .child(
                            div()
                                .id(("remove-composer-attachment", index))
                                .cursor_pointer()
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.remove_composer_attachment(index, cx);
                                }))
                                .child(
                                    Icon::new(IconName::Close)
                                        .with_size(px(12.0))
                                        .text_color(colors.text_tertiary),
                                ),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>(),
        )
}

fn composer_add_menu(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let skills = app
        .enabled_composer_skills()
        .take(8)
        .map(|skill| {
            (
                skill.name.clone(),
                skill
                    .short_description
                    .clone()
                    .unwrap_or_else(|| skill.description.clone()),
            )
        })
        .collect::<Vec<_>>();
    div()
        .id("composer-add-menu")
        .w_full()
        .max_h(px(280.0))
        .overflow_y_scroll()
        .rounded(px(11.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .when(app.can_attach_images(), |this| {
            this.child(composer_add_action(
                "composer-add-image",
                "icons/gallery-vertical-end.svg",
                "Attach images",
                "PNG, JPEG, WebP, or GIF",
                cx,
                |app, _, _, cx| app.select_composer_images(cx),
            ))
        })
        .when(app.can_mention_files(), |this| {
            this.child(composer_add_action(
                "composer-add-mention",
                "icons/file.svg",
                "Mention files",
                "Add exact local file references",
                cx,
                |app, _, _, cx| app.select_composer_mention(cx),
            ))
        })
        .when(!skills.is_empty(), |this| {
            this.child(
                div()
                    .px(px(8.0))
                    .pt(px(7.0))
                    .pb(px(3.0))
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(colors.text_tertiary)
                    .child("Skills"),
            )
        })
        .children(
            skills
                .into_iter()
                .enumerate()
                .map(|(index, (name, detail))| {
                    let selected_name = name.clone();
                    div()
                        .id(("composer-add-skill", index))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(9.0))
                        .px(px(8.0))
                        .py(px(7.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.add_composer_skill(selected_name.clone(), cx);
                        }))
                        .child(
                            Icon::empty()
                                .path("icons/puzzle.svg")
                                .with_size(px(14.0))
                                .text_color(colors.text_tertiary),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.text_secondary)
                                        .child(name),
                                )
                                .when(!detail.trim().is_empty(), |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.text_tertiary)
                                            .overflow_hidden()
                                            .child(detail),
                                    )
                                }),
                        )
                        .into_any_element()
                }),
        )
}

fn composer_add_action(
    id: &'static str,
    icon: &'static str,
    title: &'static str,
    detail: &'static str,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &gpui::ClickEvent, &mut gpui::Window, &mut Context<MitsuroApp>)
        + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(9.0))
        .px(px(8.0))
        .py(px(7.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(on_click))
        .child(
            Icon::empty()
                .path(icon)
                .with_size(px(14.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(detail),
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

fn workspace_chip(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let label = app.composer_workspace_label();
    let enabled = app.can_select_composer_workspace();
    div()
        .id("workspace-chip")
        .flex()
        .flex_row()
        .items_center()
        .min_w_0()
        .max_w(px(154.0))
        .gap(px(5.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .text_color(if enabled {
            colors.text_tertiary
        } else {
            theme::hex_alpha(0xffffff, 0.32)
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, _, cx| app.select_composer_workspace(cx)))
        })
        .child(Icon::empty().path("icons/folder.svg").with_size(px(12.0)))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_xs()
                .child(label),
        )
}

fn access_chip(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let enabled = !app.turn_in_progress();
    div()
        .id("access-chip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .text_color(if enabled {
            colors.text_tertiary
        } else {
            theme::hex_alpha(0xffffff, 0.32)
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, _, cx| app.toggle_composer_access_menu(cx)))
        })
        .child(Icon::empty().path("icons/shield.svg").with_size(px(12.0)))
        .child(div().text_xs().child(app.composer_access_label()))
        .child(Icon::new(IconName::ChevronDown).with_size(px(11.0)))
}

fn composer_access_menu(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let choices = app.composer_access_choices();
    div()
        .id("composer-access-menu")
        .w_full()
        .rounded(px(11.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .px(px(8.0))
                .pt(px(5.0))
                .pb(px(4.0))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_tertiary)
                .child("Agent access"),
        )
        .children(
            choices
                .into_iter()
                .enumerate()
                .map(|(index, (mode, label, detail))| {
                    let selected = app.composer_access_mode_is(mode);
                    div()
                        .id(("composer-access-choice", index))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(9.0))
                        .px(px(8.0))
                        .py(px(7.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(colors.bg_hover))
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.select_composer_access_mode(mode, cx);
                        }))
                        .child(
                            div()
                                .w(px(14.0))
                                .text_xs()
                                .text_color(colors.text_secondary)
                                .child(if selected { "✓" } else { "" }),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(1.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.text_secondary)
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.text_tertiary)
                                        .child(detail),
                                ),
                        )
                        .into_any_element()
                }),
        )
}

fn composer_model_menu(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let search = app.composer_model_search_input().clone();
    let query = search.read(cx).value().to_string();
    let selected_id = app.selected_model_id().map(str::to_owned);
    let mut models = app
        .visible_composer_models(&query)
        .into_iter()
        .map(|model| {
            (
                model.id.clone(),
                model.label().to_owned(),
                model.description.clone(),
                model.is_default,
            )
        })
        .collect::<Vec<_>>();
    let match_count = models.len();
    models.truncate(60);
    let visible_count = models.len();

    div()
        .id("composer-model-menu")
        .w_full()
        .max_h(px(360.0))
        .rounded(px(11.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .h(px(34.0))
                .px(px(8.0))
                .rounded(px(8.0))
                .bg(colors.bg_button_secondary)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(&search).appearance(false).h(px(32.0))),
                ),
        )
        .child(
            div()
                .id("composer-model-results")
                .min_h_0()
                .max_h(px(300.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .when(models.is_empty(), |this| {
                    this.child(
                        div()
                            .px(px(9.0))
                            .py(px(12.0))
                            .text_xs()
                            .text_color(colors.text_tertiary)
                            .child("No models match this search."),
                    )
                })
                .children(models.into_iter().enumerate().map(
                    |(index, (id, label, description, is_default))| {
                        let selected = selected_id.as_deref() == Some(id.as_str());
                        let selected_model_id = id;
                        div()
                            .id(("composer-model-choice", index))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(9.0))
                            .px(px(8.0))
                            .py(px(7.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .when(selected, |this| this.bg(colors.bg_hover))
                            .hover(|style| style.bg(colors.bg_hover))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.select_model(selected_model_id.clone(), cx);
                            }))
                            .child(
                                div()
                                    .w(px(14.0))
                                    .text_xs()
                                    .text_color(colors.text_secondary)
                                    .child(if selected { "✓" } else { "" }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.0))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(6.0))
                                            .text_xs()
                                            .text_color(colors.text_secondary)
                                            .child(label)
                                            .when(is_default, |this| {
                                                this.child(
                                                    div()
                                                        .text_color(colors.text_tertiary)
                                                        .child("Default"),
                                                )
                                            }),
                                    )
                                    .when(!description.trim().is_empty(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(colors.text_tertiary)
                                                .overflow_hidden()
                                                .child(description),
                                        )
                                    }),
                            )
                            .into_any_element()
                    },
                ))
                .when(match_count > visible_count, |this| {
                    this.child(
                        div()
                            .px(px(9.0))
                            .py(px(8.0))
                            .text_xs()
                            .text_color(colors.text_tertiary)
                            .child(format!(
                                "Showing {visible_count} of {match_count} matches · refine search",
                            )),
                    )
                }),
        )
}

fn composer_reasoning_menu(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let choices = app.composer_reasoning_choices();
    div()
        .id("composer-reasoning-menu")
        .w_full()
        .rounded(px(11.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .px(px(8.0))
                .pt(px(5.0))
                .pb(px(4.0))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_tertiary)
                .child("Reasoning effort"),
        )
        .children(
            choices
                .into_iter()
                .enumerate()
                .map(|(index, (effort, description))| {
                    let selected = app.selected_reasoning_effort_is(&effort);
                    let selected_effort = effort.clone();
                    let label = crate::app::reasoning_effort_display_name(&effort);
                    div()
                        .id(("composer-reasoning-choice", index))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(9.0))
                        .px(px(8.0))
                        .py(px(7.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(colors.bg_hover))
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.select_reasoning_effort(selected_effort.clone(), cx);
                        }))
                        .child(
                            div()
                                .w(px(14.0))
                                .text_xs()
                                .text_color(colors.text_secondary)
                                .child(if selected { "✓" } else { "" }),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(1.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.text_secondary)
                                        .child(label),
                                )
                                .when(!description.trim().is_empty(), |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.text_tertiary)
                                            .child(description),
                                    )
                                }),
                        )
                        .into_any_element()
                }),
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
        .on_click(cx.listener(|app, _, window, cx| {
            app.toggle_composer_model_menu(window, cx);
        }))
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

fn reasoning_chip(label: &str, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let label = label.to_string();
    div()
        .id("reasoning-chip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| {
            app.toggle_composer_reasoning_menu(cx);
        }))
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

fn fast_chip(label: &str, enabled: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let label = label.to_string();
    div()
        .id("fast-mode-chip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(enabled, |this| this.bg(colors.bg_hover))
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| app.toggle_fast_mode(cx)))
        .child(
            div()
                .text_xs()
                .text_color(if enabled {
                    colors.text
                } else {
                    colors.text_tertiary
                })
                .child(label),
        )
}

fn work_mode_chip(label: &str, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let label = label.to_string();
    div()
        .id("work-mode-chip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| app.toggle_work_mode(cx)))
        .child(
            Icon::empty()
                .path("icons/book-open.svg")
                .with_size(px(11.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
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

fn round_path_action(
    id: &'static str,
    icon_path: &'static str,
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
        .bg(colors.bg_button_primary)
        .hover(|style| style.bg(colors.bg_button_primary_hover))
        .on_click(cx.listener(on_click))
        .child(
            Icon::empty()
                .path(icon_path)
                .with_size(px(15.0))
                .text_color(colors.fg_button_primary),
        )
}

fn realtime_voice_action(
    id: &'static str,
    active: bool,
    cx: &mut Context<MitsuroApp>,
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
        .bg(if active {
            theme::hex_alpha(0xfa423e, 0.9)
        } else {
            colors.bg_button_primary
        })
        .hover(|style| {
            if active {
                style.bg(theme::hex_alpha(0xfa423e, 0.75))
            } else {
                style.bg(colors.bg_button_primary_hover)
            }
        })
        .on_click(cx.listener(|app, _, _, cx| app.toggle_realtime_voice(cx)))
        .child(
            Icon::empty()
                .path("icons/mic.svg")
                .with_size(px(15.0))
                .text_color(if active {
                    colors.text
                } else {
                    colors.fg_button_primary
                }),
        )
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
