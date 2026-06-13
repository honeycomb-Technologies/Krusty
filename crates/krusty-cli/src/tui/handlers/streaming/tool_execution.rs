//! Tool execution and result handling
//!
//! Handles visual block creation and tool result updates for the TUI.
//! The core orchestrator handles actual tool execution. This module provides
//! TUI-specific block management and user interaction (approval, AskUser).

use std::time::Duration;

use crate::ai::types::AiToolCall;
use crate::tui::app::App;
use crate::tui::components::{PromptOption, PromptQuestion};
use crate::tui::tool_presentation::{
    display_tool_name, payload_for_render, presentation_for_tool, renderable_tool_output,
    tool_pattern, ToolPresentation,
};
use crate::tui::utils::edit_diff;

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

impl App {
    /// Handle AskUserQuestion tool calls via UI instead of registry
    pub(crate) fn handle_ask_user_question_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        let Some(tool_call) = tool_calls.into_iter().next() else {
            return;
        };

        tracing::info!("Handling AskUserQuestion tool call: {}", tool_call.id);

        let questions_arg = tool_call.arguments.get("questions");
        let Some(questions_array) = questions_arg.and_then(|v| v.as_array()) else {
            tracing::warn!("AskUserQuestion missing questions array");
            return;
        };

        let mut prompt_questions: Vec<PromptQuestion> = Vec::new();

        for q in questions_array {
            let question = q.get("question").and_then(|v| v.as_str()).unwrap_or("");
            let header = q.get("header").and_then(|v| v.as_str()).unwrap_or("Q");
            let multi_select = q
                .get("multiSelect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let mut options: Vec<PromptOption> = Vec::new();
            if let Some(opts) = q.get("options").and_then(|v| v.as_array()) {
                for opt in opts {
                    let label = opt.get("label").and_then(|v| v.as_str()).unwrap_or("");
                    let description = opt.get("description").and_then(|v| v.as_str());
                    options.push(PromptOption {
                        label: label.to_string(),
                        description: description.map(|s| s.to_string()),
                    });
                }
            }

            // Add "Other" option for custom input
            options.push(PromptOption {
                label: "Other".to_string(),
                description: Some("Enter custom response".to_string()),
            });

            prompt_questions.push(PromptQuestion {
                question: question.to_string(),
                header: header.to_string(),
                options,
                multi_select,
            });
        }

        if prompt_questions.is_empty() {
            tracing::warn!("AskUserQuestion has no valid questions");
            return;
        }

        // Remove the "Preparing questions..." message
        if let Some((tag, _)) = self.runtime.chat.messages.last() {
            if tag == "tool" {
                self.runtime.chat.messages.pop();
            }
        }

        self.ui
            .decision_prompt
            .show_ask_user(prompt_questions, tool_call.id);
    }

