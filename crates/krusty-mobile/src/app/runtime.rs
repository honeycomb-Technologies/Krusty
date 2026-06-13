use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Timer};
use krusty_client::{
    ChatStreamEvent, ModelsResponse, ServerAccessResponse, SessionInfo, SessionStateResponse,
    SessionWithMessages,
};

use super::KrustyMobile;

#[derive(Debug)]
pub(super) enum BackgroundEvent {
    Models(Box<Result<ModelsResponse, String>>),
    ServerAccess(Box<Result<ServerAccessResponse, String>>),
    Sessions(Box<Result<Vec<SessionInfo>, String>>),
    LoadedSession(Box<Result<SessionWithMessages, String>>),
    SessionState(Box<Result<SessionStateResponse, String>>),
    Stream(Result<ChatStreamEvent, String>),
    StreamDone,
    Approval(Result<String, String>),
}

impl KrustyMobile {
    pub(super) fn load_models(&mut self, cx: &mut Context<Self>) {
        self.pending_background += 1;
        let client = self.client.clone();
        let tx = self.background_tx.clone();
        let runtime = Arc::clone(&self.runtime);
        std::thread::spawn(move || {
            let result = runtime
                .block_on(async move { client.list_models().await })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(BackgroundEvent::Models(Box::new(result)));
        });
        self.schedule_poll(cx);
    }

    pub(super) fn load_server_access(&mut self, cx: &mut Context<Self>) {
        self.pending_background += 1;
        let client = self.client.clone();
        let tx = self.background_tx.clone();
        let runtime = Arc::clone(&self.runtime);
        std::thread::spawn(move || {
            let result = runtime
                .block_on(async move { client.server_access().await })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(BackgroundEvent::ServerAccess(Box::new(result)));
        });
        self.schedule_poll(cx);
    }

    pub(super) fn load_sessions(&mut self, cx: &mut Context<Self>) {
        self.pending_background += 1;
        let client = self.client.clone();
        let tx = self.background_tx.clone();
        let runtime = Arc::clone(&self.runtime);
        std::thread::spawn(move || {
            let result = runtime
                .block_on(async move { client.list_sessions().await })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(BackgroundEvent::Sessions(Box::new(result)));
        });
        self.schedule_poll(cx);
    }

    pub(super) fn load_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.pending_background += 1;
        let client = self.client.clone();
        let tx = self.background_tx.clone();
        let runtime = Arc::clone(&self.runtime);
        std::thread::spawn(move || {
            let result = runtime
                .block_on(async move { client.get_session(&session_id).await })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(BackgroundEvent::LoadedSession(Box::new(result)));
        });
        self.schedule_poll(cx);
    }

    pub(super) fn load_session_state(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.pending_background += 1;
        let client = self.client.clone();
        let tx = self.background_tx.clone();
        let runtime = Arc::clone(&self.runtime);
        std::thread::spawn(move || {
            let result = runtime
                .block_on(async move { client.get_session_state(&session_id).await })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(BackgroundEvent::SessionState(Box::new(result)));
        });
        self.schedule_poll(cx);
    }

    pub(super) fn load_latest_session(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.first() {
            self.load_session(session.id.clone(), cx);
        } else {
            self.store.push_system("No server sessions available yet.");
        }
    }

    pub(super) fn refresh_runtime_state(&mut self, cx: &mut Context<Self>) {
        if let Some(session_id) = self.store.state.session_id.clone() {
            self.load_session_state(session_id, cx);
        } else {
            self.load_sessions(cx);
        }
    }

    pub(super) fn schedule_poll(&mut self, cx: &mut Context<Self>) {
        if self.poll_scheduled {
            return;
        }
        self.poll_scheduled = true;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(16)).await;
            let _ = this.update(cx, |app, cx| {
                app.poll_scheduled = false;
                app.drain_background(cx);
                if app.pending_background > 0 {
                    app.schedule_poll(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn drain_background(&mut self, cx: &mut Context<Self>) {
        while let Ok(event) = self.background_rx.try_recv() {
            match event {
                BackgroundEvent::Models(result) => {
                    self.pending_background = self.pending_background.saturating_sub(1);
                    match *result {
                        Ok(models) => self.store.set_models(models.models, models.default_model),
                        Err(error) => self
                            .store
                            .push_system(format!("Model list unavailable: {error}")),
                    }
                }
                BackgroundEvent::ServerAccess(result) => {
                    self.pending_background = self.pending_background.saturating_sub(1);
                    match *result {
                        Ok(access) => {
                            let remote = access
                                .remote_launch_url
                                .as_deref()
                                .unwrap_or("remote access disabled");
                            self.store.push_system(format!(
                                "Server access: local {} · remote {} · tailscale {}.",
                                access.local_url, remote, access.tailscale.status
                            ));
                            self.server_access = Some(access);
                        }
                        Err(error) => self
                            .store
                            .push_system(format!("Server access unavailable: {error}")),
                    }
                }
                BackgroundEvent::Sessions(result) => {
                    self.pending_background = self.pending_background.saturating_sub(1);
                    match *result {
                        Ok(mut sessions) => {
                            sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
                            let count = sessions.len();
                            self.sessions = sessions;
                            self.store
                                .push_system(format!("Loaded {count} server sessions."));
                        }
                        Err(error) => self
                            .store
                            .push_system(format!("Session list unavailable: {error}")),
                    }
                }
                BackgroundEvent::LoadedSession(result) => {
                    self.pending_background = self.pending_background.saturating_sub(1);
                    match *result {
                        Ok(snapshot) => {
                            let title = snapshot.session.title.clone();
                            let id = snapshot.session.id.clone();
                            self.store.load_session_snapshot(&snapshot);
                            self.store.push_system(format!("Loaded session {title}."));
                            self.load_session_state(id, cx);
                        }
                        Err(error) => self
                            .store
                            .push_system(format!("Session load failed: {error}")),
                    }
                }
                BackgroundEvent::SessionState(result) => {
                    self.pending_background = self.pending_background.saturating_sub(1);
                    match *result {
                        Ok(snapshot) => self.store.apply_session_state_snapshot(&snapshot),
                        Err(error) => self
                            .store
                            .push_system(format!("Session state unavailable: {error}")),
                    }
                }
                BackgroundEvent::Stream(Ok(event)) => self.store.apply_stream_event(event),
                BackgroundEvent::Stream(Err(error)) => self.store.fail_stream(error),
                BackgroundEvent::StreamDone => {
                    self.pending_background = self.pending_background.saturating_sub(1);
                    if self.store.state.is_streaming {
                        self.store.finish_stream();
                    }
                    self.load_sessions(cx);
                }
                BackgroundEvent::Approval(result) => {
                    self.pending_background = self.pending_background.saturating_sub(1);
                    let session_id = self.store.state.session_id.clone();
                    match result {
                        Ok(message) => self.store.push_system(message),
                        Err(error) => self.store.push_system(format!("Approval failed: {error}")),
                    }
                    if let Some(session_id) = session_id {
                        self.load_session_state(session_id, cx);
                    }
                }
            }
        }

        while let Some(action) = self.store.pop_shell_action() {
            self.describe_shell_action(action);
        }

        if self.pending_background > 0 {
            self.schedule_poll(cx);
        }
    }
}
