use gpui::prelude::FluentBuilder as _;
use gpui::{div, AnyElement, FontWeight, IntoElement, ParentElement as _, Styled as _};
use krusty_client_state::{
    ChatMessage, MessageRole, SystemNotice, ThinkingBlock, ToolBlock, ToolStatus, TranscriptNode,
};

use crate::markdown::markdown_view;
use crate::theme;

pub fn transcript_view(items: &[TranscriptNode]) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(items.iter().map(render_transcript_item))
}

fn render_transcript_item(item: &TranscriptNode) -> AnyElement {
    match item {
        TranscriptNode::Message(message) => message_bubble(message).into_any_element(),
        TranscriptNode::System(notice) => system_notice(notice).into_any_element(),
        TranscriptNode::Thinking(block) => thinking_block(block).into_any_element(),
        TranscriptNode::Tool(block) => tool_block(block).into_any_element(),
    }
}

fn message_bubble(message: &ChatMessage) -> impl IntoElement {
    let (label, border, bg) = match message.role {
        MessageRole::User => ("You", theme::accent(), theme::app_bg()),
        MessageRole::Assistant => ("Krusty", theme::hairline(), theme::surface_selected()),
    };

    div()
        .border_1()
        .border_color(border)
        .bg(bg)
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_muted())
                        .child(label),
                )
                .when(message.streaming, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme::complement())
                            .child("streaming"),
                    )
                }),
        )
        .when(!message.attachments.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(message.attachments.iter().map(|attachment| {
                        div()
                            .border_1()
                            .border_color(theme::hairline())
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child(attachment.name.clone())
                    })),
            )
        })
        .child(message_body(message))
}

fn message_body(message: &ChatMessage) -> AnyElement {
    if message.content.is_empty() && message.streaming && message.role == MessageRole::Assistant {
        return div()
            .text_sm()
            .text_color(theme::text_muted())
            .child("Streaming…")
            .into_any_element();
    }

    match message.role {
        MessageRole::Assistant => markdown_view(&message.content).into_any_element(),
        MessageRole::User => div()
            .text_sm()
            .text_color(theme::text())
            .child(message.content.clone())
            .into_any_element(),
    }
}

fn system_notice(notice: &SystemNotice) -> impl IntoElement {
    div()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::app_bg())
        .p_2()
        .text_xs()
        .text_color(theme::text_muted())
        .child(notice.content.clone())
}

fn thinking_block(block: &ThinkingBlock) -> impl IntoElement {
    div()
        .border_1()
        .border_color(theme::complement().opacity(0.5))
        .bg(theme::surface())
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::complement())
                .child(if block.streaming {
                    "Thinking…"
                } else {
                    "Thinking"
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme::text_muted())
                .child(block.content.clone()),
        )
}

fn tool_block(block: &ToolBlock) -> impl IntoElement {
    let (label, color) = match block.status {
        ToolStatus::Pending => ("pending", theme::text_muted()),
        ToolStatus::Running => ("running", theme::tool()),
        ToolStatus::Success => ("done", theme::success()),
        ToolStatus::Error => ("error", theme::danger()),
        ToolStatus::AwaitingApproval => ("approval", theme::complement()),
    };

    div()
        .border_1()
        .border_color(color.opacity(0.65))
        .bg(theme::surface())
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(block.name.clone()),
                )
                .child(div().text_xs().text_color(color).child(label)),
        )
        .when(!block.output.trim().is_empty(), |this| {
            this.child(
                div()
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::app_bg())
                    .p_2()
                    .text_xs()
                    .text_color(theme::text_muted())
                    .child(block.output.clone()),
            )
        })
}