    /// Create visual blocks for tool calls.
    pub(crate) fn create_tool_blocks(&mut self, tools: &[AiToolCall]) {
        for tool_call in tools {
            let tool_name = &tool_call.name;
            let presentation = presentation_for_tool(tool_name, &tool_call.arguments);

            match presentation {
                ToolPresentation::Bash => {
                    let command = tool_call
                        .arguments
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("bash")
                        .to_string();
                    self.runtime
                        .blocks
                        .bash
                        .push(crate::tui::blocks::BashBlock::with_tool_id(
                            command,
                            tool_call.id.clone(),
                        ));
                    self.runtime
                        .chat
                        .messages
                        .push(("bash".to_string(), tool_call.id.clone()));
                }
                ToolPresentation::Search => {
                    self.runtime
                        .blocks
                        .tool_result
                        .push(crate::tui::blocks::ToolResultBlock::new(
                            tool_call.id.clone(),
                            display_tool_name(tool_name, &tool_call.arguments),
                            tool_pattern(tool_name, &tool_call.arguments),
                        ));
                    self.runtime
                        .chat
                        .messages
                        .push(("tool_result".to_string(), tool_call.id.clone()));
                }
                ToolPresentation::WebSearch => {
                    self.runtime
                        .blocks
                        .web_search
                        .push(crate::tui::blocks::WebSearchBlock::new(
                            tool_call.id.clone(),
                            tool_pattern(tool_name, &tool_call.arguments),
                        ));
                    self.runtime
                        .chat
                        .messages
                        .push(("web_search".to_string(), tool_call.id.clone()));
                }
                ToolPresentation::Read => {
                    let file_path = tool_call
                        .arguments
                        .get("file_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("file")
                        .to_string();
                    self.runtime
                        .blocks
                        .read
                        .push(crate::tui::blocks::ReadBlock::new(
                            tool_call.id.clone(),
                            file_path,
                        ));
                    self.runtime
                        .chat
                        .messages
                        .push(("read".to_string(), tool_call.id.clone()));
                }
                ToolPresentation::Edit => self.create_or_update_edit_block(tool_call),
                ToolPresentation::Write => self.create_or_update_write_block(tool_call),
                ToolPresentation::ExploreAgent => {
                    let prompt = tool_call
                        .arguments
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Exploring...")
                        .to_string();
                    tracing::info!("Creating ExploreBlock for id={}", tool_call.id);
                    self.runtime.blocks.explore.push(
                        crate::tui::blocks::ExploreBlock::with_tool_id(
                            prompt,
                            tool_call.id.clone(),
                        ),
                    );
                    self.runtime
                        .chat
                        .messages
                        .push(("explore".to_string(), tool_call.id.clone()));
                }
                ToolPresentation::BuildAgent => {
                    let prompt = tool_call
                        .arguments
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Building...")
                        .to_string();
                    tracing::info!("Creating BuildBlock for id={}", tool_call.id);
                    self.runtime
                        .blocks
                        .build
                        .push(crate::tui::blocks::BuildBlock::with_tool_id(
                            prompt,
                            tool_call.id.clone(),
                        ));
                    self.runtime
                        .chat
                        .messages
                        .push(("build".to_string(), tool_call.id.clone()));
                }
                ToolPresentation::GenericStatus => {
                    self.runtime
                        .blocks
                        .tool_result
                        .push(crate::tui::blocks::ToolResultBlock::new(
                            tool_call.id.clone(),
                            display_tool_name(tool_name, &tool_call.arguments),
                            tool_pattern(tool_name, &tool_call.arguments),
                        ));
                    self.runtime
                        .chat
                        .messages
                        .push(("tool_result".to_string(), tool_call.id.clone()));
                }
                ToolPresentation::UiOnly => {}
            }

            if self.ui.scroll_system.scroll.auto_scroll && !presentation.is_ui_only() {
                self.ui.scroll_system.scroll.request_scroll_to_bottom();
            }
        }
    }

    fn create_or_update_edit_block(&mut self, tool_call: &AiToolCall) {
        let file_path = tool_call
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("file")
            .to_string();
        let old_string = tool_call
            .arguments
            .get("old_string")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let new_string = tool_call
            .arguments
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let start_line =
            edit_diff::find_start_line_in_file(&self.runtime.working_dir, &file_path, &old_string)
                .unwrap_or(1);

        let mut updated = false;
        if let Some(block) = self.runtime.blocks.edit.last_mut() {
            if block.is_pending() {
                block.set_tool_use_id(tool_call.id.clone());
                block.set_diff_data(
                    file_path.clone(),
                    old_string.clone(),
                    new_string.clone(),
                    start_line,
                );
                updated = true;
            }
        }
        if !updated {
            let mut block = crate::tui::blocks::EditBlock::new_pending(file_path.clone());
            block.set_diff_mode(self.runtime.blocks.diff_mode);
            block.set_tool_use_id(tool_call.id.clone());
            block.set_diff_data(file_path, old_string, new_string, start_line);
            self.runtime.blocks.edit.push(block);
            self.runtime
                .chat
                .messages
                .push(("edit".to_string(), tool_call.id.clone()));
        } else if let Some((kind, id)) = self.runtime.chat.messages.last_mut() {
            if kind == "edit" && id.is_empty() {
                *id = tool_call.id.clone();
            }
        }
    }

    fn create_or_update_write_block(&mut self, tool_call: &AiToolCall) {
        let file_path = tool_call
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("file")
            .to_string();
        let content = tool_call
            .arguments
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut updated = false;
        if let Some(block) = self.runtime.blocks.write.last_mut() {
            if block.is_pending() {
                block.set_tool_use_id(tool_call.id.clone());
                block.set_content(file_path.clone(), content.clone());
                updated = true;
            }
        }
        if !updated {
            let mut block = crate::tui::blocks::WriteBlock::new_pending(file_path.clone());
            block.set_tool_use_id(tool_call.id.clone());
            block.set_content(file_path, content);
            self.runtime.blocks.write.push(block);
            self.runtime
                .chat
                .messages
                .push(("write".to_string(), tool_call.id.clone()));
        } else if let Some((kind, id)) = self.runtime.chat.messages.last_mut() {
            if kind == "write" && id.is_empty() {
                *id = tool_call.id.clone();
            }
        }
    }

    /// Handle tool approval decision from DecisionPrompt
    pub(crate) fn handle_tool_approval_answer(
        &mut self,
        answers: &[crate::tui::components::PromptAnswer],
    ) {
        use crate::tui::components::PromptAnswer;

        self.runtime.approval_requested_at = None;
        let approved = matches!(answers.first(), Some(PromptAnswer::Selected(0)));

        // Send approval via orchestrator LoopInput channel
        if let Some(ref tx) = self.runtime.channels.loop_input {
            if let Some(ref id) = self.ui.decision_prompt.tool_use_id {
                let _ = tx.send(crate::agent::loop_events::LoopInput::ToolApproval {
                    tool_call_id: id.clone(),
                    approved,
                });
            }
        }
    }

    /// Update ToolResultBlock with output
    pub(crate) fn update_tool_result_block(
        &mut self,
        tool_use_id: &str,
        output_str: &str,
        is_error: bool,
    ) {
        for block in &mut self.runtime.blocks.tool_result {
            if block.tool_use_id() == tool_use_id {
                block.set_error(is_error);
                block.set_results(output_str);
                block.complete();
                if is_error {
                    block.set_collapsed(false);
                    self.ui.block_ui.set_collapsed(tool_use_id, false);
                }
                break;
            }
        }
    }

    /// Update ReadBlock with content
    pub(crate) fn update_read_block(&mut self, tool_use_id: &str, output_str: &str) {
        let render_output = renderable_tool_output(output_str);
        for block in &mut self.runtime.blocks.read {
            if block.tool_use_id() == tool_use_id {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&render_output) {
                    let payload = json.get("data").unwrap_or(&json);
                    let content = payload
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let total_lines = payload
                        .get("total_lines")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let lines_returned = payload
                        .get("lines_returned")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    block.set_content(content.to_string(), total_lines, lines_returned);
                } else {
                    let line_count = render_output.lines().count();
                    block.set_content(render_output, line_count, line_count);
                }
                block.complete();
                break;
            }
        }
    }

