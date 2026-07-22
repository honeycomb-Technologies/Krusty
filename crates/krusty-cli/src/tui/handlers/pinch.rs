//! Manual in-place compaction (`/pinch`).

use crate::agent::{
    effective_context_window_for_runtime, estimate_rendered_request_tokens, inject_context,
    run_compaction_pipeline, CompactionManager, CompactionRequest, CompactionTrigger,
};
use crate::ai::client::CallOptions;
use crate::paths;
use crate::storage::ProjectSettings;
use crate::tools::registry::ToolRequestPolicy;
use crate::tui::app::App;
use crate::tui::blocks::PinchBlock;
use crate::tui::utils::CompactionUpdate;

impl App {
    /// Start the in-chat pinch animation (orchestrator-driven compaction).
    pub fn show_compaction_started(&mut self) {
        if self.runtime.active_pinch_block.is_some() || self.runtime.channels.compaction.is_some() {
            return;
        }
        self.start_pinch_animation();
    }

    fn start_pinch_animation(&mut self) -> usize {
        let idx = self.runtime.blocks.pinch.len();
        self.runtime.blocks.pinch.push(PinchBlock::new());
        self.runtime
            .chat
            .messages
            .push(("pinch".to_string(), String::new()));
        self.runtime.active_pinch_block = Some(idx);
        self.ui.needs_redraw = true;
        idx
    }

    pub(crate) fn finish_pinch_animation(&mut self, success: bool) {
        if let Some(idx) = self.runtime.active_pinch_block.take() {
            if let Some(block) = self.runtime.blocks.pinch.get_mut(idx) {
                block.complete(success);
            }
        }
        self.ui.needs_redraw = true;
    }

    /// Start manual in-place compaction for the current session.
    pub fn start_manual_compaction(&mut self, auto_continue: bool) {
        if self.runtime.chat.is_busy() {
            self.runtime.chat.messages.push((
                "system".to_string(),
                "Wait for the active response and tool execution to finish before compacting."
                    .to_string(),
            ));
            return;
        }

        if self.runtime.chat.conversation.is_empty() {
            self.runtime.chat.messages.push((
                "system".to_string(),
                "No conversation to compact. Start a chat first.".to_string(),
            ));
            return;
        }

        if self.runtime.channels.compaction.is_some() {
            return;
        }

        let Some(session_id) = self.runtime.current_session_id.clone() else {
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), "No active session.".to_string()));
            return;
        };

        self.start_pinch_animation();

        let db_path = paths::config_dir().join("krusty.db");
        let conversation = self.runtime.chat.conversation.clone();
        let working_dir = self.runtime.working_dir.clone();
        let current_model = self.runtime.current_model.clone();
        let selected_context_window = self.max_context_tokens();
        let project_dir = Some(self.runtime.working_dir.to_string_lossy().into_owned());

        let client = self.create_ai_client();
        let (compaction_manager, request_budget) = client.as_ref().map_or_else(
            || (CompactionManager::default(), None),
            |ai_client| {
                let effective_window = effective_context_window_for_runtime(
                    ai_client.config().uses_chatgpt_codex_format(),
                    selected_context_window,
                );
                let manager = CompactionManager::for_model(
                    ai_client.provider_id(),
                    ai_client.config().api_format,
                    &current_model,
                    effective_window,
                );

                let project_settings = ProjectSettings::load(&working_dir);
                let has_active_plan = self
                    .services
                    .plan_manager
                    .as_ref()
                    .and_then(|manager| manager.get_active_plan(&session_id).ok())
                    .flatten()
                    .is_some();
                let tools = ToolRequestPolicy::code(
                    self.runtime.permission_mode,
                    self.ui.work_mode == crate::tui::app::WorkMode::Plan,
                    has_active_plan,
                    true,
                    project_settings.disabled_tools.as_deref().unwrap_or(&[]),
                )
                .filter(self.services.cached_ai_tools.clone());
                let options = CallOptions {
                    tools: (!tools.is_empty()).then_some(tools),
                    enable_caching: true,
                    session_id: Some(session_id.clone()),
                    codex_parallel_tool_calls: true,
                    ..Default::default()
                };
                let with_context = inject_context(
                    &conversation,
                    &db_path,
                    &session_id,
                    &working_dir,
                    Some(&working_dir),
                    self.ui.work_mode.into(),
                    self.services.skills_manager.as_ref(),
                    Some(current_model.as_str()),
                    Some("code"),
                    None,
                    None,
                );
                let rendered = estimate_rendered_request_tokens(ai_client, &with_context, &options);
                let pressure = self
                    .runtime
                    .last_token_usage
                    .as_ref()
                    .map(|usage| usage.input_tokens())
                    .unwrap_or_default()
                    .max(rendered.total_tokens);
                (manager, Some(rendered.compaction_budget(pressure)))
            },
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.channels.compaction = Some(rx);

        tokio::spawn(async move {
            let result = async {
                let ai_client = client.as_ref();
                run_compaction_pipeline(CompactionRequest {
                    db_path: &db_path,
                    session_id: &session_id,
                    conversation: &conversation,
                    working_dir: &working_dir,
                    ai_client,
                    model: Some(current_model.as_str()),
                    trigger: CompactionTrigger::Manual {
                        preservation_hints: None,
                        direction: None,
                    },
                    compaction_manager,
                    request_budget,
                    last_usage_prompt_tokens: None,
                    messages_after_usage: 0,
                    summary_override: None,
                    project_dir: project_dir.as_deref(),
                    user_id: None,
                })
                .await
                .map_err(|error| error.to_string())
            }
            .await;

            let _ = tx.send(CompactionUpdate {
                result,
                auto_continue,
            });
        });
    }

    /// Poll for compaction completion.
    pub fn poll_compaction(&mut self) {
        let rx = match self.runtime.channels.compaction.as_mut() {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(update) => {
                self.runtime.channels.compaction = None;
                match update.result {
                    Ok(result) => {
                        self.runtime.chat.conversation = result.compacted_conversation;
                        self.runtime.context_tokens_used = result.estimated_tokens_after;
                        self.runtime.last_token_usage = None;
                        self.finish_pinch_animation(true);
                        if update.auto_continue {
                            self.send_to_ai();
                        }
                    }
                    Err(error) => {
                        self.finish_pinch_animation(false);
                        self.runtime
                            .chat
                            .messages
                            .push(("system".to_string(), format!("Compaction failed: {error}")));
                    }
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.runtime.channels.compaction = None;
                self.finish_pinch_animation(false);
            }
        }
    }

    /// Show a completed pinch indicator for orchestrator-driven auto compaction.
    pub fn show_auto_compaction_complete(&mut self) {
        let mut block = PinchBlock::new();
        block.complete(true);
        self.runtime.blocks.pinch.push(block);
        self.runtime
            .chat
            .messages
            .push(("pinch".to_string(), String::new()));
        self.ui.needs_redraw = true;
    }
}
