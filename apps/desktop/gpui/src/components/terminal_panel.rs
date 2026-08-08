//! Terminal / process panel — `process/spawn` surface (Codex dark theme).
//!
//! Offline path uses [`mitsuro_desktop_backend::FixtureBackend`] process mock.
//! Live path forwards to app-server `process/*` methods when Ready.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, TerminalSessionStatus};
use crate::theme;

/// Full-height Terminal panel: command bar + scrollable output + stdin + kill.
pub fn terminal_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let session = app.terminal_session();
    let cmd_input = app.terminal_cmd_input().clone();
    let stdin_input = app.terminal_stdin_input().clone();
    let running = session.running;
    let handle = session.process_handle.clone();
    let status = session.status;

    div()
        .id("terminal-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(terminal_title_bar(
            session.status_label(),
            session.backend_label.as_ref(),
        ))
        .child(command_bar(
            &cmd_input,
            running,
            app.terminal_interactive_available(),
            cx,
        ))
        .child(output_scroll(session.output.as_ref(), handle.as_deref()))
        .child(stdin_bar(app, &stdin_input, running, cx))
        .child(status_footer(status, handle.as_deref(), session.exit_code))
}

fn terminal_title_bar(status: &str, backend: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("terminal-title")
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
                    Icon::new(IconName::SquareTerminal)
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
                                .child("Terminal"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!("process/* · {backend}")),
                        ),
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
                        .text_xs()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(6.0))
                        .bg(colors.bg_elevated)
                        .border_1()
                        .border_color(colors.border)
                        .text_color(colors.text_secondary)
                        .child(status.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(6.0))
                        .bg(theme::hex_alpha(0xffffff, 0.04))
                        .text_color(colors.text_tertiary)
                        .child("bash -lc"),
                ),
        )
}

fn command_bar(
    cmd_input: &gpui::Entity<gpui_component::input::InputState>,
    running: bool,
    interactive: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("terminal-command-bar")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_tertiary)
                .child("$"),
        )
        .child(
            div()
                .id("terminal-cmd-input")
                .flex()
                .flex_1()
                .min_w_0()
                .h(px(34.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border_heavy)
                .text_sm()
                .text_color(colors.text)
                .child(Input::new(cmd_input).appearance(false).h(px(28.0))),
        )
        .child(
            div()
                .id("terminal-start")
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(34.0))
                .px(px(14.0))
                .rounded(px(10.0))
                .bg(if running {
                    colors.bg_button_secondary
                } else {
                    colors.accent_soft
                })
                .border_1()
                .border_color(colors.border)
                .when(!running && interactive, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.terminal_spawn(window, cx);
                        }))
                })
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if running {
                            colors.text_tertiary
                        } else {
                            colors.accent
                        })
                        .child(if running {
                            "Running"
                        } else if interactive {
                            "Start"
                        } else {
                            "Read-only"
                        }),
                ),
        )
        .child(
            div()
                .id("terminal-kill")
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(34.0))
                .px(px(14.0))
                .rounded(px(10.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .when(running, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.terminal_kill(window, cx);
                        }))
                })
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if running {
                            colors.status_error
                        } else {
                            colors.text_tertiary
                        })
                        .child("Kill"),
                ),
        )
}

fn output_scroll(output: &str, handle: Option<&str>) -> impl IntoElement {
    let colors = theme::colors();
    let empty = output.is_empty();
    div()
        .id("terminal-output")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .bg(colors.bg_under)
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                // Prompt / session line (finished-page density)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_xs()
                                .font_family("monospace")
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.accent)
                                .child("mitsuro@local"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("monospace")
                                .text_color(colors.text_tertiary)
                                .child("~/Work/Mitsuro"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("monospace")
                                .text_color(colors.text_secondary)
                                .child("$"),
                        )
                        .when_some(handle, |this, h| {
                            this.child(
                                div()
                                    .ml(px(8.0))
                                    .text_xs()
                                    .text_color(colors.text_tertiary)
                                    .child(format!("· {h}")),
                            )
                        }),
                )
                .child(if empty {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_sm()
                                .font_family("monospace")
                                .text_color(colors.text_tertiary)
                                .child("# Ready · process/spawn"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family("monospace")
                                .text_color(colors.text_tertiary)
                                .child(
                                    "# Type a command above and press Start. Stdin bar appears when running.",
                                ),
                        )
                        .into_any_element()
                } else {
                    div()
                        .id("terminal-output-body")
                        .text_sm()
                        .font_family("monospace")
                        .text_color(colors.text)
                        .child(output.to_string())
                        .into_any_element()
                }),
        )
}

fn stdin_bar(
    _app: &MitsuroApp,
    stdin_input: &gpui::Entity<gpui_component::input::InputState>,
    running: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("terminal-stdin-bar")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("stdin"),
        )
        .child(
            div()
                .id("terminal-stdin-input")
                .flex()
                .flex_1()
                .min_w_0()
                .h(px(34.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border_heavy)
                .text_sm()
                .text_color(colors.text)
                .child(Input::new(stdin_input).appearance(false).h(px(28.0))),
        )
        .child(
            div()
                .id("terminal-stdin-send")
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(34.0))
                .px(px(14.0))
                .rounded(px(10.0))
                .bg(colors.accent_soft)
                .border_1()
                .border_color(colors.border)
                .when(running, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.terminal_write_stdin(window, cx);
                        }))
                })
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if running {
                            colors.accent
                        } else {
                            colors.text_tertiary
                        })
                        .child("Send"),
                ),
        )
}

fn status_footer(
    status: TerminalSessionStatus,
    handle: Option<&str>,
    exit_code: Option<i32>,
) -> impl IntoElement {
    let colors = theme::colors();
    let detail = match (status, handle, exit_code) {
        (TerminalSessionStatus::Running, Some(h), _) => format!("running · {h}"),
        (TerminalSessionStatus::Exited, Some(h), Some(code)) => {
            format!("exited {code} · {h}")
        }
        (TerminalSessionStatus::Error, _, _) => "error".into(),
        _ => "idle · process/spawn".into(),
    };
    div()
        .id("terminal-status")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(16.0))
        .py(px(8.0))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(detail),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("Mitsuro · process/*"),
        )
}
