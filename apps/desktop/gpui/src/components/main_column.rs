//! Main chat column: empty-state centered greeting OR transcript scroll.
//! Composer + transcript share one centered max-width content rail (~720).
//!
//! Transcript blocks (Codex-like): user / assistant bubbles, muted reasoning,
//! plan list surface, command `$ cmd` + output, file-change patch preview.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, relative, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::InputState;
use gpui_component::{Icon, Sizable as _};
use mitsuro_desktop_backend::{
    DelegationGroupStatus, DelegationTaskStatus, SessionDelegationProjection,
};

use crate::app::{MitsuroApp, ProductMode};
use crate::components::{approval_bar, composer, markdown};
use crate::demo::{DemoMessage, DemoMessageKind, ThreadSurface};
use crate::theme;

/// Shared content width for transcript + composer (Codex chat density).
const CONTENT_MAX_W: f32 = 720.0;

/// History grows in deliberate pages instead of laying out an entire long session.
const TRANSCRIPT_PAGE_SIZE: usize = 16;
/// Protect text layout from pathological tool payloads while preserving normal replies.
const DISPLAY_BODY_CAP: usize = 4_000;

pub fn main_column(
    app: &MitsuroApp,
    composer_input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let calm =
        matches!(app.active_mode(), ProductMode::Chat | ProductMode::Codex) && app.is_calm_stage();

    div()
        .id("main-column")
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        // Soft ambient wash (dark blue → near-black) — product polish, not OpenAI bloom IP.
        .bg(theme::ambient_main_bg())
        // Multi soft radial-like layers via overlapping linear gradients.
        .when(calm, |this| this.child(ambient_atmosphere_layers()))
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(match app.active_mode() {
                    ProductMode::Atlas => {
                        crate::components::browser_panel(app, cx).into_any_element()
                    }
                    ProductMode::Terminal => {
                        crate::components::terminal_panel(app, cx).into_any_element()
                    }
                    ProductMode::Files => {
                        crate::components::files_panel(app, cx).into_any_element()
                    }
                    ProductMode::Computer => {
                        crate::components::computer_panel(app, cx).into_any_element()
                    }
                    ProductMode::Extensions => {
                        crate::components::extensions_panel(app, cx).into_any_element()
                    }
                    ProductMode::Settings => {
                        crate::components::settings_panel(app, cx).into_any_element()
                    }
                    ProductMode::Work => crate::components::work_panel(app, cx).into_any_element(),
                    ProductMode::PullRequests => {
                        crate::components::pull_requests_panel(app, cx).into_any_element()
                    }
                    ProductMode::Sites => {
                        crate::components::sites_panel(app, cx).into_any_element()
                    }
                    ProductMode::Scheduled => {
                        crate::components::scheduled_panel(app, cx).into_any_element()
                    }
                    ProductMode::Chat | ProductMode::Codex => thread_main(app, composer_input, cx),
                }),
        )
}

