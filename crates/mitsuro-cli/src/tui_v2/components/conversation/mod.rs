//! Conversation-route components.

mod artifact_panel;
mod context_bar;
mod decision_dock;
mod transcript;

use std::sync::Arc;

use crate::tui_v2::{
    layout::measure::MeasuredPart,
    model::conversation::{ConversationMetadata, PendingInteraction},
    presentation::transcript::ConversationDisplayList,
};

pub use context_bar::render_context_bar;
pub use decision_dock::render_decision_dock;
pub use transcript::render_transcript;

pub struct ConversationRenderData<'a> {
    pub display: &'a ConversationDisplayList,
    pub measured: &'a [Arc<MeasuredPart>],
    pub metadata: &'a ConversationMetadata,
    pub pending: &'a [PendingInteraction],
}