    /// Mark EditBlock complete when the edit tool finishes.
    pub(crate) fn update_edit_block(
        &mut self,
        tool_use_id: &str,
        output_str: &str,
        is_error: bool,
    ) {
        if is_error {
            self.replace_write_or_edit_with_error(tool_use_id, "edit", output_str);
            return;
        }

        let render_output = renderable_tool_output(output_str);
        for block in &mut self.runtime.blocks.edit {
            if block.tool_use_id() == Some(tool_use_id) {
                if let Some(start_line) = edit_diff::start_line_from_tool_output(&render_output) {
                    block.set_start_line(start_line);
                }
                block.complete();
                self.ui.block_ui.set_collapsed(tool_use_id, false);
                break;
            }
        }
    }

    pub(crate) fn update_write_block(
        &mut self,
        tool_use_id: &str,
        output_str: &str,
        is_error: bool,
    ) {
        if is_error {
            self.replace_write_or_edit_with_error(tool_use_id, "write", output_str);
            return;
        }

        for block in &mut self.runtime.blocks.write {
            if block.tool_use_id() == Some(tool_use_id) {
                block.complete();
                break;
            }
        }
    }

    fn replace_write_or_edit_with_error(
        &mut self,
        tool_use_id: &str,
        tool_name: &str,
        output: &str,
    ) {
        match tool_name {
            "edit" => self
                .runtime
                .blocks
                .edit
                .retain(|block| block.tool_use_id() != Some(tool_use_id)),
            "write" => self
                .runtime
                .blocks
                .write
                .retain(|block| block.tool_use_id() != Some(tool_use_id)),
            _ => {}
        }

        for (role, id) in &mut self.runtime.chat.messages {
            if id == tool_use_id && role == tool_name {
                *role = "tool_result".to_string();
                break;
            }
        }

        if !self
            .runtime
            .blocks
            .tool_result
            .iter()
            .any(|block| block.tool_use_id() == tool_use_id)
        {
            let mut block = crate::tui::blocks::ToolResultBlock::new(
                tool_use_id.to_string(),
                tool_name.to_string(),
                String::new(),
            );
            block.set_error(true);
            block.set_results(output);
            block.set_collapsed(false);
            block.complete();
            self.runtime.blocks.tool_result.push(block);
        }
        self.ui.block_ui.set_collapsed(tool_use_id, false);
    }