/// Soft multi-blob atmosphere (overlapping gradient quads). Mitsuro palette only —
/// not a clone of OpenAI bloom / logo IP.
fn ambient_atmosphere_layers() -> impl IntoElement {
    div()
        .id("ambient-atmosphere")
        .absolute()
        .inset_0()
        .overflow_hidden()
        // Upper-left cool pool
        .child(
            div()
                .absolute()
                .top(px(-80.0))
                .left(px(-120.0))
                .w(relative(0.72))
                .h(relative(0.58))
                .bg(theme::ambient_wash_cool()),
        )
        // Lower-left warm ember
        .child(
            div()
                .absolute()
                .bottom(px(-60.0))
                .left(px(-40.0))
                .w(relative(0.55))
                .h(relative(0.5))
                .bg(theme::ambient_wash_warm()),
        )
        // Upper-right teal
        .child(
            div()
                .absolute()
                .top(px(-40.0))
                .right(px(-100.0))
                .w(relative(0.6))
                .h(relative(0.55))
                .bg(theme::ambient_wash_teal()),
        )
        // Soft bottom vignette so hero stays readable
        .child(
            div()
                .absolute()
                .bottom(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .h(relative(0.55))
                .bg(theme::ambient_wash_vignette()),
        )
}

fn thread_main(
    app: &MitsuroApp,
    composer_input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> gpui::AnyElement {
    let surface = app.active_thread_surface();
    let chat_mode = app.active_mode() == ProductMode::Chat;
    let thread = app.selected_thread();
    let transcript_thread_id = thread
        .as_ref()
        .map(|thread| thread.summary.id.as_str())
        .unwrap_or("unselected");
    let default_title = match surface {
        ThreadSurface::Chat => "New chat",
        ThreadSurface::Codex => "New thread",
    };
    let title = thread
        .as_ref()
        .map(|t| t.summary.display_title())
        .unwrap_or_else(|| default_title.into());
    let project_path = thread.as_ref().and_then(|t| t.summary.cwd.clone());
    // Borrow only — never clone the full message Vec every frame (ANR on open-thread).
    let messages: &[DemoMessage] = thread
        .as_ref()
        .map(|t| t.messages.as_slice())
        .unwrap_or(&[]);
    let delegation = app.selected_delegation();
    let transcript_visible_limit = app.transcript_visible_limit();
    // Selected + empty messages → loading (thread/read), not calm hero.
    let loading_transcript = thread.is_some() && messages.is_empty() && delegation.is_none();
    let empty = thread.is_none();
    let calm = app.is_calm_stage();
    // Open-thread chrome whenever a recent is selected (not calm home).
    let show_title = thread.is_some();

    div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        // Bar: quiet panel-toggle pair pinned top-right of the main stage.
        .child(stage_panel_toggles(cx))
        // Chat home: Chat | Work segmented control top-center.
        .when(chat_mode && calm, |this| {
            this.child(chat_work_segment(ProductMode::Chat, cx))
        })
        // Open-thread header: title · optional path chip · overflow
        .when(show_title, |this| {
            this.child(thread_title_bar(&title, project_path.as_deref(), app, cx))
        })
        // Full-bleed stage on calm empty; constrained column once threads exist.
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .items_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .when(calm, |this| {
                            // Full-bleed stage; center hero + quiet composer column.
                            this.items_center()
                        })
                        .when(!calm, |this| this.max_w(px(CONTENT_MAX_W)))
                        .child(if loading_transcript {
                            loading_transcript_state().into_any_element()
                        } else if empty {
                            empty_state(surface, chat_mode, calm, composer_input, cx)
                                .into_any_element()
                        } else {
                            transcript(
                                messages,
                                surface,
                                delegation,
                                transcript_visible_limit,
                                transcript_thread_id,
                                app,
                                cx,
                            )
                            .into_any_element()
                        })
                        .when_some(app.pending_approval().cloned(), |this, pending| {
                            this.child(approval_bar::approval_bar(&pending, cx))
                        })
                        .child(composer::composer(app, composer_input, cx)),
                ),
        )
        .into_any_element()
}

/// Open-thread while `thread/read` fills the local cache.
fn loading_transcript_state() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("transcript-loading")
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_secondary)
                .child("Loading thread…"),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("thread/read · includeTurns"),
        )
}

/// Top-center Chat | Work control (ChatGPT-mode home density).
fn chat_work_segment(active: ProductMode, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("chat-work-segment")
        .absolute()
        .top(px(10.0))
        .left(px(0.0))
        .right(px(0.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .p(px(3.0))
                .rounded(px(999.0))
                .bg(theme::hex_alpha(0xffffff, 0.05))
                .border_1()
                .border_color(colors.border_subtle)
                .child(segment_btn(
                    "seg-chat",
                    "Chat",
                    active == ProductMode::Chat,
                    cx,
                    |app, window, cx| app.set_mode(ProductMode::Chat, window, cx),
                ))
                .child(segment_btn(
                    "seg-work",
                    "Work",
                    active == ProductMode::Work,
                    cx,
                    |app, window, cx| app.set_mode(ProductMode::Work, window, cx),
                )),
        )
}

fn segment_btn(
    id: &'static str,
    label: &'static str,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .px(px(14.0))
        .py(px(5.0))
        .rounded(px(999.0))
        .cursor_pointer()
        .bg(if selected {
            theme::hex_alpha(0xffffff, 0.10)
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| on_click(app, window, cx)))
        .child(
            div()
                .text_xs()
                .font_weight(if selected {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::MEDIUM
                })
                .text_color(if selected {
                    colors.text
                } else {
                    colors.text_tertiary
                })
                .child(label),
        )
}

/// Quiet top-right chrome: two panel-layout toggle icons (no-op stubs, bar density).
fn stage_panel_toggles(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("stage-panel-toggles")
        .absolute()
        .top(px(10.0))
        .right(px(14.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .child(stage_icon_btn(
            "stage-panel-left",
            "icons/panel-left.svg",
            "Panel layout · stub",
            colors,
            cx,
        ))
        .child(stage_icon_btn(
            "stage-panel-right",
            "icons/panel-right.svg",
            "Side panel · stub",
            colors,
            cx,
        ))
}

fn stage_icon_btn(
    id: &'static str,
    path: &'static str,
    status: &'static str,
    colors: theme::CodexColors,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
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
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_status_line(status, cx);
        }))
        .child(
            Icon::empty()
                .path(path)
                .with_size(px(15.0))
                .text_color(colors.text_tertiary),
        )
}

