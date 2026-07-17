mod runtime;

use futures_util::StreamExt as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, AnyElement, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};
use gpui_component::input::{Input, InputEvent, InputState};
use krusty_client::{KrustyClient, ServerAccessResponse, SessionInfo};
use krusty_client_state::{ChatStore, MobileSurface, ShellAction};
use krusty_mobile_ui::theme;
use krusty_mobile_ui::transcript::transcript_view;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use tokio::runtime::Runtime;

use self::runtime::BackgroundEvent;

const DEFAULT_SERVER: &str = "http://127.0.0.1:3000";

pub struct KrustyMobile {
    client: KrustyClient,
    runtime: Arc<Runtime>,
    input: Entity<InputState>,
    store: ChatStore,
    sessions: Vec<SessionInfo>,
    server_access: Option<ServerAccessResponse>,
    accordion_open: bool,
    model_picker_open: bool,
    attach_sheet_open: bool,
    pending_background: usize,
    poll_scheduled: bool,
    background_tx: Sender<BackgroundEvent>,
    background_rx: Receiver<BackgroundEvent>,
    stop_requested: Arc<AtomicBool>,
    _input_subscription: Subscription,
}

impl KrustyMobile {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Message Krusty…")
                .auto_grow(1, 6)
        });
        let input_subscription =
            cx.subscribe_in(&input, window, |app, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { secondary: false }) {
                    app.submit(input, window, cx);
                }
            });

        let server =
            std::env::var("KRUSTY_MOBILE_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_owned());
        let client = KrustyClient::new(server).unwrap_or_else(|_| {
            KrustyClient::local().expect("local Krusty client should construct")
        });
        let runtime = Arc::new(Runtime::new().expect("mobile Tokio runtime should start"));
        let (background_tx, background_rx) = mpsc::channel();
        let mut app = Self {
            client,
            runtime,
            input,
            store: ChatStore::default(),
            sessions: Vec::new(),
            server_access: None,
            accordion_open: false,
            model_picker_open: false,
            attach_sheet_open: false,
            pending_background: 0,
            poll_scheduled: false,
            background_tx,
            background_rx,
            stop_requested: Arc::new(AtomicBool::new(false)),
            _input_subscription: input_subscription,
        };
        app.store.push_system(format!(
            "Connected to {}. Set KRUSTY_MOBILE_SERVER to target a Mac/server.",
            app.client.base_url()
        ));
        app.load_models(cx);
        app.load_server_access(cx);
        app.load_sessions(cx);
        if let Ok(session_id) = std::env::var("KRUSTY_MOBILE_SESSION_ID") {
            if !session_id.trim().is_empty() {
                app.load_session(session_id, cx);
            }
        }
        app
    }

    fn describe_shell_action(&mut self, action: ShellAction) {
        match action {
            ShellAction::PickAttachment => {
                self.attach_sheet_open = true;
                self.store.push_system(
                    "Swift shell will own Photos/Files/Camera attachment picking on iOS.",
                );
            }
            ShellAction::OpenBrowser { url } => self.store.push_system(format!(
                "Browser bridge requested for {url}. Swift WKWebView owns this surface."
            )),
            ShellAction::OpenTerminal { session_id } => self.store.push_system(format!(
                "Terminal bridge requested for {}. Swift WKWebView terminal owns input/viewport.",
                session_id.unwrap_or_else(|| "new session".to_owned())
            )),
            ShellAction::OpenLocalRuntimeSpike => self.store.push_system(
                "litter-ish is tracked as a separate local-runtime spike, not linked into this app.",
            ),
        }
    }

    fn submit(&mut self, input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.state.is_streaming {
            return;
        }
        let text = input.read(cx).value().trim().to_owned();
        if text.is_empty() && self.store.state.attachments.is_empty() {
            return;
        }

        let request = self.store.chat_request_for(text.clone());
        self.store.submit_user_message(text);
        input.update(cx, |input, cx| input.set_value("", window, cx));
        self.stop_requested = Arc::new(AtomicBool::new(false));
        self.pending_background += 1;
        self.model_picker_open = false;
        self.attach_sheet_open = false;

        let client = self.client.clone();
        let tx = self.background_tx.clone();
        let runtime = Arc::clone(&self.runtime);
        let stop_flag = Arc::clone(&self.stop_requested);
        std::thread::spawn(move || {
            runtime.block_on(async move {
                match client.chat_stream(request).await {
                    Ok(mut stream) => {
                        while let Some(event) = stream.next().await {
                            if stop_flag.load(Ordering::SeqCst) {
                                break;
                            }
                            let mapped = event.map_err(|error| format!("{error:#}"));
                            let _ = tx.send(BackgroundEvent::Stream(mapped));
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(BackgroundEvent::Stream(Err(format!("{error:#}"))));
                    }
                }
                let _ = tx.send(BackgroundEvent::StreamDone);
            });
        });
        self.schedule_poll(cx);
        cx.notify();
    }

    fn stop_stream(&mut self, cx: &mut Context<Self>) {
        self.stop_requested.store(true, Ordering::SeqCst);
        self.store.finish_stream();
        cx.notify();
    }

    fn send_tool_approval(&mut self, approved: bool, cx: &mut Context<Self>) {
        let Some(session_id) = self.store.state.session_id.clone() else {
            self.store
                .push_system("Cannot approve before a session id exists.");
            cx.notify();
            return;
        };
        let Some(approval) = self.store.state.pending_approval.clone() else {
            return;
        };
        self.store.state.pending_approval = None;
        self.pending_background += 1;

        let client = self.client.clone();
        let tx = self.background_tx.clone();
        let runtime = Arc::clone(&self.runtime);
        std::thread::spawn(move || {
            let tool_call_id = approval.tool_call_id;
            let request_tool_call_id = tool_call_id.clone();
            let label = if approved { "Approved" } else { "Denied" };
            let result = runtime
                .block_on(async move {
                    client
                        .approve_tool(&session_id, &request_tool_call_id, approved)
                        .await
                })
                .map(|_| format!("{label} tool call {tool_call_id}."))
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(BackgroundEvent::Approval(result));
        });
        self.schedule_poll(cx);
        cx.notify();
    }

    fn model_label(&self) -> String {
        let Some(selected) = &self.store.state.controls.selected_model else {
            return "default".to_owned();
        };
        self.store
            .state
            .models
            .iter()
            .find(|model| &model.id == selected)
            .map(|model| model.label().to_owned())
            .unwrap_or_else(|| selected.clone())
    }

    fn runtime_summary(&self) -> String {
        let active = self
            .store
            .state
            .session_id
            .as_deref()
            .map(|id| format!("active {id}"))
            .unwrap_or_else(|| "no active session".to_owned());
        let remote = self
            .server_access
            .as_ref()
            .and_then(|access| access.remote_launch_url.as_deref())
            .unwrap_or("no remote URL");
        format!("{} sessions · {active} · {remote}", self.sessions.len())
    }

    fn top_action(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
        on_press: fn(&mut Self, &mut Context<Self>),
    ) -> AnyElement {
        div()
            .id(id)
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::app_bg())
            .px_2()
            .py_1()
            .text_xs()
            .text_color(theme::text_muted())
            .child(label)
            .on_click(cx.listener(move |app, _, _, cx| on_press(app, cx)))
            .into_any_element()
    }

    fn render_top_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .border_b_1()
            .border_color(theme::hairline())
            .bg(theme::surface())
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child("Krusty Mobile"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child(self.client.base_url().to_owned()),
                    ),
            )
            .child(
                div().flex().gap_1().children(
                    MobileSurface::ALL
                        .into_iter()
                        .map(|surface| self.surface_tab(surface, cx)),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child(self.runtime_summary()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(self.top_action("latest-session", "Latest", cx, |app, cx| {
                                app.load_latest_session(cx);
                                cx.notify();
                            }))
                            .child(self.top_action(
                                "refresh-sessions",
                                "Sessions",
                                cx,
                                |app, cx| {
                                    app.load_sessions(cx);
                                    cx.notify();
                                },
                            ))
                            .child(self.top_action("refresh-state", "State", cx, |app, cx| {
                                app.refresh_runtime_state(cx);
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn surface_tab(&self, surface: MobileSurface, cx: &mut Context<Self>) -> AnyElement {
        let active = self.store.state.surface == surface;
        div()
            .id(match surface {
                MobileSurface::Chat => "surface-chat",
                MobileSurface::Folder => "surface-folder",
                MobileSurface::Research => "surface-research",
                MobileSurface::Paper => "surface-paper",
                MobileSurface::Terminal => "surface-terminal",
                MobileSurface::Browser => "surface-browser",
            })
            .border_1()
            .border_color(if active {
                theme::accent()
            } else {
                theme::hairline()
            })
            .bg(if active {
                theme::surface_selected()
            } else {
                theme::app_bg()
            })
            .px_2()
            .py_1()
            .text_xs()
            .text_color(if active {
                theme::text()
            } else {
                theme::text_muted()
            })
            .child(surface.label())
            .on_click(cx.listener(move |app, _, _, cx| {
                app.store.set_surface(surface);
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_content(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.store.state.surface {
            MobileSurface::Terminal => self.bridge_placeholder(
                "Terminal",
                "WKWebView terminal bridge",
                "Remote/server terminal first; litter-ish local Linux stays an isolated spike.",
                cx,
            ),
            MobileSurface::Browser => self.bridge_placeholder(
                "Browser",
                "WKWebView browser bridge",
                "Use this for OAuth, previews, and port/browser sessions before Servo.",
                cx,
            ),
            _ => div()
                .id("mobile-transcript")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_3()
                .child(transcript_view(&self.store.state.transcript))
                .into_any_element(),
        }
    }

    fn bridge_placeholder(
        &self,
        title: &str,
        subtitle: &str,
        detail: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex_1()
            .p_3()
            .child(
                div()
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::surface_selected())
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(title.to_owned()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text())
                            .child(subtitle.to_owned()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text_muted())
                            .child(detail.to_owned()),
                    )
                    .child(
                        div()
                            .id("local-runtime-spike")
                            .border_1()
                            .border_color(theme::complement())
                            .px_2()
                            .py_1()
                            .text_sm()
                            .text_color(theme::complement())
                            .child("Track litter-ish spike")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.store.state.surface = MobileSurface::Chat;
                                app.store.pop_shell_action();
                                app.store.push_system(
                                    "See experiments/litter-ish-spike for local runtime gates.",
                                );
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_plan_and_approval(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(self.store.state.plan_items.iter().map(|item| {
                div()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(if item.completed {
                        theme::success()
                    } else {
                        theme::text_muted()
                    })
                    .child(format!(
                        "{} {}",
                        if item.completed { "✓" } else { "□" },
                        item.content
                    ))
            }))
            .when_some(
                self.store.state.pending_approval.clone(),
                |this, approval| {
                    this.child(
                        div()
                            .border_t_1()
                            .border_color(theme::complement())
                            .bg(theme::surface())
                            .p_2()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::text())
                                    .child(format!("Approve {}?", approval.tool_name)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .id("deny-tool")
                                            .border_1()
                                            .border_color(theme::danger())
                                            .px_2()
                                            .py_1()
                                            .text_xs()
                                            .text_color(theme::danger())
                                            .child("Deny")
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.send_tool_approval(false, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("approve-tool")
                                            .border_1()
                                            .border_color(theme::success())
                                            .px_2()
                                            .py_1()
                                            .text_xs()
                                            .text_color(theme::success())
                                            .child("Approve")
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.send_tool_approval(true, cx)
                                            })),
                                    ),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .border_t_1()
            .border_color(theme::hairline())
            .bg(theme::surface())
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .when(self.model_picker_open, |this| {
                this.child(self.render_model_picker(cx))
            })
            .when(self.attach_sheet_open, |this| {
                this.child(self.render_attach_sheet(cx))
            })
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .border_1()
                            .border_color(thinking_border_color(
                                self.store.state.controls.thinking_level,
                            ))
                            .bg(theme::app_bg())
                            .child(Input::new(&self.input).appearance(false)),
                    )
                    .child(self.render_crab_column(cx)),
            )
            .into_any_element()
    }

    fn render_crab_column(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w(px(56.0))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .when(self.accordion_open, |this| {
                this.child(
                    self.control_pill("model-pill", "Bot", false, cx, |app, cx| {
                        app.model_picker_open = !app.model_picker_open;
                        app.attach_sheet_open = false;
                        cx.notify();
                    }),
                )
                .child(
                    self.control_pill("attach-pill", "Clip", false, cx, |app, cx| {
                        app.store.queue_attachment_picker();
                        app.attach_sheet_open = !app.attach_sheet_open;
                        app.model_picker_open = false;
                        cx.notify();
                    }),
                )
                .child(self.control_pill(
                    "research-pill",
                    "Lab",
                    self.store.state.controls.research_enabled,
                    cx,
                    |app, cx| {
                        app.store.toggle_research();
                        cx.notify();
                    },
                ))
                .child(self.control_pill(
                    "mode-pill",
                    self.store.state.controls.work_mode.label(),
                    self.store.state.controls.work_mode == krusty_client::WorkMode::Plan,
                    cx,
                    |app, cx| {
                        app.store.toggle_work_mode();
                        cx.notify();
                    },
                ))
                .child(self.control_pill(
                    "permission-pill",
                    self.store.state.controls.permission_mode.label(),
                    self.store.state.controls.permission_mode
                        == krusty_client::PermissionMode::Autonomous,
                    cx,
                    |app, cx| {
                        app.store.toggle_permission_mode();
                        cx.notify();
                    },
                ))
                .child(self.control_pill(
                    "fast-pill",
                    if self.store.state.controls.fast_mode {
                        "Fast"
                    } else {
                        "Std"
                    },
                    self.store.state.controls.fast_mode,
                    cx,
                    |app, cx| {
                        app.store.toggle_fast_mode();
                        cx.notify();
                    },
                ))
                .child(self.control_pill(
                    "thinking-pill",
                    self.store.state.controls.thinking_level.label(),
                    self.store.state.controls.thinking_level != krusty_client::ThinkingLevel::Off,
                    cx,
                    |app, cx| {
                        app.store.cycle_thinking();
                        cx.notify();
                    },
                ))
            })
            .child(
                div()
                    .id("crab-toggle")
                    .w(px(56.0))
                    .h(px(56.0))
                    .border_1()
                    .border_color(if self.accordion_open {
                        theme::complement()
                    } else {
                        theme::hairline()
                    })
                    .bg(if self.accordion_open {
                        theme::surface_selected()
                    } else {
                        theme::app_bg()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_lg()
                    .text_color(if self.accordion_open {
                        theme::complement()
                    } else {
                        theme::text_muted()
                    })
                    .child("🦀")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.accordion_open = !app.accordion_open;
                        if !app.accordion_open {
                            app.model_picker_open = false;
                            app.attach_sheet_open = false;
                        }
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn control_pill(
        &self,
        id: &'static str,
        label: impl Into<String>,
        active: bool,
        cx: &mut Context<Self>,
        on_press: fn(&mut Self, &mut Context<Self>),
    ) -> AnyElement {
        div()
            .id(id)
            .w(px(56.0))
            .h(px(42.0))
            .border_1()
            .border_color(if active {
                theme::complement()
            } else {
                theme::hairline()
            })
            .bg(if active {
                theme::surface_selected()
            } else {
                theme::app_bg()
            })
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(if active {
                theme::text()
            } else {
                theme::text_muted()
            })
            .child(label.into())
            .on_click(cx.listener(move |app, _, _, cx| on_press(app, cx)))
            .into_any_element()
    }

    fn render_model_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::app_bg())
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_muted())
                    .child(format!("Model: {}", self.model_label())),
            )
            .children(
                self.store
                    .state
                    .models
                    .iter()
                    .take(8)
                    .enumerate()
                    .map(|(index, model)| {
                        let model_id = model.id.clone();
                        let active =
                            self.store.state.controls.selected_model.as_ref() == Some(&model.id);
                        div()
                            .id(("model", index))
                            .border_1()
                            .border_color(if active {
                                theme::accent()
                            } else {
                                theme::hairline()
                            })
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(if active {
                                theme::text()
                            } else {
                                theme::text_muted()
                            })
                            .child(format!("{} · {}", model.label(), model.provider))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.store.select_model(model_id.clone());
                                app.model_picker_open = false;
                                cx.notify();
                            }))
                    }),
            )
            .into_any_element()
    }

    fn render_attach_sheet(&self, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::app_bg())
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_muted())
                    .child("Attachments"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme::text())
                    .child("Photos · Camera · Files will be resolved by the Swift shell."),
            )
            .into_any_element()
    }
}

impl Render for KrustyMobile {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(theme::app_bg())
            .flex()
            .flex_col()
            .child(self.render_top_bar(cx))
            .child(self.render_content(cx))
            .child(self.render_plan_and_approval(cx))
            .child(
                div()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .bg(theme::surface())
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child(format!(
                                "{} · {}",
                                self.store.state.surface.label(),
                                self.model_label()
                            )),
                    )
                    .child(if self.store.state.is_streaming {
                        div()
                            .id("stop-stream")
                            .border_1()
                            .border_color(theme::danger())
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme::danger())
                            .child("Stop")
                            .on_click(cx.listener(|app, _, _, cx| app.stop_stream(cx)))
                            .into_any_element()
                    } else {
                        div()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child("Enter sends · Shift+Enter newline")
                            .into_any_element()
                    }),
            )
            .child(self.render_composer(cx))
    }
}

fn thinking_border_color(level: krusty_client::ThinkingLevel) -> gpui::Hsla {
    match level {
        krusty_client::ThinkingLevel::Off => theme::hairline(),
        krusty_client::ThinkingLevel::Minimal => theme::complement().opacity(0.25),
        krusty_client::ThinkingLevel::Low => theme::complement().opacity(0.35),
        krusty_client::ThinkingLevel::Medium => theme::complement().opacity(0.55),
        krusty_client::ThinkingLevel::High => theme::complement().opacity(0.75),
        krusty_client::ThinkingLevel::XHigh => theme::complement(),
        krusty_client::ThinkingLevel::Max | krusty_client::ThinkingLevel::Ultra => {
            theme::complement()
        }
    }
}
