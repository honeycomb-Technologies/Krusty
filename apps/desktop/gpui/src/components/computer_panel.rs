//! Computer-use / environment status panel.
//!
//! Data sources:
//! - Environment catalog — explicit fixtures only; live transports do not expose a list
//! - Status / info — `environment/status`, `environment/info`
//! - Optional — `collaborationMode/list`
//!
//! Native Computer Use stack (`cua_node`) is not ported; UI shows protocol-backed catalog.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _};
use mitsuro_desktop_backend::{
    CollaborationModeMask, EnvironmentInfoResponse, EnvironmentStatusKind,
    EnvironmentStatusResponse, EnvironmentSummary,
};

use crate::app::{MitsuroApp, SurfaceDataState};
use crate::theme;

/// Full-height Computer panel for the Computer rail item.
pub fn computer_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let envs = app.environments().to_vec();
    let selected = app.selected_environment_id().map(str::to_string);
    let status = app.environment_status_detail().cloned();
    let info = app.environment_info_detail().cloned();
    let modes = app.collaboration_modes().to_vec();
    let chip = app.connection().chip_label();
    let data_state = app.environments_state();
    let source_note = match data_state {
        SurfaceDataState::Fixture => "explicit fixture catalog",
        SurfaceDataState::Live => "live backend",
        SurfaceDataState::Loading => "loading",
        SurfaceDataState::Unsupported => "environment/list unsupported",
        SurfaceDataState::Error => "backend error",
    };
    let connected = envs.iter().filter(|e| e.is_connected()).count();
    let n = envs.len();

    div()
        .id("computer-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(title_bar(
            chip,
            n,
            connected,
            app.status_line().as_ref(),
            cx,
        ))
        .child(
            div()
                .id("computer-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .px(px(24.0))
                .py(px(20.0))
                .gap(px(16.0))
                .child(permissions_card(data_state))
                .child(section_header(
                    "Environments",
                    &format!("environment/status · {source_note}"),
                ))
                .child(env_list(&envs, selected.as_deref(), cx))
                .child(section_header(
                    "Status detail",
                    "environment/status + environment/info for selection",
                ))
                .child(status_detail_panel(
                    selected.as_deref(),
                    &envs,
                    status.as_ref(),
                    info.as_ref(),
                ))
                .child(section_header(
                    "Collaboration modes",
                    "collaborationMode/list · experimental presets",
                ))
                .child(collab_modes_section(&modes)),
        )
}

/// Permissions / safety card — finished-page density for Computer surface.
fn permissions_card(state: SurfaceDataState) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("computer-permissions")
        .flex()
        .flex_col()
        .gap(px(10.0))
        .px(px(14.0))
        .py(px(14.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    Icon::empty()
                        .path("icons/shield.svg")
                        .with_size(px(14.0))
                        .text_color(colors.accent_orange),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("Permissions"),
                ),
        )
        .child(div().text_xs().text_color(colors.text_secondary).child(match state {
            SurfaceDataState::Fixture => "Explicit fixture capabilities; no operating-system permission grant is implied.",
            _ => "Screen, pointer, keyboard, shell, and network grants are not reported by the connected backend.",
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .child(perm_chip("Screen", false))
                .child(perm_chip("Pointer", false))
                .child(perm_chip("Keyboard", false))
                .child(perm_chip("Shell", false))
                .child(perm_chip("Network", false)),
        )
}

fn perm_chip(label: &'static str, granted: bool) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(999.0))
        .bg(if granted {
            colors.accent_soft
        } else {
            colors.bg_sidebar
        })
        .border_1()
        .border_color(colors.border)
        .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(if granted {
            colors.status_ready
        } else {
            colors.status_offline
        }))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if granted {
                    colors.text
                } else {
                    colors.text_tertiary
                })
                .child(label),
        )
}

fn title_bar(
    chip: &str,
    env_count: usize,
    connected: usize,
    status: &str,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("computer-title-bar")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(20.0))
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
                    Icon::new(IconName::Building2)
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
                                .child("Computer"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!(
                                    "{env_count} env(s) · {connected} connected · {status}"
                                )),
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
                        .id("computer-refresh")
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .h(px(30.0))
                        .px(px(12.0))
                        .rounded(px(8.0))
                        .bg(colors.accent_soft)
                        .border_1()
                        .border_color(colors.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.refresh_environments(window, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.accent)
                                .child("Refresh"),
                        ),
                )
                .when(
                    chip != "Offline" && chip != "Demo" && chip != "Fixture",
                    |this| {
                        this.child(
                            div()
                                .px(px(10.0))
                                .py(px(4.0))
                                .rounded(px(999.0))
                                .bg(colors.bg_elevated)
                                .border_1()
                                .border_color(colors.border)
                                .text_xs()
                                .text_color(colors.text_secondary)
                                .child(chip.to_string()),
                        )
                    },
                ),
        )
}

fn section_header(title: &str, subtitle: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(subtitle.to_string()),
        )
}