/// Open-thread header: title · optional project path chip · quiet status · overflow menu.
fn thread_title_bar(
    title: &str,
    project_path: Option<&str>,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let menu_open = app.thread_menu_open();
    let is_archived = app
        .selected_thread()
        .and_then(|t| t.summary.archived)
        .unwrap_or(false);
    let status = app.status_line().to_string();
    // Quiet lifecycle / read feedback only (avoid dumping full connection noise).
    let quiet_status = if status.starts_with("thread/")
        || status.starts_with("thread ·")
        || status.starts_with("Archive")
        || status.starts_with("Delete")
        || status.starts_with("Fork")
    {
        Some(status)
    } else {
        None
    };

    div()
        .id("thread-title-bar")
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(48.0))
        .px(px(20.0))
        .pr(px(56.0)) // room for absolute panel toggles
        .border_b_1()
        .border_color(colors.border_subtle)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(title.to_string()),
                )
                .when_some(project_path.map(str::to_string), |this, path| {
                    this.child(project_path_chip(&path, cx))
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .flex_shrink_0()
                .when_some(quiet_status, |this, line| {
                    this.child(
                        div()
                            .id("thread-status-quiet")
                            .max_w(px(200.0))
                            .text_xs()
                            .text_color(colors.text_tertiary)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(line),
                    )
                })
                .child(thread_overflow_menu(menu_open, is_archived, cx)),
        )
        .when(menu_open, |this| {
            this.child(thread_overflow_dropdown(is_archived, cx))
        })
}

