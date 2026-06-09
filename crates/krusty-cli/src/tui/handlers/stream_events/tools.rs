use crate::ai::types::AiToolCall;
use crate::tui::app::App;
use crate::tui::blocks::StreamBlock;

impl App {
    pub(super) fn handle_tool_call_complete(
        &mut self,
        id: String,
        name: String,
        arguments: serde_json::Value,
    ) {
        if name == "AskUserQuestion" {
            self.runtime.pending_ask_user_calls.push(AiToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            });
        }

        self.create_tool_blocks(&[AiToolCall {
            id,
            name,
            arguments,
        }]);
    }

    pub(super) fn handle_tool_executing(&mut self) {
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

    pub(super) fn handle_tool_result(&mut self, id: String, output: String) {
        self.update_tool_result_block(&id, &output);
        self.update_read_block(&id, &output);
        self.update_edit_block(&id, &output);
        self.update_bash_block(&id, &output);
        self.update_explore_block(&id, &output);
        self.update_build_block(&id, &output);
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
        self.runtime.chat.streaming_assistant_idx = None;

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

        if name == "Task" || name == "explore" {
            tracing::info!(
                "handle_tool_start: explore tool '{}' detected, block will be created on execution",
                name
            );
        }

        if !matches!(
            name.as_str(),
            "bash"
                | "grep"
                | "glob"
                | "read"
                | "edit"
                | "write"
                | "processes"
                | "Task"
                | "explore"
                | "build"
                | "AskUserQuestion"
                | "task_start"
                | "task_complete"
                | "add_subtask"
                | "set_dependency"
                | "enter_plan_mode"
                | "set_work_mode"
        ) {
            self.runtime
                .chat
                .messages
                .push(("tool".to_string(), format!("Using tool: {} ...", name)));
        }

        if name == "AskUserQuestion" {
            self.runtime
                .chat
                .messages
                .push(("tool".to_string(), "Preparing questions...".to_string()));
        }
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
