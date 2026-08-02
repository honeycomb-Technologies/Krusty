use crate::tui::app::App;
use crate::tui::blocks::{BlockType, StreamBlock};
use crate::tui::state::BlockIndices;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisplayItemKind {
    Message { message_index: usize },
    Block { block_type: BlockType, index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisplayItem {
    pub line_start: usize,
    pub height: usize,
    pub kind: DisplayItemKind,
}

#[derive(Debug, Default)]
pub(super) struct DisplayList {
    pub items: Vec<DisplayItem>,
    pub total_lines: usize,
}

impl DisplayList {
    pub fn build<MessageHeight, BlockHeight>(
        messages: &[(String, String)],
        mut message_height: MessageHeight,
        mut block_height: BlockHeight,
    ) -> Self
    where
        MessageHeight: FnMut(usize, &str, &str) -> usize,
        BlockHeight: FnMut(BlockType, usize) -> Option<usize>,
    {
        let mut items = Vec::with_capacity(messages.len());
        let mut indices = BlockIndices::new();
        let mut total_lines = 0;

        for (message_index, (role, content)) in messages.iter().enumerate() {
            let (kind, height) = if let Some((block_type, index)) = indices.get_and_increment(role)
            {
                let Some(height) = block_height(block_type, index) else {
                    continue;
                };
                (DisplayItemKind::Block { block_type, index }, height)
            } else {
                (
                    DisplayItemKind::Message { message_index },
                    message_height(message_index, role, content),
                )
            };

            items.push(DisplayItem {
                line_start: total_lines,
                height,
                kind,
            });
            total_lines = total_lines.saturating_add(height).saturating_add(1);
        }

        Self { items, total_lines }
    }
}

impl App {
    pub(super) fn stream_block_height(
        &self,
        block_type: BlockType,
        index: usize,
        content_width: u16,
    ) -> Option<usize> {
        let height = match block_type {
            BlockType::Thinking => self
                .runtime
                .blocks
                .thinking
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::Pinch => self
                .runtime
                .blocks
                .pinch
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::Bash => self
                .runtime
                .blocks
                .bash
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::TerminalPane => {
                if self.runtime.blocks.pinned_terminal == Some(index) {
                    None
                } else {
                    self.runtime
                        .blocks
                        .terminal
                        .get(index)
                        .map(|block| block.height(content_width, &self.ui.theme))
                }
            }
            BlockType::ToolResult => self
                .runtime
                .blocks
                .tool_result
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::Read => self
                .runtime
                .blocks
                .read
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::Edit => self
                .runtime
                .blocks
                .edit
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::Write => self
                .runtime
                .blocks
                .write
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::WebSearch => self
                .runtime
                .blocks
                .web_search
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::Explore => self
                .runtime
                .blocks
                .explore
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
            BlockType::Build => self
                .runtime
                .blocks
                .build
                .get(index)
                .map(|block| block.height(content_width, &self.ui.theme)),
        }?;

        Some(height as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_list_uses_one_coordinate_space_for_messages_and_blocks() {
        let messages = vec![
            ("user".to_owned(), "hello".to_owned()),
            ("thinking".to_owned(), String::new()),
            ("assistant".to_owned(), "done".to_owned()),
        ];

        let list = DisplayList::build(
            &messages,
            |_, _, content| content.len(),
            |block_type, index| {
                assert_eq!(block_type, BlockType::Thinking);
                assert_eq!(index, 0);
                Some(3)
            },
        );

        assert_eq!(list.items.len(), 3);
        assert_eq!(list.items[0].line_start, 0);
        assert_eq!(list.items[1].line_start, 6);
        assert_eq!(list.items[2].line_start, 10);
        assert_eq!(list.total_lines, 15);
    }

    #[test]
    fn omitted_blocks_do_not_leave_placeholder_rows() {
        let messages = vec![("terminal".to_owned(), String::new())];
        let list = DisplayList::build(&messages, |_, _, _| 0, |_, _| None);

        assert!(list.items.is_empty());
        assert_eq!(list.total_lines, 0);
    }

    fn layout_snapshot(list: &DisplayList) -> Vec<String> {
        list.items
            .iter()
            .map(|item| {
                let kind = match item.kind {
                    DisplayItemKind::Message { message_index } => {
                        format!("message:{message_index}")
                    }
                    DisplayItemKind::Block { block_type, index } => {
                        format!("block:{block_type:?}:{index}")
                    }
                };
                format!("{kind}@{}+{}", item.line_start, item.height)
            })
            .collect()
    }

    #[test]
    fn mixed_stream_layout_snapshots_remain_contiguous_after_resize() {
        let messages = vec![
            ("user".to_owned(), "abcdefghijkl".to_owned()),
            ("thinking".to_owned(), String::new()),
            ("assistant".to_owned(), "abcdefghijklmnopqrst".to_owned()),
            ("bash".to_owned(), String::new()),
            ("assistant".to_owned(), "done".to_owned()),
        ];

        let build = |width: usize| {
            DisplayList::build(
                &messages,
                |_, _, content| content.len().div_ceil(width),
                |block_type, _| match (block_type, width) {
                    (BlockType::Thinking, 8) => Some(3),
                    (BlockType::Bash, 8) => Some(4),
                    (BlockType::Thinking, _) => Some(2),
                    (BlockType::Bash, _) => Some(2),
                    _ => None,
                },
            )
        };

        assert_eq!(
            layout_snapshot(&build(8)),
            vec![
                "message:0@0+2",
                "block:Thinking:0@3+3",
                "message:2@7+3",
                "block:Bash:0@11+4",
                "message:4@16+1",
            ]
        );
        assert_eq!(
            layout_snapshot(&build(16)),
            vec![
                "message:0@0+1",
                "block:Thinking:0@2+2",
                "message:2@5+2",
                "block:Bash:0@8+2",
                "message:4@11+1",
            ]
        );
    }

    #[test]
    fn streaming_resize_stress_never_overlaps_or_leaves_coordinate_holes() {
        let mut messages = Vec::new();
        for index in 0..250 {
            messages.push((
                "user".to_owned(),
                format!("request-{index}-{}", "x".repeat(index % 41)),
            ));
            messages.push(("thinking".to_owned(), String::new()));
            messages.push((
                "assistant".to_owned(),
                format!("response-{index}-{}", "y".repeat(index % 67)),
            ));
            messages.push(("bash".to_owned(), String::new()));
        }

        for width in [18_usize, 32, 79, 24, 120, 16] {
            let list = DisplayList::build(
                &messages,
                |_, _, content| content.len().max(1).div_ceil(width),
                |block_type, index| match block_type {
                    BlockType::Thinking => Some(1 + index % 5),
                    BlockType::Bash => Some(2 + index % 11),
                    _ => None,
                },
            );

            for pair in list.items.windows(2) {
                assert_eq!(
                    pair[1].line_start,
                    pair[0]
                        .line_start
                        .saturating_add(pair[0].height)
                        .saturating_add(1),
                    "display items overlapped or left an unexpected hole at width {width}"
                );
            }
            let expected_total = list
                .items
                .last()
                .map_or(0, |item| item.line_start + item.height + 1);
            assert_eq!(list.total_lines, expected_total);
        }
    }
}