fn project_path_chip(path: &str, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    // Shorten home-ish prefixes for density.
    let short = path
        .strip_prefix("~/")
        .or_else(|| path.strip_prefix("/home/"))
        .unwrap_or(path);
    let short = if short.len() > 36 {
        format!("…{}", &short[short.len().saturating_sub(34)..])
    } else {
        short.to_string()
    };
    let full = path.to_string();
    div()
        .id("thread-path-chip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(999.0))
        .bg(theme::hex_alpha(0xffffff, 0.05))
        .border_1()
        .border_color(colors.border_subtle)
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_status_line(format!("Project · {full}"), cx);
        }))
        .child(
            Icon::empty()
                .path("icons/folder.svg")
                .with_size(px(12.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(short),
        )
}

/// Subtle ⋯ control that toggles the lifecycle overflow (Archive / Fork / Delete).
fn thread_overflow_menu(
    menu_open: bool,
    _is_archived: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("thread-overflow")
        .w(px(30.0))
        .h(px(30.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .bg(if menu_open {
            theme::hex_alpha(0xffffff, 0.06)
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| {
            app.toggle_thread_menu(cx);
        }))
        .child(
            Icon::empty()
                .path("icons/ellipsis.svg")
                .with_size(px(16.0))
                .text_color(colors.text_tertiary),
        )
}

/// Dense dropdown under the ⋯ control — Codex-like lifecycle actions.
fn thread_overflow_dropdown(is_archived: bool, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let archive_label = if is_archived { "Unarchive" } else { "Archive" };
    div()
        .id("thread-overflow-menu")
        .absolute()
        .top(px(44.0))
        .right(px(56.0))
        .w(px(168.0))
        .rounded(px(10.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .p(px(4.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .child(thread_menu_item(
            "thread-menu-archive",
            archive_label,
            "icons/inbox.svg",
            false,
            cx,
            |app, cx| app.archive_selected_thread(cx),
        ))
        .child(thread_menu_item(
            "thread-menu-fork",
            "Fork",
            "icons/git-branch.svg",
            false,
            cx,
            |app, cx| app.fork_selected_thread(cx),
        ))
        .child(
            div()
                .h(px(1.0))
                .mx(px(6.0))
                .my(px(3.0))
                .bg(colors.border_subtle),
        )
        .child(thread_menu_item(
            "thread-menu-delete",
            "Delete",
            "icons/delete.svg",
            true,
            cx,
            |app, cx| app.delete_selected_thread(cx),
        ))
}

fn thread_menu_item(
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    destructive: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    let fg = if destructive {
        colors.status_error
    } else {
        colors.text
    };
    let icon_fg = if destructive {
        colors.status_error
    } else {
        colors.text_tertiary
    };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(7.0))
        .rounded(px(7.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| on_click(app, cx)))
        .child(
            Icon::empty()
                .path(icon)
                .with_size(px(14.0))
                .text_color(icon_fg),
        )
        .child(div().text_sm().text_color(fg).child(label))
}

/// Home empty state: Codex “What should we build?” vs Chat “Ready when you are.”
fn empty_state(
    surface: ThreadSurface,
    chat_mode: bool,
    calm: bool,
    composer_input: &Entity<InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let _ = (composer_input, cx, surface, calm);
    let colors = theme::colors();
    let headline = if chat_mode {
        "Ready when you are."
    } else {
        "What should we build?"
    };

    div()
        .flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .items_center()
        .justify_center()
        .px(px(32.0))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(if chat_mode { px(14.0) } else { px(18.0) })
                .max_w(px(560.0))
                // Soft cloud / blob Mitsuro mark (not OpenAI logo IP) — quieter on Chat home.
                .when(!chat_mode, |this| this.child(mitsuro_cloud_mark()))
                .when(chat_mode, |this| this.child(mitsuro_chat_mark()))
                .child(
                    div()
                        .text_3xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(headline),
                ),
        )
}

/// Minimal mark for Chat home (no Codex cloud stack).
fn mitsuro_chat_mark() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("mitsuro-chat-mark")
        .w(px(36.0))
        .h(px(36.0))
        .rounded_full()
        .bg(theme::hex_alpha(0xffffff, 0.05))
        .border_1()
        .border_color(colors.border_subtle)
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::empty()
                .path("icons/pen-line.svg")
                .with_size(px(16.0))
                .text_color(colors.text_tertiary),
        )
}

/// Soft layered cloud silhouette for home hero (product mark, not OpenAI bloom).
fn mitsuro_cloud_mark() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("mitsuro-cloud")
        .relative()
        .w(px(56.0))
        .h(px(40.0))
        .flex()
        .items_center()
        .justify_center()
        // Base soft plate
        .child(
            div()
                .absolute()
                .bottom(px(4.0))
                .left(px(4.0))
                .w(px(48.0))
                .h(px(22.0))
                .rounded(px(14.0))
                .bg(theme::hex_alpha(0xffffff, 0.06))
                .border_1()
                .border_color(colors.border_subtle),
        )
        // Left puff
        .child(
            div()
                .absolute()
                .bottom(px(12.0))
                .left(px(6.0))
                .w(px(22.0))
                .h(px(22.0))
                .rounded_full()
                .bg(theme::hex_alpha(0xffffff, 0.05))
                .border_1()
                .border_color(colors.border_subtle),
        )
        // Center tall puff
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .left(px(16.0))
                .w(px(26.0))
                .h(px(26.0))
                .rounded_full()
                .bg(theme::hex_alpha(0xffffff, 0.07))
                .border_1()
                .border_color(colors.border_subtle),
        )
        // Right puff
        .child(
            div()
                .absolute()
                .bottom(px(10.0))
                .right(px(4.0))
                .w(px(20.0))
                .h(px(20.0))
                .rounded_full()
                .bg(theme::hex_alpha(0xffffff, 0.05))
                .border_1()
                .border_color(colors.border_subtle),
        )
}

fn transcript(
    messages: &[DemoMessage],
    surface: ThreadSurface,
    delegation: Option<&SessionDelegationProjection>,
    visible_limit: usize,
    thread_id: &str,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    // Chat mode uses a simpler bubble layout (no tool/reasoning chrome emphasis).
    let simple_bubbles = surface == ThreadSurface::Chat;
    let visible_range = transcript_tail_range(messages.len(), visible_limit);
    let hidden_count = visible_range.start;
    let first_visible_index = visible_range.start;
    let visible = &messages[visible_range];
    div()
        .id("transcript")
        .flex()
        .flex_col()
        .flex_1()
        .h_0()
        .min_h_0()
        .px(px(20.0))
        .pt(px(18.0))
        .pb(px(12.0))
        // Codex open-thread: slightly airier between blocks; chat stays denser.
        .gap(if simple_bubbles { px(12.0) } else { px(14.0) })
        .overflow_y_scroll()
        .track_scroll(app.transcript_scroll_handle())
        .when(hidden_count > 0, |this| {
            this.child(show_earlier_button(hidden_count, messages.len(), cx))
        })
        .children(visible.iter().enumerate().map(|(i, msg)| {
            let absolute_index = first_visible_index + i;
            let message_identity = msg
                .item_id
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| absolute_index.to_string());
            let message_key = format!("{thread_id}:{message_identity}");
            let expanded = app.transcript_message_is_expanded(&message_key);
            transcript_block(
                absolute_index as u64,
                msg,
                simple_bubbles,
                message_key,
                expanded,
                cx,
            )
        }))
        .when_some(delegation, |this, projection| {
            this.child(delegation_block(projection))
        })
}

