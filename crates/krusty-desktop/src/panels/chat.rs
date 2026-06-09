use gpui::{
    div, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::StyledExt as _;

use crate::api::KrustyApiClient;
use crate::components::input::krusty_input;
use crate::design::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChatLineKind {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChatLine {
    kind: ChatLineKind,
    text: String,
}

pub struct ChatPanel {
    client: KrustyApiClient,
    input: Entity<InputState>,
    lines: Vec<ChatLine>,
    session_id: Option<String>,
    streaming: bool,
    _input_subscription: Subscription,
}

impl ChatPanel {
    pub fn new(client: KrustyApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Ask Krusty…")
                .auto_grow(1, 4)
        });
        let input_subscription =
            cx.subscribe_in(&input, window, |panel, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: false }) {
                    panel.submit(input, window, cx);
                }
            });

        let server = client.base_url().to_owned();
        Self {
            client,
            input,
            lines: vec![ChatLine {
                kind: ChatLineKind::System,
                text: format!("Chat panel ready. Streaming through {server}/api/chat."),
            }],
            session_id: None,
            streaming: false,
            _input_subscription: input_subscription,
        }
    }

    pub fn set_client(&mut self, client: KrustyApiClient) {
        if self.client.base_url() == client.base_url() {
            return;
        }
        let server = client.base_url().to_owned();
        self.client = client;
        self.session_id = None;
        self.lines.push(ChatLine {
            kind: ChatLineKind::System,
            text: format!("Server changed to {server}; the next message starts a new session."),
        });
    }

    fn submit(&mut self, input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        let text = input.read(cx).value().trim().to_owned();
        if text.is_empty() || self.streaming {
            return;
        }

        input.update(cx, |input, cx| input.set_value("", window, cx));
        self.lines.push(ChatLine {
            kind: ChatLineKind::User,
            text: text.clone(),
        });
        let response_index = self.lines.len();
        self.lines.push(ChatLine {
            kind: ChatLineKind::Assistant,
            text: "Streaming from Krusty server…".to_owned(),
        });
        self.streaming = true;
        cx.notify();

        let client = self.client.clone();
        let session_id = self.session_id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.send_chat_collect(session_id, text) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.streaming = false;
                match result {
                    Ok(result) => {
                        if result.session_id.is_some() {
                            panel.session_id = result.session_id;
                        }
                        if let Some(line) = panel.lines.get_mut(response_index) {
                            line.text = if result.text.trim().is_empty() {
                                "Turn completed without assistant text.".to_owned()
                            } else {
                                result.text
                            };
                        }
                        if let Some(title) = result.title {
                            panel.lines.push(ChatLine {
                                kind: ChatLineKind::System,
                                text: format!("Session title updated: {title}"),
                            });
                        }
                    }
                    Err(error) => {
                        if let Some(line) = panel.lines.get_mut(response_index) {
                            line.text = format!("Chat request failed: {error:#}");
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
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
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(self.lines.iter().map(render_line)),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .p_2()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().flex_1().child(krusty_input(&self.input)))
                    .child(
                        div()
                            .id("chat-panel-send")
                            .px_3()
                            .py_1()
                            .border_1()
                            .border_color(theme::hairline())
                            .bg(theme::app_bg())
                            .hover(|style| style.bg(theme::surface_hover()))
                            .cursor_pointer()
                            .text_sm()
                            .child("Send")
                            .on_click(cx.listener(|panel, _, window, cx| {
                                let input = panel.input.clone();
                                panel.submit(&input, window, cx);
                            })),
                    ),
            )
    }
}

fn render_line(line: &ChatLine) -> gpui::Div {
    let (label, border, bg) = match line.kind {
        ChatLineKind::User => ("You", theme::accent(), theme::app_bg()),
        ChatLineKind::Assistant => ("Krusty", theme::hairline(), theme::surface_selected()),
        ChatLineKind::System => ("System", theme::hairline(), theme::app_bg()),
    };

    div()
        .border_1()
        .border_color(border)
        .bg(bg)
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(theme::text_muted())
                .child(label),
        )
        .child(div().text_sm().child(line.text.clone()))
}
