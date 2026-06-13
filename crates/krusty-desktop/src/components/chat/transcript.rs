use gpui::{div, IntoElement, ParentElement as _, Styled as _};

use crate::components::chat::blocks::{render_block, TranscriptBlock};
use crate::components::chat::message_bubble::{message_bubble, MessageRole};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptItem {
    User(String),
    Assistant { content: String, streaming: bool },
    System(String),
    Block(TranscriptBlock),
}

pub fn transcript_view(items: &[TranscriptItem]) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(items.iter().map(render_transcript_item))
}

fn render_transcript_item(item: &TranscriptItem) -> gpui::AnyElement {
    match item {
        TranscriptItem::User(content) => {
            message_bubble(MessageRole::User, content, false).into_any_element()
        }
        TranscriptItem::Assistant { content, streaming } => {
            message_bubble(MessageRole::Assistant, content, *streaming).into_any_element()
        }
        TranscriptItem::System(content) => {
            message_bubble(MessageRole::System, content, false).into_any_element()
        }
        TranscriptItem::Block(block) => render_block(block),
    }
}