fn transcript_tail_range(total: usize, visible_limit: usize) -> std::ops::Range<usize> {
    let visible = visible_limit.max(TRANSCRIPT_PAGE_SIZE).min(total);
    total.saturating_sub(visible)..total
}

fn show_earlier_button(
    hidden_count: usize,
    total_messages: usize,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let next_count = hidden_count.min(TRANSCRIPT_PAGE_SIZE);
    div()
        .id(("transcript-show-earlier", hidden_count))
        .flex()
        .items_center()
        .justify_center()
        .pb(px(2.0))
        .child(
            div()
                .id(("transcript-show-earlier-button", hidden_count))
                .px(px(10.0))
                .py(px(5.0))
                .rounded(px(7.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover).text_color(colors.text_secondary))
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.show_earlier_transcript_messages(total_messages, cx);
                }))
                .child(format!("Show {next_count} earlier · {hidden_count} hidden")),
        )
}

fn delegation_block(projection: &SessionDelegationProjection) -> impl IntoElement {
    let colors = theme::colors();
    let (active_groups, active_tasks) = projection.active_counts();
    div()
        .id("delegation-transcript-block")
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(10.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_elevated)
        .px(px(12.0))
        .py(px(10.0))
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_secondary)
                        .child("Delegation"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!(
                            "{active_groups} active groups · {active_tasks} active tasks"
                        )),
                ),
        )
        .children(
            projection
                .groups
                .iter()
                .enumerate()
                .map(|(group_index, group)| {
                    let group_status_color = delegation_group_status_color(group.status);
                    div()
                        .id(("delegation-group", group_index))
                        .flex()
                        .flex_col()
                        .w_full()
                        .gap(px(5.0))
                        .pt(px(6.0))
                        .border_t_1()
                        .border_color(colors.border_subtle)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.text_tertiary)
                                        .child(group.id.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(group_status_color)
                                        .child(group.status.label()),
                                ),
                        )
                        .children(group.tasks.iter().enumerate().map(|(task_index, task)| {
                            div()
                                .id(("delegation-task", group_index * 10_000 + task_index))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(10.0))
                                .pl(px(8.0))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.text)
                                        .child(task.key.clone()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(delegation_task_status_color(
                                                    task.status,
                                                ))
                                                .child(task.status.label()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(colors.text_tertiary)
                                                .child(format!("attempts {}", task.attempt_count)),
                                        ),
                                )
                        }))
                }),
        )
}

fn delegation_group_status_color(status: DelegationGroupStatus) -> gpui::Hsla {
    let colors = theme::colors();
    match status {
        DelegationGroupStatus::Complete => colors.status_ready,
        DelegationGroupStatus::Degraded => colors.accent_orange,
        DelegationGroupStatus::Failed => colors.status_error,
        DelegationGroupStatus::Cancelled => colors.status_offline,
        _ => colors.status_connecting,
    }
}

fn delegation_task_status_color(status: DelegationTaskStatus) -> gpui::Hsla {
    let colors = theme::colors();
    match status {
        DelegationTaskStatus::Complete => colors.status_ready,
        DelegationTaskStatus::Degraded => colors.accent_orange,
        DelegationTaskStatus::Failed => colors.status_error,
        DelegationTaskStatus::Cancelled => colors.status_offline,
        _ => colors.status_connecting,
    }
}

