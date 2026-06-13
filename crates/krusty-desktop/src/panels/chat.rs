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

use crate::api::{ChatRequest, ChatStreamEvent, KrustyApiClient, ModelResponse, PlanItem};
use crate::chat::session::ChatSessionState;
use crate::components::chat::approval_bar::tool_approval_bar;
use crate::components::chat::blocks::bash_output::BashOutputBlockState;
use crate::components::chat::blocks::thinking::ThinkingBlockState;
use crate::components::chat::blocks::tool_call::{ToolCallBlockState, ToolCallStatus};
use crate::components::chat::blocks::TranscriptBlock;
use crate::components::chat::composer::chat_composer;
use crate::components::chat::plan_tracker::plan_tracker;
use crate::components::chat::transcript::{transcript_view, TranscriptItem};
use crate::design::theme;

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

        let models = client
            .list_models()
            .map(|response| response.models)
            .unwrap_or_default();
        let model_index = 0;
        let server = client.base_url().to_owned();
        let mut panel = Self {
            client,
            input,
            session: ChatSessionState::new(),
            items: Vec::new(),
            models,
            model_index,
            plan_items: Vec::new(),
            pending_approval: None,
            stop_requested: Arc::new(AtomicBool::new(false)),
            _input_subscription: input_subscription,
        };
        panel.items.push(TranscriptItem::System(format!(
            "Chat ready. Streaming through {server}/api/chat."
        )));
        panel
    }

    pub fn set_client(&mut self, client: KrustyApiClient) {
        if self.client.base_url() == client.base_url() {
            return;
        }
        let server = client.base_url().to_owned();
        self.client = client;
        self.models = self
            .client
            .list_models()
            .map(|response| response.models)
            .unwrap_or_default();
        self.model_index = 0;
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
        if self.models.is_empty() {
            return;
        }
        self.model_index = (self.model_index + 1) % self.models.len();
        self.session.model = self.models.get(self.model_index).map(|m| m.id.clone());
        cx.notify();
    }

    pub fn cycle_thinking(&mut self, cx: &mut Context<Self>) {
        self.session.thinking_level = self.session.thinking_level.cycle();
        cx.notify();
    }

    pub fn toggle_permission(&mut self, cx: &mut Context<Self>) {
        self.session.permission_mode = self.session.permission_mode.toggle();
        cx.notify();
    }

    pub fn toggle_fast_mode(&mut self, cx: &mut Context<Self>) {
        self.session.fast_mode = !self.session.fast_mode;
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
                cx,
            ))
    }
}
