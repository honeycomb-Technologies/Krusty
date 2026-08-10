//! Terminal / process panel — current `command/exec*` plus retained process inventory.
//!
//! Offline path uses [`mitsuro_desktop_backend::FixtureBackend`] process mock.
//! Live Codex commands use sandboxed `command/exec*`; Mitsuro exposes its real
//! tracked-process catalog without pretending to support interactive spawn.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};
use mitsuro_desktop_backend::{BackendKind, ProductProcess, ThreadBackgroundTerminal};

use crate::app::{MitsuroApp, SurfaceDataState, TerminalSessionStatus};
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
    let contract = app.terminal_contract_label();

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
            contract,
        ))
        .child(command_bar(
            &cmd_input,
            running,
            app.terminal_interactive_available(),
            cx,
        ))
        .child(background_processes_panel(app, cx))
        .child(output_scroll(
            session.output.as_ref(),
            handle.as_deref(),
            contract,
        ))
        .child(stdin_bar(app, &stdin_input, running, cx))
        .child(status_footer(
            status,
            handle.as_deref(),
            session.exit_code,
            contract,
        ))
}

fn background_processes_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> gpui::AnyElement {
    let colors = theme::colors();
    let kind = app.terminal_background_backend_kind();
    let mutating = app.background_process_mutation_in_progress();
    let codex_entries = app.thread_background_terminals();
    let mitsuro_entries = app.tracked_background_processes();
    let title = match kind {
        Some(BackendKind::CodexStdio) => app
            .terminal_background_thread_label()
            .map(|thread| format!("Thread background terminals · {thread}"))
            .unwrap_or_else(|| "Thread background terminals".to_owned()),
        Some(BackendKind::MitsuroHttp) => "Mitsuro tracked processes".to_owned(),
        _ => "Background processes".to_owned(),
    };
    let count = match kind {
        Some(BackendKind::CodexStdio) => codex_entries.len(),
        Some(BackendKind::MitsuroHttp) => mitsuro_entries.len(),
        _ => 0,
    };
    let can_clean = kind == Some(BackendKind::CodexStdio)
        && app.terminal_background_thread_label().is_some()
        && mutating.is_none();

    let mut list = div()
        .id("terminal-background-list")
        .flex()
        .flex_col()
        .max_h(px(190.0))
        .overflow_y_scroll();
    match kind {
        Some(BackendKind::CodexStdio) => {
            if app.thread_background_terminals_state() == SurfaceDataState::Loading {
                list = list.child(background_empty("Loading thread processes…"));
            } else if app.thread_background_terminals_state() == SurfaceDataState::Error {
                list = list.child(background_empty(
                    "The selected thread's process catalog could not be loaded.",
                ));
            } else if app.terminal_background_thread_label().is_none() {
                list = list.child(background_empty(
                    "Select a Codex thread, then return here to inspect its shell processes.",
                ));
            } else if codex_entries.is_empty() {
                list = list.child(background_empty(
                    "No background terminals are retained by this thread.",
                ));
            } else {
                for (index, terminal) in codex_entries.iter().enumerate() {
                    list = list.child(codex_background_row(index, terminal, mutating, cx));
                }
            }
        }
        Some(BackendKind::MitsuroHttp) => {
            if app.tracked_background_processes_state() == SurfaceDataState::Loading {
                list = list.child(background_empty("Loading Mitsuro tracked processes…"));
            } else if app.tracked_background_processes_state() == SurfaceDataState::Error {
                list = list.child(background_empty(
                    "The Mitsuro process catalog could not be loaded.",
                ));
            } else if mitsuro_entries.is_empty() {
                list = list.child(background_empty(
                    "No processes are currently tracked by the Mitsuro server.",
                ));
            } else {
                for (index, process) in mitsuro_entries.iter().enumerate() {
                    list = list.child(mitsuro_background_row(index, process, mutating, cx));
                }
            }
        }
        _ => {
            list = list.child(background_empty(
                "Connect a live backend to inspect background processes.",
            ));
        }
    }

    div()
        .id("terminal-background-processes")
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_main)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(14.0))
                .py(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text_secondary)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(count.to_string()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .when(kind == Some(BackendKind::CodexStdio), |this| {
                            this.child(
                                compact_action("terminal-background-clean", "Clean", can_clean)
                                    .when(can_clean, |button| {
                                        button.on_click(cx.listener(|app, _, _, cx| {
                                            app.clean_thread_background_terminals(cx);
                                        }))
                                    }),
                            )
                        })
                        .child(
                            compact_action(
                                "terminal-background-refresh",
                                "Refresh",
                                mutating.is_none(),
                            )
                            .when(mutating.is_none(), |button| {
                                button.on_click(cx.listener(|app, _, _, cx| {
                                    app.refresh_terminal_backgrounds(cx);
                                }))
                            }),
                        ),
                ),
        )
        .child(list)
        .into_any_element()
}