fn transcript_block(
    index: u64,
    msg: &DemoMessage,
    simple_bubbles: bool,
    message_key: String,
    expanded: bool,
    cx: &mut Context<MitsuroApp>,
) -> gpui::AnyElement {
    match &msg.kind {
        DemoMessageKind::User { body } => chat_bubble(
            index,
            "You",
            body,
            msg.streaming,
            true,
            simple_bubbles,
            message_key,
            expanded,
            cx,
        )
        .into_any_element(),
        DemoMessageKind::Assistant { body } => chat_bubble(
            index,
            "Mitsuro",
            body,
            msg.streaming,
            false,
            simple_bubbles,
            message_key,
            expanded,
            cx,
        )
        .into_any_element(),
        DemoMessageKind::Reasoning { body } => {
            if simple_bubbles {
                // Chat mode: fold reasoning into a muted one-liner instead of agent chrome.
                chat_bubble(
                    index,
                    "Thinking",
                    body,
                    msg.streaming,
                    false,
                    true,
                    message_key,
                    expanded,
                    cx,
                )
                .into_any_element()
            } else {
                reasoning_block(index, body, msg.streaming).into_any_element()
            }
        }
        DemoMessageKind::Plan { body } => {
            if simple_bubbles {
                chat_bubble(
                    index,
                    "Mitsuro",
                    body,
                    msg.streaming,
                    false,
                    true,
                    message_key,
                    expanded,
                    cx,
                )
                .into_any_element()
            } else {
                plan_block(index, body, msg.streaming).into_any_element()
            }
        }
        DemoMessageKind::CommandExecution {
            command,
            cwd,
            status,
            output,
        } => {
            if simple_bubbles {
                // One-liner only — avoid format! of full output every frame.
                chat_bubble(
                    index,
                    "Mitsuro",
                    command,
                    msg.streaming,
                    false,
                    true,
                    message_key,
                    expanded,
                    cx,
                )
                .into_any_element()
            } else {
                command_block(index, command, cwd, status, output, msg.streaming).into_any_element()
            }
        }
        DemoMessageKind::FileChange {
            paths_summary,
            patch_preview,
            status,
        } => {
            if simple_bubbles {
                chat_bubble(
                    index,
                    "Mitsuro",
                    paths_summary,
                    msg.streaming,
                    false,
                    true,
                    message_key,
                    expanded,
                    cx,
                )
                .into_any_element()
            } else {
                file_change_block(index, paths_summary, patch_preview, status, msg.streaming)
                    .into_any_element()
            }
        }
        DemoMessageKind::Error { body } => error_block(index, body).into_any_element(),
    }
}

fn error_block(index: u64, body: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(("msg-error", index))
        .flex()
        .items_start()
        .gap(px(9.0))
        .w_full()
        .py(px(7.0))
        .px(px(10.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(theme::hex_alpha(0xef4444, 0.34))
        .bg(theme::hex_alpha(0xef4444, 0.07))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.status_error)
                .child("Error"),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_sm()
                .text_color(colors.text_secondary)
                .child(body.to_owned()),
        )
}

/// Standard user / assistant text bubble.
///
/// The user is the elevated object; assistant content stays cardless in the workspace.
fn chat_bubble(
    index: u64,
    label: &str,
    body: &str,
    streaming: bool,
    is_user: bool,
    _simple: bool,
    message_key: String,
    expanded: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let label_color = colors.text_tertiary;
    let bubble_bg = if is_user {
        colors.bg_elevated
    } else {
        theme::transparent()
    };
    let (display, truncated) = display_body_light(body, streaming, expanded);

    div()
        .id(("msg", index))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .w_full()
        .when(is_user, |this| this.items_end())
        .when(!is_user, |this| this.items_start())
        .child(
            div()
                .rounded(px(14.0))
                .px(if is_user { px(12.0) } else { px(0.0) })
                .py(if is_user { px(8.0) } else { px(4.0) })
                .max_w(if is_user {
                    px(560.0)
                } else {
                    px(CONTENT_MAX_W)
                })
                .bg(bubble_bg)
                .when(is_user, |this| {
                    this.border_1().border_color(colors.border_subtle)
                })
                .flex()
                .flex_col()
                .gap(px(4.0))
                .when(!is_user, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(label_color)
                            .child(label.to_string()),
                    )
                })
                .child(if is_user {
                    div()
                        .text_sm()
                        .text_color(colors.text)
                        .child(display)
                        .into_any_element()
                } else {
                    markdown::markdown_body(index, &display).into_any_element()
                })
                .when(truncated || expanded, |this| {
                    let key = message_key.clone();
                    this.child(
                        div()
                            .id(("message-expand", index))
                            .mt(px(3.0))
                            .text_xs()
                            .text_color(colors.text_tertiary)
                            .cursor_pointer()
                            .hover(|style| style.text_color(colors.text_secondary))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.toggle_transcript_message_expanded(key.clone(), cx);
                            }))
                            .child(if expanded {
                                "Show less"
                            } else {
                                "Show full response"
                            }),
                    )
                }),
        )
}

