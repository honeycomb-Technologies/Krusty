use gpui::{div, Context, Entity, IntoElement, ParentElement as _, Styled as _};
use gpui_component::input::InputState;

use crate::chat::session::{ChatSessionState, PermissionMode, ThinkingLevel, WorkMode};
use crate::components::button::{krusty_button, KrustyButtonKind};
use crate::components::chat::control_pill::{control_pill, control_pill_row};
use crate::components::input::krusty_input;
use crate::design::theme;
use crate::panels::chat::ChatPanel;

pub fn chat_composer(
    input: &Entity<InputState>,
    session: &ChatSessionState,
    model_label: &str,
    cx: &mut Context<ChatPanel>,
) -> impl IntoElement {
    let thinking_active = session.thinking_level != ThinkingLevel::Off;
    let permission_autonomous = session.permission_mode == PermissionMode::Autonomous;
    let model_label = model_label.to_owned();
    let input_entity = input.clone();

    div()
        .border_t_1()
        .border_color(theme::hairline())
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(control_pill_row(vec![
            control_pill(
                "pill-model",
                format!("Model: {model_label}"),
                false,
                true,
                cx,
            )
            .on_click(cx.listener(|panel, _, _, cx| panel.cycle_model(cx)))
            .into_any_element(),
            control_pill(
                "pill-thinking",
                format!("Think: {}", session.thinking_level.label()),
                thinking_active,
                thinking_active,
                cx,
            )
            .on_click(cx.listener(|panel, _, _, cx| panel.cycle_thinking(cx)))
            .into_any_element(),
            control_pill(
                "pill-permission",
                format!("Mode: {}", session.permission_mode.label()),
                permission_autonomous,
                permission_autonomous,
                cx,
            )
            .on_click(cx.listener(|panel, _, _, cx| panel.toggle_permission(cx)))
            .into_any_element(),
            control_pill(
                "pill-fast",
                if session.fast_mode {
                    "Fast: on".to_owned()
                } else {
                    "Fast: off".to_owned()
                },
                session.fast_mode,
                session.fast_mode,
                cx,
            )
            .on_click(cx.listener(|panel, _, _, cx| panel.toggle_fast_mode(cx)))
            .into_any_element(),
            control_pill(
                "pill-work-mode",
                session.work_mode.label().to_owned(),
                session.work_mode == WorkMode::Plan,
                session.work_mode == WorkMode::Plan,
                cx,
            )
            .on_click(cx.listener(|panel, _, _, cx| panel.toggle_work_mode(cx)))
            .into_any_element(),
        ]))
        .child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .border_1()
                        .border_color(thinking_border_color(session.thinking_level))
                        .child(krusty_input(input)),
                )
                .child(if session.is_streaming {
                    krusty_button("chat-stop", "Stop", KrustyButtonKind::Danger, cx)
                        .on_click(cx.listener(|panel, _, _, cx| panel.stop_stream(cx)))
                        .into_any_element()
                } else {
                    krusty_button("chat-send", "Send", KrustyButtonKind::Primary, cx)
                        .on_click(cx.listener(move |panel, _, window, cx| {
                            panel.submit(&input_entity, window, cx);
                        }))
                        .into_any_element()
                }),
        )
}

fn thinking_border_color(level: ThinkingLevel) -> gpui::Hsla {
    match level {
        ThinkingLevel::Off => theme::hairline(),
        ThinkingLevel::Minimal => theme::complement().opacity(0.25),
        ThinkingLevel::Low => theme::complement().opacity(0.35),
        ThinkingLevel::Medium => theme::complement().opacity(0.55),
        ThinkingLevel::High => theme::complement().opacity(0.75),
        ThinkingLevel::XHigh => theme::complement(),
        ThinkingLevel::Max | ThinkingLevel::Ultra => theme::complement(),
    }
}