    /// Update BashBlock for background processes and final exit status.
    pub(crate) fn update_bash_block(
        &mut self,
        tool_use_id: &str,
        output_str: &str,
        is_error: bool,
    ) {
        let render_output = renderable_tool_output(output_str);
        for block in &mut self.runtime.blocks.bash {
            if block.tool_use_id() == Some(tool_use_id) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&render_output) {
                    if let Some(process_id) =
                        json.get("processId").and_then(|v| v.as_str()).or_else(|| {
                            json.get("data")
                                .and_then(|v| v.get("process_id"))
                                .and_then(|v| v.as_str())
                        })
                    {
                        block.set_background_process_id(process_id.to_string());
                        tracing::info!(
                            tool_use_id = %tool_use_id,
                            process_id = %process_id,
                            "BashBlock converted to background process"
                        );
                    }

                    let exit_code =
                        json.get("metadata")
                            .and_then(|v| v.get("exit_code"))
                            .and_then(|v| v.as_i64())
                            .or_else(|| json.get("exit_code").and_then(|v| v.as_i64()))
                            .unwrap_or(if is_error { 1 } else { 0 }) as i32;
                    if block.is_streaming() {
                        block.complete(exit_code);
                    }
                } else if block.is_streaming() {
                    block.complete(if is_error { 1 } else { 0 });
                }
                break;
            }
        }
    }

    /// Update WebSearchBlock for registered local web_search tool results.
    pub(crate) fn update_web_search_block(
        &mut self,
        tool_use_id: &str,
        output_str: &str,
        is_error: bool,
    ) {
        let Some(block) = self
            .runtime
            .blocks
            .web_search
            .iter_mut()
            .find(|block| block.tool_use_id() == tool_use_id)
        else {
            return;
        };

        if is_error {
            block.complete();
            return;
        }

        let Some(payload) = payload_for_render(output_str) else {
            block.complete();
            return;
        };
        let payload = payload.get("data").unwrap_or(&payload);
        let Some(results) = payload.get("results").and_then(|value| value.as_array()) else {
            block.complete();
            return;
        };

        let parsed = results
            .iter()
            .filter_map(|value| serde_json::from_value(value.clone()).ok())
            .collect::<Vec<crate::ai::types::WebSearchResult>>();
        block.set_results(parsed);
    }

    /// Update ExploreBlock with results
    pub(crate) fn update_explore_block(
        &mut self,
        tool_use_id: &str,
        output_str: &str,
        _is_error: bool,
    ) {
        let render_output = renderable_tool_output(output_str);
        for block in &mut self.runtime.blocks.explore {
            if block.tool_use_id() == Some(tool_use_id) {
                tracing::info!(
                    tool_use_id = %tool_use_id,
                    output_len = render_output.len(),
                    "Found matching ExploreBlock, completing with output"
                );
                block.complete(render_output);
                break;
            }
        }
    }

    /// Update BuildBlock with results
    pub(crate) fn update_build_block(
        &mut self,
        tool_use_id: &str,
        output_str: &str,
        _is_error: bool,
    ) {
        let render_output = renderable_tool_output(output_str);
        for block in &mut self.runtime.blocks.build {
            if block.tool_use_id() == Some(tool_use_id) {
                tracing::info!(
                    tool_use_id = %tool_use_id,
                    output_len = render_output.len(),
                    "Found matching BuildBlock, completing with output"
                );
                block.complete(render_output);
                break;
            }
        }
    }

    /// Check if a tool approval prompt has timed out and auto-reject if so
    pub fn check_approval_timeout(&mut self) {
        if self.ui.decision_prompt.prompt_type != crate::tui::components::PromptType::ToolApproval
            || !self.ui.decision_prompt.visible
        {
            return;
        }

        let Some(requested_at) = self.runtime.approval_requested_at else {
            return;
        };

        if requested_at.elapsed() >= APPROVAL_TIMEOUT {
            tracing::warn!("Tool approval timed out after {:?}", APPROVAL_TIMEOUT);
            self.runtime.approval_requested_at = None;
            self.ui.decision_prompt.hide();
            self.runtime.chat.messages.push((
                "system".to_string(),
                "Tool approval timed out (5 min). Automatically denied.".to_string(),
            ));

            // Send denial via orchestrator
            if let Some(ref tx) = self.runtime.channels.loop_input {
                if let Some(ref id) = self.ui.decision_prompt.tool_use_id {
                    let _ = tx.send(crate::agent::loop_events::LoopInput::ToolApproval {
                        tool_call_id: id.clone(),
                        approved: false,
                    });
                }
            }
        }
    }
}