/// Collapsible-looking muted reasoning / thinking block (smaller text).
fn reasoning_block(index: u64, body: &str, streaming: bool) -> impl IntoElement {
    let colors = theme::colors();
    let display = stream_body(body, streaming);
    let preview: String = display
        .lines()
        .next()
        .unwrap_or("Thinking…")
        .chars()
        .take(96)
        .collect();
    let multi = display.lines().count() > 1 || display.chars().count() > 96;
    let shown = if multi && !streaming {
        format!("▸ {preview}")
    } else if streaming && body.is_empty() {
        "▸ Thinking…".into()
    } else {
        format!("▾ {display}")
    };

    div()
        .id(("msg-reason", index))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .w_full()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_tertiary)
                .child("Reasoning"),
        )
        .child(
            div()
                .rounded(px(10.0))
                .px(px(12.0))
                .py(px(8.0))
                .bg(colors.bg_sidebar)
                .border_1()
                .border_color(colors.border_subtle)
                .text_xs()
                .text_color(colors.text_tertiary)
                .whitespace_normal()
                .child(shown),
        )
}

/// Numbered plan list surface.
fn plan_block(index: u64, body: &str, streaming: bool) -> impl IntoElement {
    let colors = theme::colors();
    let display = stream_body(body, streaming);
    let lines: Vec<String> = display
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    div()
        .id(("msg-plan", index))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .w_full()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.accent)
                .child("Plan"),
        )
        .child(
            div()
                .rounded(px(12.0))
                .px(px(14.0))
                .py(px(10.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(if lines.is_empty() {
                    vec![div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child(if streaming { "…" } else { "(empty plan)" })
                        .into_any_element()]
                } else {
                    lines
                        .into_iter()
                        .enumerate()
                        .map(|(i, line)| {
                            let text = strip_leading_number(&line);
                            div()
                                .id(("plan-step", index.saturating_mul(1000) + i as u64))
                                .flex()
                                .flex_row()
                                .items_start()
                                .gap(px(10.0))
                                .child(
                                    div()
                                        .min_w(px(20.0))
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.accent)
                                        .child(format!("{}.", i + 1)),
                                )
                                .child(div().text_sm().text_color(colors.text).child(text))
                                .into_any_element()
                        })
                        .collect()
                }),
        )
}

/// Command execution: monospace `$ cmd` header + elevated output box.
fn command_block(
    index: u64,
    command: &str,
    cwd: &str,
    status: &str,
    output: &str,
    streaming: bool,
) -> impl IntoElement {
    let colors = theme::colors();
    let cmd_display = if command.is_empty() {
        "$ …".to_string()
    } else {
        format!("$ {command}")
    };
    let status_label = status_chip_label(status, streaming);
    let out = stream_body(output, streaming);
    let show_out = !out.is_empty() || streaming;

    div()
        .id(("msg-cmd", index))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .w_full()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_tertiary)
                        .child("Command"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(status_color(status, colors))
                        .child(status_label),
                ),
        )
        .child(
            div()
                .rounded(px(12.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .overflow_hidden()
                .flex()
                .flex_col()
                // Header: $ command
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(8.0))
                        .bg(colors.bg_sidebar)
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_sm()
                                .font_family("monospace")
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .child(cmd_display),
                        )
                        .when(!cwd.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(colors.text_tertiary)
                                    .child(format!("cwd {cwd}")),
                            )
                        }),
                )
                .when(show_out, |this| {
                    this.child(
                        div()
                            .px(px(12.0))
                            .py(px(10.0))
                            .text_xs()
                            .font_family("monospace")
                            .text_color(colors.text_secondary)
                            .child(if out.is_empty() && streaming {
                                "Running…".to_string()
                            } else {
                                out
                            }),
                    )
                }),
        )
}

