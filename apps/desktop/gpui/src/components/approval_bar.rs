//! Codex-like approval strip: command / patch summary + Approve / Reject.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use mitsuro_desktop_backend::{ApprovalChoice, ApprovalKind, PendingApproval};

use crate::app::MitsuroApp;
use crate::theme;

/// Floating bar above the composer when a server approval request is pending.
pub fn approval_bar(pending: &PendingApproval, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let kind_label = match pending.kind {
        ApprovalKind::ExecCommand | ApprovalKind::CommandExecution => "Command",
        ApprovalKind::ApplyPatch | ApprovalKind::FileChange => "File change",
    };
    let title = pending.title.clone();
    let summary = pending.summary.clone();
    let detail = pending.detail.clone();
    let has_detail = !detail.trim().is_empty();

    div()
        .id("approval-bar")
        .w_full()
        .px(px(16.0))
        .pt(px(4.0))
        .pb(px(8.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .rounded(px(14.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.accent)
                .px(px(14.0))
                .py(px(12.0))
                // Header
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .px(px(8.0))
                                        .py(px(2.0))
                                        .rounded(px(999.0))
                                        .bg(colors.accent_soft)
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(colors.accent)
                                        .child(kind_label),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.text)
                                        .child(title),
                                ),
                        ),
                )
                // Command / path summary
                .child(
                    div()
                        .rounded(px(8.0))
                        .bg(colors.bg_sidebar)
                        .border_1()
                        .border_color(colors.border)
                        .px(px(10.0))
                        .py(px(8.0))
                        .text_sm()
                        .font_family("monospace")
                        .text_color(colors.text_secondary)
                        .child(summary),
                )
                // Optional detail (cwd / reason / diff summary)
                .when(has_detail, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(colors.text_tertiary)
                            .whitespace_normal()
                            .child(detail),
                    )
                })
                // Actions
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(reject_button(cx))
                        .child(approve_button(cx)),
                ),
        )
}

fn approve_button(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("approval-approve")
        .px(px(14.0))
        .py(px(6.0))
        .rounded(px(999.0))
        .bg(colors.bg_button_primary)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.fg_button_primary)
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_button_primary_hover))
        .on_click(cx.listener(|app, _, _window, cx| {
            app.resolve_pending_approval(ApprovalChoice::Approve, cx);
        }))
        .child("Approve")
}

fn reject_button(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("approval-reject")
        .px(px(14.0))
        .py(px(6.0))
        .rounded(px(999.0))
        .bg(colors.bg_button_secondary)
        .border_1()
        .border_color(colors.border_heavy)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text_secondary)
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _window, cx| {
            app.resolve_pending_approval(ApprovalChoice::Reject, cx);
        }))
        .child("Reject")
}
