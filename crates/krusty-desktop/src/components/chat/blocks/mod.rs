pub mod bash_output;
pub mod thinking;
pub mod tool_call;

use gpui::AnyElement;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptBlock {
    Thinking(thinking::ThinkingBlockState),
    ToolCall(tool_call::ToolCallBlockState),
    BashOutput(bash_output::BashOutputBlockState),
}

pub fn render_block(block: &TranscriptBlock) -> AnyElement {
    match block {
        TranscriptBlock::Thinking(state) => thinking::thinking_block(state),
        TranscriptBlock::ToolCall(state) => tool_call::tool_call_block(state),
        TranscriptBlock::BashOutput(state) => bash_output::bash_output_block(state),
    }
}