/// File change: paths summary + red/green-ish unified diff lines.
fn file_change_block(
    index: u64,
    paths_summary: &str,
    patch_preview: &str,
    status: &str,
    streaming: bool,
) -> impl IntoElement {
    let colors = theme::colors();
    let status_label = status_chip_label(status, streaming);
    let paths = if paths_summary.is_empty() {
        if streaming {
            "Preparing patch…".to_string()
        } else {
            "file change".to_string()
        }
    } else {
        paths_summary.to_string()
    };
    let patch = stream_body(patch_preview, streaming);
    let lines: Vec<String> = if patch.is_empty() {
        Vec::new()
    } else {
        patch.lines().map(|l| l.to_string()).collect()
    };

    div()
        .id(("msg-file", index))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .w_full()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_tertiary)
                        .child("File change"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(status_color(status, colors))
                        .child(status_label),
                ),
        )
        .child(
            div()
                .rounded(px(12.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(8.0))
                        .bg(colors.bg_sidebar)
                        .border_b_1()
                        .border_color(colors.border)
                        .text_sm()
                        .font_family("monospace")
                        .text_color(colors.text)
                        .child(paths),
                )
                .when(!lines.is_empty() || streaming, |this| {
                    this.child(
                        div()
                            .px(px(10.0))
                            .py(px(8.0))
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .children(if lines.is_empty() {
                                vec![div()
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(colors.text_tertiary)
                                    .child(if streaming { "…" } else { "(no diff)" })
                                    .into_any_element()]
                            } else {
                                lines
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, line)| {
                                        let color = diff_line_color(&line, colors);
                                        div()
                                            .id((
                                                "diff-line",
                                                index.saturating_mul(1000) + i as u64,
                                            ))
                                            .text_xs()
                                            .font_family("monospace")
                                            .text_color(color)
                                            .child(line)
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                    )
                }),
        )
}

fn stream_body(body: &str, streaming: bool) -> String {
    if streaming && body.is_empty() {
        "…".to_string()
    } else if streaming {
        format!("{body}▍")
    } else {
        body.to_string()
    }
}

/// Preserve message formatting while bounding pathological payloads.
fn display_body_light(body: &str, streaming: bool, expanded: bool) -> (String, bool) {
    if streaming && body.is_empty() {
        return ("…".into(), false);
    }
    const EXPANDED_BODY_CAP: usize = 32_000;
    let cap = if expanded {
        EXPANDED_BODY_CAP
    } else {
        DISPLAY_BODY_CAP
    };
    let body_chars = body.chars().count();
    let mut out = String::with_capacity(body.len().min(cap + 4));
    for (n, c) in body.chars().enumerate() {
        if n >= cap {
            out.push('…');
            break;
        }
        out.push(c);
    }
    if streaming {
        out.push('▍');
    }
    (
        out,
        body_chars > cap || (!expanded && body_chars > DISPLAY_BODY_CAP),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_starts_with_one_bounded_page() {
        assert_eq!(transcript_tail_range(279, 16), 263..279);
        assert_eq!(transcript_tail_range(8, 16), 0..8);
    }

    #[test]
    fn transcript_expansion_never_exceeds_available_history() {
        assert_eq!(transcript_tail_range(35, 32), 3..35);
        assert_eq!(transcript_tail_range(20, usize::MAX), 0..20);
    }

    #[test]
    fn long_message_preview_expands_without_unbounded_layout() {
        let body = "a".repeat(DISPLAY_BODY_CAP + 64);
        let (preview, truncated) = display_body_light(&body, false, false);
        assert!(truncated);
        assert_eq!(preview.chars().count(), DISPLAY_BODY_CAP + 1);

        let (expanded, still_truncated) = display_body_light(&body, false, true);
        assert!(!still_truncated);
        assert_eq!(expanded, body);
    }
}

fn strip_leading_number(line: &str) -> String {
    let trimmed = line.trim();
    // "1. foo" or "1) foo" or "1 foo"
    let mut chars = trimmed.chars().peekable();
    let mut saw_digit = false;
    while let Some(c) = chars.peek().copied() {
        if c.is_ascii_digit() {
            saw_digit = true;
            chars.next();
        } else {
            break;
        }
    }
    if !saw_digit {
        return trimmed.to_string();
    }
    match chars.peek().copied() {
        Some('.') | Some(')') => {
            chars.next();
            let rest: String = chars.collect();
            rest.trim_start().to_string()
        }
        Some(' ') => {
            let rest: String = chars.collect();
            rest.trim_start().to_string()
        }
        _ => trimmed.to_string(),
    }
}

fn status_chip_label(status: &str, streaming: bool) -> String {
    if streaming && (status.is_empty() || status == "inProgress") {
        "running".into()
    } else if status.is_empty() {
        "done".into()
    } else {
        match status {
            "inProgress" => "running".into(),
            "completed" => "done".into(),
            "failed" => "failed".into(),
            "declined" => "declined".into(),
            other => other.to_string(),
        }
    }
}

fn status_color(status: &str, colors: theme::CodexColors) -> gpui::Hsla {
    match status {
        "completed" => colors.status_ready,
        "failed" => colors.status_error,
        "declined" => colors.status_offline,
        "inProgress" | "" => colors.status_connecting,
        _ => colors.text_tertiary,
    }
}

fn diff_line_color(line: &str, colors: theme::CodexColors) -> gpui::Hsla {
    let t = line.trim_start();
    if t.starts_with("+++") || t.starts_with("---") || t.starts_with("@@") || t.starts_with("diff ")
    {
        colors.diff_meta
    } else if t.starts_with('+') {
        colors.diff_add
    } else if t.starts_with('-') {
        colors.diff_del
    } else {
        colors.text_secondary
    }
}
