use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Timer,
    Window,
};
use gpui_component::input::{InputEvent, InputState};
use std::time::Duration;

use crate::api::{
    ChatRequest, ChatStreamEvent, KrustyApiClient, ModelResponse, PlanItem, ReasoningControl,
};
use crate::chat::session::{ChatSessionState, ThinkingLevel};
use crate::components::chat::approval_bar::tool_approval_bar;
use crate::components::chat::blocks::bash_output::BashOutputBlockState;
use crate::components::chat::blocks::thinking::ThinkingBlockState;
use crate::components::chat::blocks::tool_call::{ToolCallBlockState, ToolCallStatus};
use crate::components::chat::blocks::TranscriptBlock;
use crate::components::chat::composer::chat_composer;
use crate::components::chat::plan_tracker::plan_tracker;
use crate::components::chat::transcript::{transcript_view, TranscriptItem};
use crate::design::theme;

const MODEL_CATALOG_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingToolApproval {
    pub tool_call_id: String,
    pub tool_name: String,
}

pub struct ChatPanel {
    client: KrustyApiClient,
    input: Entity<InputState>,
    session: ChatSessionState,
    items: Vec<TranscriptItem>,
    models: Vec<ModelResponse>,
    model_index: usize,
    plan_items: Vec<PlanItem>,
    pending_approval: Option<PendingToolApproval>,
    model_refresh_pending: bool,
    stop_requested: Arc<AtomicBool>,
    _input_subscription: Subscription,
}