fn env_list(
    envs: &[EnvironmentSummary],
    selected: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("computer-env-list")
        .flex()
        .flex_col()
        .gap(px(8.0))
        .children(if envs.is_empty() {
            vec![div()
                .px(px(14.0))
                .py(px(12.0))
                .rounded(px(12.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("No environments loaded.")
                .into_any_element()]
        } else {
            envs.iter()
                .enumerate()
                .map(|(i, e)| {
                    let is_sel = selected == Some(e.id.as_str());
                    env_row(i as u64, e, is_sel, cx).into_any_element()
                })
                .collect()
        })
}

fn env_row(
    index: u64,
    env: &EnvironmentSummary,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let id = env.id.clone();
    let name = env.name.clone();
    let kind = env.kind_label().to_string();
    let status = env.status_label().to_string();
    let desc = env.description.clone().unwrap_or_default();
    let connected = env.is_connected();
    let status_color = status_chip_color(env.status);

    div()
        .id(("computer-env-row", index))
        .flex()
        .flex_row()
        .items_start()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(12.0))
        .bg(if selected {
            colors.bg_selected
        } else {
            colors.bg_elevated
        })
        .border_1()
        .border_color(if selected {
            colors.border_heavy
        } else {
            colors.border
        })
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _window, cx| {
            app.select_environment(id.clone(), cx);
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            Icon::new(if connected {
                                IconName::CircleCheck
                            } else {
                                IconName::Building2
                            })
                            .with_size(px(14.0))
                            .text_color(if connected {
                                colors.status_ready
                            } else {
                                colors.text_tertiary
                            }),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .child(name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("monospace")
                                .text_color(colors.text_tertiary)
                                .child(env.id.clone()),
                        ),
                )
                .when(!desc.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(colors.text_secondary)
                            .child(desc),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!("{kind} · {}", env.id)),
                ),
        )
        .child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(colors.bg_sidebar)
                .border_1()
                .border_color(colors.border)
                .text_xs()
                .text_color(status_color)
                .child(status),
        )
}

fn status_chip_color(status: EnvironmentStatusKind) -> gpui::Hsla {
    let colors = theme::colors();
    match status {
        EnvironmentStatusKind::Ready => colors.status_ready,
        EnvironmentStatusKind::Pending => colors.status_connecting,
        EnvironmentStatusKind::Disconnected => colors.status_offline,
        EnvironmentStatusKind::Unknown => colors.status_error,
    }
}

fn status_detail_panel(
    selected: Option<&str>,
    envs: &[EnvironmentSummary],
    status: Option<&EnvironmentStatusResponse>,
    info: Option<&EnvironmentInfoResponse>,
) -> impl IntoElement {
    let colors = theme::colors();
    let entry = selected.and_then(|id| envs.iter().find(|e| e.id == id));

    div()
        .id("computer-status-detail")
        .flex()
        .flex_col()
        .gap(px(10.0))
        .px(px(14.0))
        .py(px(14.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(match entry {
            None => div()
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("Select an environment to inspect status and shell info.")
                .into_any_element(),
            Some(e) => {
                let st = status.map(|s| s.status).unwrap_or(e.status);
                let err = status
                    .and_then(|s| s.error.clone())
                    .or_else(|| e.error.clone());
                let shell_name = info
                    .map(|i| i.shell.name.clone())
                    .or_else(|| e.shell.as_ref().map(|s| s.name.clone()))
                    .unwrap_or_else(|| "—".into());
                let shell_path = info
                    .map(|i| i.shell.path.clone())
                    .or_else(|| e.shell.as_ref().map(|s| s.path.clone()))
                    .unwrap_or_else(|| "—".into());
                let cwd = info
                    .and_then(|i| i.cwd.clone())
                    .or_else(|| e.cwd.clone())
                    .unwrap_or_else(|| "—".into());
                let url = e
                    .exec_server_url
                    .clone()
                    .unwrap_or_else(|| "— (local)".into());

                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.text)
                            .child(format!("{} · {}", e.name, e.id)),
                    )
                    .child(detail_row(
                        "Status",
                        st.status_label(),
                        status_chip_color(st),
                    ))
                    .child(detail_row("Kind", e.kind_label(), colors.text_secondary))
                    .child(detail_row(
                        "Shell",
                        &format!("{shell_name} ({shell_path})"),
                        colors.text_secondary,
                    ))
                    .child(detail_row("CWD", &cwd, colors.text_secondary))
                    .child(detail_row("Exec server", &url, colors.text_secondary))
                    .when_some(err, |this, message| {
                        this.child(detail_row("Error", &message, colors.status_error))
                    })
                    .into_any_element()
            }
        })
}

fn detail_row(label: &str, value: &str, value_color: gpui::Hsla) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(12.0))
        .child(
            div()
                .w(px(96.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .font_family("monospace")
                .text_color(value_color)
                .child(value.to_string()),
        )
}

fn collab_modes_section(modes: &[CollaborationModeMask]) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("computer-collab-modes")
        .flex()
        .flex_col()
        .gap(px(8.0))
        .children(if modes.is_empty() {
            vec![div()
                .px(px(14.0))
                .py(px(12.0))
                .rounded(px(12.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("No collaboration modes (collaborationMode/list empty).")
                .into_any_element()]
        } else {
            modes
                .iter()
                .enumerate()
                .map(|(i, m)| collab_mode_row(i as u64, m).into_any_element())
                .collect()
        })
}

fn collab_mode_row(index: u64, mode: &CollaborationModeMask) -> impl IntoElement {
    let colors = theme::colors();
    let kind = mode.mode.map(|m| m.as_str()).unwrap_or("—").to_string();
    let model = mode.model.clone().unwrap_or_else(|| "—".into());
    let effort = mode.reasoning_effort.clone().unwrap_or_else(|| "—".into());

    div()
        .id(("computer-collab-row", index))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(10.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(mode.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!("mode {kind} · model {model} · effort {effort}")),
                ),
        )
        .child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(colors.bg_sidebar)
                .border_1()
                .border_color(colors.border)
                .text_xs()
                .text_color(colors.text_secondary)
                .child(kind),
        )
}
