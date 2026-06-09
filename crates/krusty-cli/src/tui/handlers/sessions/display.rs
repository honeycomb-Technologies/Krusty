use crate::ai::types::{Content, Role};
use crate::tui::app::App;
use crate::tui::blocks::{
    BashBlock, EditBlock, ReadBlock, ThinkingBlock, ToolResultBlock, WriteBlock,
};
use crate::tui::state::{hash_content, BlockManager};
use crate::tui::utils::edit_diff;

impl App {
    /// Build tool results cache from conversation
    pub(super) fn build_tool_results_cache(&mut self) {
        self.runtime.tool_results.clear();

        for msg in &self.runtime.chat.conversation {
            for content in &msg.content {
                if let Content::ToolResult {
                    tool_use_id,
                    output,
                    is_error,
                } = content
                {
                    let tool_name = self.find_tool_name_for_id(tool_use_id);
                    let output_str = match output {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    tracing::info!(
                        "Caching tool result: {} (tool={}) is_error={:?}",
                        tool_use_id,
                        tool_name,
                        is_error
                    );
                    self.runtime.tool_results.insert_raw(
                        tool_use_id.clone(),
                        &tool_name,
                        &output_str,
                        is_error.unwrap_or(false),
                    );
                }
            }
        }
    }
    /// Find the tool name for a given tool_use_id by searching conversation
    fn find_tool_name_for_id(&self, tool_use_id: &str) -> String {
        for msg in &self.runtime.chat.conversation {
            for content in &msg.content {
                if let Content::ToolUse { id, name, .. } = content {
                    if id == tool_use_id {
                        return name.clone();
                    }
                }
            }
        }
        "unknown".to_string()
    }

    /// Build display messages and blocks from conversation
    ///
    /// Messages array format: (role, content) where role determines rendering:
    /// - "user" / "assistant" / "system" → text message
    /// - "thinking" → ThinkingBlock at current thinking index
    /// - "bash" → BashBlock at current bash index
    /// - "read" / "edit" / "write" → respective block types
    /// - "tool_result" → ToolResultBlock (grep/glob/unknown tools)
    pub(super) fn build_display_from_conversation(&mut self) {
        self.runtime.chat.messages.clear();
        self.runtime.chat.streaming_assistant_idx = None;
        self.runtime.blocks = BlockManager::new();

        for msg in &self.runtime.chat.conversation {
            let base_role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            };

            for content in &msg.content {
                match content {
                    Content::Text { text } => {
                        // Skip filler messages (single "." used for API alternation)
                        if text == "." && msg.content.len() == 1 {
                            tracing::debug!("Skipping filler message in display");
                            continue;
                        }
                        // Text messages use the API role
                        self.runtime
                            .chat
                            .messages
                            .push((base_role.to_string(), text.clone()));
                    }

                    Content::Thinking {
                        thinking,
                        signature,
                    } => {
                        // Thinking gets its own message entry with "thinking" role
                        self.runtime
                            .chat
                            .messages
                            .push(("thinking".to_string(), String::new()));

                        let mut block = ThinkingBlock::new();
                        block.append(thinking);
                        if !signature.is_empty() {
                            block.set_signature(signature.clone());
                        }
                        block.complete();
                        block.set_collapsed(true);

                        let block_id = if signature.is_empty() {
                            hash_content(thinking)
                        } else {
                            signature.clone()
                        };
                        self.ui.block_ui.set_collapsed(&block_id, true);
                        self.runtime.blocks.thinking.push(block);
                    }

                    Content::RedactedThinking { .. } => {
                        // Redacted thinking - create a placeholder thinking block
                        self.runtime
                            .chat
                            .messages
                            .push(("thinking".to_string(), String::new()));

                        let mut block = ThinkingBlock::new();
                        block.append("[Redacted]");
                        block.complete();
                        block.set_collapsed(true);
                        self.runtime.blocks.thinking.push(block);
                    }

                    Content::ToolUse { id, name, input } => {
                        // Each tool use gets its own message entry with tool-specific role
                        match name.to_lowercase().as_str() {
                            "bash" => {
                                self.runtime
                                    .chat
                                    .messages
                                    .push(("bash".to_string(), id.clone()));

                                let command = input
                                    .get("command")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let mut block = BashBlock::with_tool_id(command, id.clone());
                                if let Some(result) = self.runtime.tool_results.get(id) {
                                    block.append(&result.output);
                                    block.complete(result.exit_code);
                                }
                                block.set_collapsed(false);
                                self.ui.block_ui.set_collapsed(id, false);
                                self.runtime.blocks.bash.push(block);
                            }

                            "read" => {
                                self.runtime
                                    .chat
                                    .messages
                                    .push(("read".to_string(), id.clone()));

                                let file_path = input
                                    .get("file_path")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let mut block = ReadBlock::new(id.clone(), file_path);
                                if let Some(result) = self.runtime.tool_results.get(id) {
                                    let line_count = result.output.lines().count();
                                    block.set_content(
                                        result.output.clone(),
                                        line_count,
                                        line_count,
                                    );
                                    block.complete();
                                }
                                block.set_collapsed(true);
                                self.ui.block_ui.set_collapsed(id, true);
                                self.runtime.blocks.read.push(block);
                            }

                            "edit" => {
                                let result = self.runtime.tool_results.get(id);
                                let is_error = result.map(|r| r.is_error).unwrap_or(false);

                                if is_error {
                                    // Failed edit - show as tool_result with error
                                    self.runtime
                                        .chat
                                        .messages
                                        .push(("tool_result".to_string(), id.clone()));

                                    let mut block = ToolResultBlock::new(
                                        id.clone(),
                                        "edit".to_string(),
                                        input
                                            .get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    );
                                    if let Some(r) = result {
                                        block.set_results(&r.output);
                                    }
                                    block.complete();
                                    block.set_collapsed(false);
                                    self.ui.block_ui.set_collapsed(id, false);
                                    self.runtime.blocks.tool_result.push(block);
                                } else {
                                    // Successful edit - show as edit block
                                    self.runtime
                                        .chat
                                        .messages
                                        .push(("edit".to_string(), id.clone()));

                                    let file_path = input
                                        .get("file_path")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let old_string = input
                                        .get("old_string")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let new_string = input
                                        .get("new_string")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    let start_line = result
                                        .and_then(|result| {
                                            edit_diff::start_line_from_tool_output(&result.output)
                                        })
                                        .or_else(|| {
                                            edit_diff::find_start_line_in_file(
                                                &self.runtime.working_dir,
                                                &file_path,
                                                &old_string,
                                            )
                                        })
                                        .unwrap_or(1);

                                    let mut block = EditBlock::new_pending(file_path.clone());
                                    block.set_tool_use_id(id.clone());
                                    block.set_diff_data(
                                        file_path, old_string, new_string, start_line,
                                    );
                                    block.complete();
                                    block.set_collapsed(false);
                                    self.ui.block_ui.set_collapsed(id, false);
                                    self.runtime.blocks.edit.push(block);
                                }
                            }

                            "write" => {
                                let result = self.runtime.tool_results.get(id);
                                let is_error = result.map(|r| r.is_error).unwrap_or(false);
                                tracing::info!(
                                    "Write tool {} - has_result={}, is_error={}",
                                    id,
                                    result.is_some(),
                                    is_error
                                );

                                if is_error {
                                    // Failed write - show as tool_result with error
                                    self.runtime
                                        .chat
                                        .messages
                                        .push(("tool_result".to_string(), id.clone()));

                                    let mut block = ToolResultBlock::new(
                                        id.clone(),
                                        "write".to_string(),
                                        input
                                            .get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    );
                                    if let Some(r) = result {
                                        block.set_results(&r.output);
                                    }
                                    block.complete();
                                    block.set_collapsed(false); // Show errors expanded
                                    self.ui.block_ui.set_collapsed(id, false);
                                    self.runtime.blocks.tool_result.push(block);
                                } else {
                                    // Successful write - show as write block
                                    self.runtime
                                        .chat
                                        .messages
                                        .push(("write".to_string(), id.clone()));

                                    let file_path = input
                                        .get("file_path")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let file_content = input
                                        .get("content")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    let mut block = WriteBlock::new_pending(file_path.clone());
                                    block.set_tool_use_id(id.clone());
                                    block.set_content(file_path, file_content);
                                    block.complete();
                                    block.set_collapsed(true);
                                    self.ui.block_ui.set_collapsed(id, true);
                                    self.runtime.blocks.write.push(block);
                                }
                            }

                            "grep" | "glob" => {
                                self.runtime
                                    .chat
                                    .messages
                                    .push(("tool_result".to_string(), id.clone()));

                                let pattern = input
                                    .get("pattern")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let mut block =
                                    ToolResultBlock::new(id.clone(), name.clone(), pattern);
                                if let Some(result) = self.runtime.tool_results.get(id) {
                                    block.set_results(&result.output);
                                    block.complete();
                                }
                                block.set_collapsed(true);
                                self.ui.block_ui.set_collapsed(id, true);
                                self.runtime.blocks.tool_result.push(block);
                            }

                            // Silent tools - don't create any visual element
                            "task_complete" | "enter_plan_mode" | "set_work_mode" | "todowrite" => {
                                // These tools are intentionally silent and should not
                                // create any UI blocks when rebuilding from conversation
                            }

                            _ => {
                                // Unknown tools go to tool_result
                                self.runtime
                                    .chat
                                    .messages
                                    .push(("tool_result".to_string(), id.clone()));

                                let mut block =
                                    ToolResultBlock::new(id.clone(), name.clone(), String::new());
                                if let Some(result) = self.runtime.tool_results.get(id) {
                                    block.set_results(&result.output);
                                    block.complete();
                                }
                                block.set_collapsed(true);
                                self.ui.block_ui.set_collapsed(id, true);
                                self.runtime.blocks.tool_result.push(block);
                            }
                        }
                    }

                    Content::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                    } => {
                        // Check if this result has a matching ToolUse in the conversation
                        let has_matching_tool_use =
                            self.runtime.chat.conversation.iter().any(|m| {
                                m.content.iter().any(
                                |c| matches!(c, Content::ToolUse { id, .. } if id == tool_use_id),
                            )
                            });

                        if !has_matching_tool_use {
                            // Orphan ToolResult - create a visible block so it's not lost
                            tracing::warn!(
                                "Found orphan ToolResult without matching ToolUse: {}",
                                tool_use_id
                            );

                            self.runtime
                                .chat
                                .messages
                                .push(("tool_result".to_string(), tool_use_id.clone()));

                            let output_str = match output {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };

                            let mut block = ToolResultBlock::new(
                                tool_use_id.clone(),
                                "unknown".to_string(),
                                String::new(),
                            );
                            block.set_results(&output_str);
                            if is_error.unwrap_or(false) {
                                block.set_collapsed(false);
                            } else {
                                block.set_collapsed(true);
                            }
                            block.complete();
                            self.ui
                                .block_ui
                                .set_collapsed(tool_use_id, is_error.unwrap_or(false));
                            self.runtime.blocks.tool_result.push(block);
                        }
                        // Otherwise: handled via the cache when creating ToolUse blocks
                    }

                    Content::Image { .. } => {
                        // Images displayed as text for now
                        self.runtime
                            .chat
                            .messages
                            .push((base_role.to_string(), "[Image]".to_string()));
                    }

                    Content::Document { .. } => {
                        // Documents (PDFs) displayed as text for now
                        self.runtime
                            .chat
                            .messages
                            .push((base_role.to_string(), "[PDF]".to_string()));
                    }
                }
            }
        }
    }
}
