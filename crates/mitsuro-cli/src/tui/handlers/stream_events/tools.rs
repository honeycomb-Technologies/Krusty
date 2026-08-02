use crate::ai::types::AiToolCall;
use crate::tui::app::App;
use crate::tui::blocks::StreamBlock;
use crate::tui::components::Toast;
use crate::tui::tool_presentation::{presentation_for_tool, tool_summary};

impl App {
    pub(super) fn handle_tool_call_complete(
        &mut self,
        id: String,
        name: String,
        arguments: serde_json::Value,
    ) {
        let call = AiToolCall {
            id,
            name,
            arguments,
        };
        self.runtime
            .live_tool_calls
            .insert(call.id.clone(), call.clone());

        if call.name == "AskUserQuestion" {
            self.runtime.pending_ask_user_calls.push(call.clone());
        }

        self.create_tool_blocks(&[call]);
    }

    pub(super) fn handle_tool_executing(&mut self) {
        // The model stream that produced the tool call has ended; any later
        // assistant text belongs to the post-tool model response and should
        // render after the tool widgets.
        self.runtime.chat.streaming_assistant_idx = None;
        if !self.runtime.chat.is_executing_tools {
            self.start_tool_execution();
        }
    }

    pub(super) fn handle_tool_output_delta(&mut self, id: String, delta: String) {
        for block in &mut self.runtime.blocks.bash {
            if block.tool_use_id() == Some(&id) {
                block.append(&delta);
                break;
            }
        }
        if self.ui.scroll_system.scroll.auto_scroll {
            self.ui.scroll_system.scroll.request_scroll_to_bottom();
        }
    }

    pub(super) fn handle_tool_result(&mut self, id: String, output: String, is_error: bool) {
        // Ensure the next model response starts after this tool result instead
        // of appending into the pre-tool assistant text.
        self.runtime.chat.streaming_assistant_idx = None;

        self.update_tool_result_block(&id, &output, is_error);
        self.update_read_block(&id, &output);
        self.update_edit_block(&id, &output, is_error);
        self.update_write_block(&id, &output, is_error);
        self.update_bash_block(&id, &output, is_error);
        self.update_web_search_block(&id, &output, is_error);
        self.update_explore_block(&id, &output, is_error);
        self.update_build_block(&id, &output, is_error);

        if is_error {
            if let Some(call) = self.runtime.live_tool_calls.get(&id) {
                if presentation_for_tool(&call.name, &call.arguments).is_ui_only() {
                    let summary =
                        tool_summary(&output).unwrap_or_else(|| "tool failed".to_string());
                    self.show_toast(Toast::error(format!("{}: {}", call.name, summary)));
                }
            }
        }
    }

    pub(super) fn handle_tool_approval_required(&mut self, id: String, name: String) {
        self.ui
            .decision_prompt
            .show_tool_approval(vec![name], vec![id]);
        self.runtime.approval_requested_at = Some(std::time::Instant::now());
    }

    pub(super) fn handle_tool_approved(&mut self, id: String) {
        tracing::info!("Tool approved: {}", id);
    }

    pub(super) fn handle_tool_denied(&mut self, id: String) {
        tracing::info!("Tool denied: {}", id);
    }

    pub(super) fn handle_awaiting_input(&mut self, tool_call_id: String, tool_name: String) {
        if tool_name == "AskUserQuestion" {
            if let Some(call_idx) = self
                .runtime
                .pending_ask_user_calls
                .iter()
                .position(|call| call.id == tool_call_id)
            {
                let call = self.runtime.pending_ask_user_calls.remove(call_idx);
                self.handle_ask_user_question_tools(vec![call]);
            }
        } else if tool_name == "PlanConfirm" {
            tracing::info!("Plan confirmation awaiting input: {}", tool_call_id);
        }
    }

    /// Handle tool start event
    pub(super) fn handle_tool_start(&mut self, name: String) {
        self.complete_streaming_blocks();

        if name == "edit" {
            self.runtime
                .blocks
                .edit
                .push(crate::tui::blocks::EditBlock::new_pending(
                    "...".to_string(),
                ));
            if let Some(block) = self.runtime.blocks.edit.last_mut() {
                block.set_diff_mode(self.runtime.blocks.diff_mode);
            }
            self.runtime
                .chat
                .messages
                .push(("edit".to_string(), String::new()));
        }

        if name == "write" {
            self.runtime
                .blocks
                .write
                .push(crate::tui::blocks::WriteBlock::new_pending(
                    "...".to_string(),
                ));
            self.runtime
                .chat
                .messages
                .push(("write".to_string(), String::new()));
        }

        // Other tool families need complete arguments before deciding whether
        // they should create a widget. Avoid transcript placeholders like
        // "Using tool..." or "Preparing questions..."; those are protocol
        // noise and should stay invisible.
    }

    /// Mark all streaming blocks as complete
    pub(super) fn complete_streaming_blocks(&mut self) {
        for block in &mut self.runtime.blocks.read {
            if block.is_streaming() {
                block.complete();
            }
        }
        for block in &mut self.runtime.blocks.edit {
            if block.is_streaming() {
                block.complete();
            }
        }
        for block in &mut self.runtime.blocks.write {
            if block.is_streaming() {
                block.complete();
            }
        }
        for block in &mut self.runtime.blocks.web_search {
            if block.is_streaming() {
                block.complete();
            }
        }
    }
}