fn background_empty(message: &'static str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .px(px(14.0))
        .py(px(10.0))
        .text_xs()
        .text_color(colors.text_tertiary)
        .child(message)
}

fn compact_action(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let colors = theme::colors();
    div()
        .id(id)
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_button_secondary)
        .text_xs()
        .text_color(if enabled {
            colors.text_secondary
        } else {
            colors.text_tertiary
        })
        .child(label)
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
        })
}

fn codex_background_row(
    index: usize,
    terminal: &ThreadBackgroundTerminal,
    mutating: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let process_id = terminal.process_id.clone();
    let enabled = mutating.is_none();
    let pid = terminal
        .os_pid
        .map(|pid| format!("pid {pid}"))
        .unwrap_or_else(|| "pid unavailable".to_owned());
    let cpu = terminal
        .cpu_percent
        .map(|value| format!("{value:.1}% CPU"))
        .unwrap_or_else(|| "CPU unavailable".to_owned());
    let memory = terminal
        .rss_kb
        .map(format_memory_kb)
        .unwrap_or_else(|| "memory unavailable".to_owned());
    background_row_shell(
        ("terminal-background", index),
        terminal.command.clone(),
        format!("{} · {pid} · {cpu} · {memory}", terminal.cwd),
        compact_action(
            ("terminal-background-terminate", index),
            if mutating == Some(terminal.process_id.as_str()) {
                "Stopping…"
            } else {
                "Terminate"
            },
            enabled,
        )
        .when(enabled, |button| {
            button.on_click(cx.listener(move |app, _, _, cx| {
                app.terminate_background_process(process_id.clone(), cx);
            }))
        }),
        colors,
    )
}

fn mitsuro_background_row(
    index: usize,
    process: &ProductProcess,
    mutating: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let process_id = process.id.clone();
    let running =
        matches!(process.status.as_str(), "running" | "suspended") && process.pid.is_some();
    let enabled = running && mutating.is_none();
    let pid = process
        .pid
        .map(|pid| format!("pid {pid}"))
        .unwrap_or_else(|| "no pid".to_owned());
    background_row_shell(
        ("mitsuro-background", index),
        process.command.clone(),
        format!(
            "{} · {} · {pid} · {}s",
            process.working_dir, process.status, process.elapsed_secs
        ),
        compact_action(
            ("mitsuro-background-terminate", index),
            if mutating == Some(process.id.as_str()) {
                "Stopping…"
            } else if running {
                "Kill"
            } else {
                "Finished"
            },
            enabled,
        )
        .when(enabled, |button| {
            button.on_click(cx.listener(move |app, _, _, cx| {
                app.terminate_background_process(process_id.clone(), cx);
            }))
        }),
        colors,
    )
}

fn background_row_shell(
    id: impl Into<gpui::ElementId>,
    command: String,
    detail: String,
    action: gpui::Stateful<gpui::Div>,
    colors: theme::CodexColors,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(8.0))
        .border_t_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    div()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(colors.text)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(command),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(detail),
                ),
        )
        .child(action)
}

fn format_memory_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1} GiB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{:.1} MiB", kb as f64 / 1024.0)
    } else {
        format!("{kb} KiB")
    }
}

fn terminal_title_bar(status: &str, backend: &str, contract: &str) -> impl IntoElement {
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
                                .child(format!("{contract} · {backend}")),
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

fn output_scroll(output: &str, handle: Option<&str>, contract: &str) -> impl IntoElement {
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
                                .child(format!("# Ready · {contract}")),
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
    contract: &str,
) -> impl IntoElement {
    let colors = theme::colors();
    let detail = match (status, handle, exit_code) {
        (TerminalSessionStatus::Running, Some(h), _) => format!("running · {h}"),
        (TerminalSessionStatus::Exited, Some(h), Some(code)) => {
            format!("exited {code} · {h}")
        }
        (TerminalSessionStatus::Error, _, _) => "error".into(),
        _ => format!("idle · {contract}"),
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
                .child(format!("Mitsuro · {contract}")),
        )
}