impl ChatPanel {
    pub fn new(client: KrustyApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Ask Krusty…")
                .auto_grow(1, 6)
        });
        let input_subscription =
            cx.subscribe_in(&input, window, move |panel, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: false }) {
                    panel.submit(input, window, cx);
                }
            });

        let model_response = client.list_models().unwrap_or_default();
        let model_index = default_model_index(
            &model_response.models,
            model_response.default_model.as_deref(),
        );
        let models = model_response.models;
        let server = client.base_url().to_owned();
        let mut session = ChatSessionState::new();
        sync_model_controls(&mut session, models.get(model_index));
        let mut panel = Self {
            client,
            input,
            session,
            items: Vec::new(),
            models,
            model_index,
            plan_items: Vec::new(),
            pending_approval: None,
            model_refresh_pending: false,
            stop_requested: Arc::new(AtomicBool::new(false)),
            _input_subscription: input_subscription,
        };
        panel.items.push(TranscriptItem::System(format!(
            "Chat ready. Streaming through {server}/api/chat."
        )));
        Self::schedule_model_catalog_refresh(cx);
        panel
    }

    pub fn set_client(&mut self, client: KrustyApiClient) {
        if self.client.base_url() == client.base_url() {
            return;
        }
        let server = client.base_url().to_owned();
        self.client = client;
        let model_response = self.client.list_models().unwrap_or_default();
        self.apply_model_catalog(model_response);
        self.model_refresh_pending = false;
        self.session.session_id = None;
        self.plan_items.clear();
        self.pending_approval = None;
        self.items.push(TranscriptItem::System(format!(
            "Server changed to {server}; the next message starts a new session."
        )));
    }

    pub fn set_project_dir(&mut self, project_dir: Option<String>) {
        if self.session.project_dir == project_dir {
            return;
        }
        self.session.project_dir = project_dir.clone();
        if let Some(dir) = project_dir {
            self.items.push(TranscriptItem::System(format!(
                "Project context set to {dir}."
            )));
        }
    }

    pub fn needs_context_sync(
        &self,
        client: &KrustyApiClient,
        project_dir: &Option<String>,
    ) -> bool {
        self.client.base_url() != client.base_url() || self.session.project_dir != *project_dir
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn cycle_model(&mut self, cx: &mut Context<Self>) {
        self.refresh_model_catalog(cx);
        if self.models.is_empty() {
            return;
        }
        self.model_index = (self.model_index + 1) % self.models.len();
        sync_model_controls(&mut self.session, self.models.get(self.model_index));
        cx.notify();
    }

    fn apply_model_catalog(&mut self, response: crate::api::ModelsResponse) {
        let selected_model = self.session.model.as_deref();
        self.model_index = selected_model
            .and_then(|selected| {
                response
                    .models
                    .iter()
                    .position(|model| model.id == selected)
            })
            .unwrap_or_else(|| {
                default_model_index(&response.models, response.default_model.as_deref())
            });
        self.models = response.models;
        sync_model_controls(&mut self.session, self.models.get(self.model_index));
    }

    fn refresh_model_catalog(&mut self, cx: &mut Context<Self>) {
        if self.model_refresh_pending {
            return;
        }
        self.model_refresh_pending = true;
        let client = self.client.clone();
        let source_server = client.base_url().to_owned();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.list_models() })
                .await;
            let _ = this.update(cx, |panel, cx| {
                if panel.client.base_url() != source_server {
                    return;
                }
                panel.model_refresh_pending = false;
                if let Ok(response) = result {
                    panel.apply_model_catalog(response);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn schedule_model_catalog_refresh(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            Timer::after(MODEL_CATALOG_REFRESH_INTERVAL).await;
            if this
                .update(cx, |panel, cx| panel.refresh_model_catalog(cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    pub fn cycle_thinking(&mut self, cx: &mut Context<Self>) {
        if let Some(model) = self.models.get(self.model_index) {
            let levels = selectable_thinking_levels(model);
            let next = levels
                .iter()
                .position(|level| *level == self.session.thinking_level)
                .map_or(0, |index| (index + 1) % levels.len());
            self.session.thinking_level = levels[next];
        } else {
            self.session.thinking_level = self.session.thinking_level.cycle();
        }
        cx.notify();
    }

    pub fn toggle_permission(&mut self, cx: &mut Context<Self>) {
        self.session.permission_mode = self.session.permission_mode.toggle();
        cx.notify();
    }

    pub fn toggle_fast_mode(&mut self, cx: &mut Context<Self>) {
        if self
            .models
            .get(self.model_index)
            .is_some_and(|model| model.supports_fast_mode)
        {
            self.session.fast_mode = !self.session.fast_mode;
        } else {
            self.session.fast_mode = false;
        }
        cx.notify();
    }

    pub fn toggle_work_mode(&mut self, cx: &mut Context<Self>) {
        self.session.work_mode = self.session.work_mode.toggle();
        cx.notify();
    }

    pub fn stop_stream(&mut self, cx: &mut Context<Self>) {
        self.stop_requested.store(true, Ordering::SeqCst);
        self.session.is_streaming = false;
        cx.notify();
    }

    pub fn respond_tool_approval(
        &mut self,
        tool_call_id: &str,
        approved: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.session.session_id.clone() else {
            self.items.push(TranscriptItem::System(
                "Cannot approve tools before a session exists.".to_owned(),
            ));
            cx.notify();
            return;
        };
        let client = self.client.clone();
        let tool_call_id = tool_call_id.to_owned();
        let status_message_id = tool_call_id.clone();
        self.pending_approval = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(0)).await;
            let result = cx
                .background_spawn(async move {
                    client.approve_tool(&session_id, &tool_call_id, approved)
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                match result {
                    Ok(()) => {
                        let action = if approved { "Approved" } else { "Denied" };
                        panel.items.push(TranscriptItem::System(format!(
                            "{action} tool call {status_message_id}."
                        )));
                    }
                    Err(error) => {
                        panel.items.push(TranscriptItem::System(format!(
                            "Tool approval failed: {error:#}"
                        )));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn submit(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = input.read(cx).value().trim().to_owned();
        if text.is_empty() || self.session.is_streaming {
            return;
        }

        input.update(cx, |input, cx| input.set_value("", window, cx));
        self.items.push(TranscriptItem::User(text.clone()));
        let assistant_index = self.items.len();
        self.items.push(TranscriptItem::Assistant {
            content: String::new(),
            streaming: true,
        });
        self.session.is_streaming = true;
        self.stop_requested.store(false, Ordering::SeqCst);
        cx.notify();

        let client = self.client.clone();
        let request = self.build_chat_request(text);
        let stop_flag = self.stop_requested.clone();

        cx.spawn(async move |this, cx| {
            // Defer until the current ChatPanel update (submit / key handler) finishes.
            Timer::after(Duration::from_millis(0)).await;

            let rx = cx
                .background_spawn(async move { client.start_chat_stream(request) })
                .await;
            let mut final_result = None;

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                match rx.try_recv() {
                    Ok(ChatStreamEvent::Complete(result)) => {
                        final_result = Some(result);
                        break;
                    }
                    Ok(event) => {
                        let _ = this.update(cx, |panel, cx| {
                            panel.apply_stream_event(assistant_index, event);
                            cx.notify();
                        });
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        Timer::after(Duration::from_millis(16)).await;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }

            let _ = this.update(cx, |panel, cx| {
                panel.session.is_streaming = false;
                match final_result {
                    Some(Ok(result)) => {
                        if let Some(session_id) = &result.session_id {
                            panel.session.session_id = Some(session_id.clone());
                        }
                        if let Some(TranscriptItem::Assistant { content, streaming }) =
                            panel.items.get_mut(assistant_index)
                        {
                            if content.trim().is_empty() {
                                *content = if result.text.trim().is_empty() {
                                    "Turn completed without assistant text.".to_owned()
                                } else {
                                    result.text
                                };
                            }
                            *streaming = false;
                        }
                    }
                    Some(Err(error)) => {
                        if let Some(TranscriptItem::Assistant { content, streaming }) =
                            panel.items.get_mut(assistant_index)
                        {
                            *content = format!("Chat request failed: {error:#}");
                            *streaming = false;
                        }
                    }
                    None if stop_flag.load(Ordering::SeqCst) => {
                        if let Some(TranscriptItem::Assistant { streaming, .. }) =
                            panel.items.get_mut(assistant_index)
                        {
                            *streaming = false;
                        }
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn build_chat_request(&self, message: String) -> ChatRequest {
        let session_type = self.session.project_dir.as_ref().map(|_| "code".to_owned());
        ChatRequest {
            session_id: self.session.session_id.clone(),
            message,
            project_dir: self.session.project_dir.clone(),
            working_dir: self.session.project_dir.clone(),
            model: self.session.model.clone().or_else(|| {
                self.models
                    .get(self.model_index)
                    .map(|model| model.id.clone())
            }),
            thinking_enabled: self.session.thinking_level.api_value().map(str::to_owned),
            permission_mode: Some(self.session.permission_mode.api_value().to_owned()),
            fast_mode: self.session.fast_mode.then_some(true),
            mode: Some(self.session.work_mode.api_value().to_owned()),
            session_type,
        }
    }

    fn apply_stream_event(&mut self, assistant_index: usize, event: ChatStreamEvent) {
        match event {
            ChatStreamEvent::TextDelta(delta) => {
                if let Some(TranscriptItem::Assistant { content, .. }) =
                    self.items.get_mut(assistant_index)
                {
                    content.push_str(&delta);
                }
            }
            ChatStreamEvent::ThinkingDelta(delta) => {
                self.append_thinking_delta(&delta);
            }
            ChatStreamEvent::ToolCallStart { id, name } => {
                self.items
                    .push(TranscriptItem::Block(TranscriptBlock::ToolCall(
                        ToolCallBlockState {
                            id,
                            name,
                            status: ToolCallStatus::Started,
                            output: String::new(),
                        },
                    )));
            }
            ChatStreamEvent::ToolExecuting { id, name } => {
                self.update_tool_call(&id, name, ToolCallStatus::Executing, None);
            }
            ChatStreamEvent::ToolOutputDelta { id, delta } => {
                self.append_bash_output(&id, &delta, true);
            }
            ChatStreamEvent::ToolResult {
                id,
                output,
                is_error,
            } => {
                let status = if is_error {
                    ToolCallStatus::Error
                } else {
                    ToolCallStatus::Complete
                };
                self.update_tool_call(&id, String::new(), status, Some(output));
                self.append_bash_output(&id, "", false);
            }
            ChatStreamEvent::PlanUpdate(items) => {
                self.plan_items = items;
            }
            ChatStreamEvent::ToolApprovalRequired { id, name } => {
                self.pending_approval = Some(PendingToolApproval {
                    tool_call_id: id,
                    tool_name: name,
                });
            }
            ChatStreamEvent::TitleUpdate(title) => {
                self.items
                    .push(TranscriptItem::System(format!("Session title: {title}")));
            }
            ChatStreamEvent::Error(message) => {
                if let Some(TranscriptItem::Assistant { content, streaming }) =
                    self.items.get_mut(assistant_index)
                {
                    content.push_str(&format!("\n\nError: {message}"));
                    *streaming = false;
                }
            }
            ChatStreamEvent::Complete(_) => {}
        }
    }

    fn append_thinking_delta(&mut self, delta: &str) {
        if let Some(TranscriptItem::Block(TranscriptBlock::Thinking(state))) = self.items.last_mut()
        {
            state.content.push_str(delta);
            state.streaming = true;
            return;
        }
        self.items
            .push(TranscriptItem::Block(TranscriptBlock::Thinking(
                ThinkingBlockState {
                    content: delta.to_owned(),
                    expanded: false,
                    streaming: true,
                },
            )));
    }

    fn update_tool_call(
        &mut self,
        id: &str,
        name: String,
        status: ToolCallStatus,
        output: Option<String>,
    ) {
        for item in &mut self.items {
            if let TranscriptItem::Block(TranscriptBlock::ToolCall(state)) = item {
                if state.id == id {
                    if !name.is_empty() {
                        state.name = name;
                    }
                    state.status = status;
                    if let Some(output) = output {
                        state.output = output;
                    }
                    return;
                }
            }
        }
    }

    fn append_bash_output(&mut self, id: &str, delta: &str, running: bool) {
        if let Some(TranscriptItem::Block(TranscriptBlock::BashOutput(state))) =
            self.items.last_mut()
        {
            if state.id == id {
                state.output.push_str(delta);
                state.running = running;
                return;
            }
        }
        self.items
            .push(TranscriptItem::Block(TranscriptBlock::BashOutput(
                BashOutputBlockState {
                    id: id.to_owned(),
                    output: delta.to_owned(),
                    running,
                },
            )));
    }

    fn model_label(&self) -> String {
        self.session
            .model
            .clone()
            .or_else(|| {
                self.models
                    .get(self.model_index)
                    .map(|model| model.id.clone())
            })
            .unwrap_or_else(|| "default".to_owned())
    }

    fn supports_thinking_control(&self) -> bool {
        self.models.get(self.model_index).is_some_and(|model| {
            selectable_thinking_levels(model)
                .iter()
                .any(|level| *level != ThinkingLevel::Off)
        })
    }
}

fn selectable_thinking_levels(model: &ModelResponse) -> Vec<ThinkingLevel> {
    if model.reasoning_control == Some(ReasoningControl::OutputOnly) {
        return vec![ThinkingLevel::Off];
    }

    let mut levels = model
        .supported_reasoning_levels
        .iter()
        .filter_map(|level| ThinkingLevel::from_api_value(level))
        .filter(|level| *level != ThinkingLevel::Ultra)
        .collect::<Vec<_>>();
    levels.dedup();
    if levels.is_empty() {
        return if model.supports_thinking {
            let fallback = model
                .default_reasoning_level
                .as_deref()
                .and_then(ThinkingLevel::from_api_value)
                .filter(|level| !matches!(level, ThinkingLevel::Off | ThinkingLevel::Ultra))
                .unwrap_or(ThinkingLevel::Medium);
            if model.reasoning_is_mandatory {
                vec![fallback]
            } else {
                vec![ThinkingLevel::Off, fallback]
            }
        } else {
            vec![ThinkingLevel::Off]
        };
    }
    if model.reasoning_is_mandatory {
        levels.retain(|level| *level != ThinkingLevel::Off);
        if levels.is_empty() {
            levels.push(
                model
                    .default_reasoning_level
                    .as_deref()
                    .and_then(ThinkingLevel::from_api_value)
                    .filter(|level| !matches!(level, ThinkingLevel::Off | ThinkingLevel::Ultra))
                    .unwrap_or(ThinkingLevel::Medium),
            );
        }
    } else if !levels.contains(&ThinkingLevel::Off) {
        levels.insert(0, ThinkingLevel::Off);
    }
    levels
}

fn default_model_index(models: &[ModelResponse], default_model: Option<&str>) -> usize {
    default_model
        .and_then(|default| models.iter().position(|model| model.id == default))
        .unwrap_or(0)
}

fn sync_model_controls(session: &mut ChatSessionState, model: Option<&ModelResponse>) {
    let Some(model) = model else {
        session.model = None;
        session.fast_mode = false;
        return;
    };

    session.model = Some(model.id.clone());
    session.fast_mode &= model.supports_fast_mode;
    let levels = selectable_thinking_levels(model);
    if !levels.contains(&session.thinking_level) {
        session.thinking_level = model
            .default_reasoning_level
            .as_deref()
            .and_then(ThinkingLevel::from_api_value)
            .filter(|level| levels.contains(level))
            .unwrap_or(levels[0]);
    }
}

impl Render for ChatPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::surface())
            .child(
                div()
                    .id("chat-panel-transcript")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .child(transcript_view(&self.items)),
            )
            .children(plan_tracker(&self.plan_items))
            .when_some(self.pending_approval.clone(), |this, approval| {
                this.child(tool_approval_bar(&approval, cx))
            })
            .child(chat_composer(
                &self.input,
                &self.session,
                &self.model_label(),
                self.supports_thinking_control(),
                cx,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelResponse {
        ModelResponse {
            id: id.to_owned(),
            display_name: None,
            provider: None,
            supports_thinking: true,
            reasoning_control: Some(ReasoningControl::OpenAiEffort),
            supported_reasoning_levels: vec!["low".to_owned(), "high".to_owned()],
            default_reasoning_level: Some("high".to_owned()),
            reasoning_is_mandatory: true,
            supports_fast_mode: false,
        }
    }

    #[test]
    fn advertised_default_model_drives_initial_controls() {
        let models = vec![model("first"), model("default")];
        let index = default_model_index(&models, Some("default"));
        let mut session = ChatSessionState::new();
        session.fast_mode = true;

        sync_model_controls(&mut session, models.get(index));

        assert_eq!(index, 1);
        assert_eq!(session.model.as_deref(), Some("default"));
        assert_eq!(session.thinking_level, ThinkingLevel::High);
        assert!(!session.fast_mode);
    }

    #[test]
    fn legacy_model_metadata_uses_its_advertised_default_effort() {
        let mut legacy = model("legacy");
        legacy.supported_reasoning_levels.clear();

        assert_eq!(
            selectable_thinking_levels(&legacy),
            vec![ThinkingLevel::High]
        );

        legacy.supported_reasoning_levels = vec!["none".to_owned()];
        legacy.default_reasoning_level = None;
        assert_eq!(
            selectable_thinking_levels(&legacy),
            vec![ThinkingLevel::Medium]
        );

        legacy.reasoning_control = Some(ReasoningControl::OutputOnly);
        assert_eq!(
            selectable_thinking_levels(&legacy),
            vec![ThinkingLevel::Off]
        );
    }
}
