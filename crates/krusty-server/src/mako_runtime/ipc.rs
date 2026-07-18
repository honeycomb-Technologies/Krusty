use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use krusty_core::storage::RuntimeTraceEvent;
use krusty_mako_protocol::{
    AckResponse, Actor, ClientError, Command, DaemonStats, DispatchCommand, EventEnvelope,
    EventSubscription, MakoEvent, MakoIpcClient, MakoIpcClientConfig, MessageCommand,
    RecoverCommand, RequestEnvelope, ResponsePayload, ScheduleCommand, SessionCommand,
    SetCrewCommand, SetPriorityCommand, SteerCommand, SubscribeCommand, ToolApprovalCommand,
    UserResponseCommand,
};

use super::MakoRuntimeStats;
use crate::types::AgenticEvent;

const CLIENT_ID: &str = "krusty-server";
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(super) enum MakoDaemonError {
    Remote { code: String, message: String },
    Unavailable(String),
}

impl std::fmt::Display for MakoDaemonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remote { code, message } => write!(formatter, "{code}: {message}"),
            Self::Unavailable(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for MakoDaemonError {}

impl From<ClientError> for MakoDaemonError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Remote { code, message } => Self::Remote { code, message },
            error => Self::Unavailable(error.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct MakoDaemonControl {
    client: MakoIpcClient,
}

impl MakoDaemonControl {
    pub(super) async fn connect_discovered() -> Result<Self> {
        let socket_path = discover_socket_path()?;
        let key_path = discover_key_path();
        let mut config = MakoIpcClientConfig::new(socket_path.clone(), CLIENT_ID);
        config.request_timeout = DEFAULT_REQUEST_TIMEOUT;
        let client = MakoIpcClient::from_key_path(config, &key_path)
            .with_context(|| format!("loading Mako daemon IPC key from {}", key_path.display()))?;
        let control = Self { client };
        control
            .healthcheck()
            .await
            .with_context(|| format!("Mako daemon unavailable at {}", socket_path.display()))?;
        Ok(control)
    }

    #[cfg(test)]
    pub(super) async fn connect_client(client: MakoIpcClient) -> Result<Self> {
        let control = Self { client };
        control.healthcheck().await?;
        Ok(control)
    }

    async fn healthcheck(&self) -> Result<()> {
        match self
            .command(None, Command::Ping, Some(unique_key("healthcheck")))
            .await?
        {
            ResponsePayload::Pong(_) => Ok(()),
            payload => bail!("Mako healthcheck returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn start(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        wake_reason: &str,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            start_command(session_id),
            unique_key(&format!("start:{session_id}:{wake_reason}")),
            "start session",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch(
        &self,
        user_id: Option<&str>,
        task: &str,
        working_dir: &str,
        project_dir: Option<&str>,
        model: Option<&str>,
        start_at_unix_ms: Option<i64>,
        priority: Option<&str>,
        crew_slug: Option<&str>,
    ) -> Result<(String, String)> {
        let command = dispatch_command(
            task,
            working_dir,
            project_dir,
            model,
            start_at_unix_ms,
            priority,
            crew_slug,
        );
        match self
            .command(user_id, command, Some(unique_key("dispatch")))
            .await?
        {
            ResponsePayload::Dispatch(response) => Ok((response.session_id, response.status)),
            payload => bail!("Mako dispatch returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn resume(&self, user_id: Option<&str>, session_id: &str) -> Result<()> {
        self.expect_ack(
            user_id,
            resume_command(session_id),
            unique_key(&format!("resume:{session_id}")),
            "resume session",
        )
        .await
    }

    pub(super) async fn pause(&self, user_id: Option<&str>, session_id: &str) -> Result<()> {
        self.expect_ack(
            user_id,
            pause_command(session_id),
            unique_key(&format!("pause:{session_id}")),
            "pause session",
        )
        .await
    }

    pub(super) async fn schedule(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        wake_at_unix_ms: i64,
        reason: &str,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            schedule_command(session_id, wake_at_unix_ms, reason),
            unique_key(&format!("schedule:{session_id}:{wake_at_unix_ms}")),
            "schedule session",
        )
        .await
    }

    pub(super) async fn cancel(&self, user_id: Option<&str>, session_id: &str) -> Result<()> {
        self.expect_ack(
            user_id,
            cancel_command(session_id),
            unique_key(&format!("cancel:{session_id}")),
            "cancel session",
        )
        .await
    }

    pub(super) async fn delete(&self, user_id: Option<&str>, session_id: &str) -> Result<()> {
        self.expect_ack(
            user_id,
            delete_command(session_id),
            stable_key("delete", session_id),
            "delete session",
        )
        .await
    }

    pub(super) async fn send_message(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        message: &str,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            message_command(session_id, message),
            unique_key(&format!("message:{session_id}")),
            "send message",
        )
        .await
    }

    pub(super) async fn set_priority(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        priority: &str,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            priority_command(session_id, priority),
            unique_key(&format!("priority:{session_id}:{priority}")),
            "set priority",
        )
        .await
    }

    pub(super) async fn set_crew(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        crew_slug: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            crew_command(session_id, crew_slug),
            unique_key(&format!(
                "crew:{session_id}:{}",
                crew_slug.unwrap_or("none")
            )),
            "set crew",
        )
        .await
    }

    pub(super) async fn recover(
        &self,
        user_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<usize> {
        let key = unique_key(&format!("recover:{}", session_id.unwrap_or("all")));
        match self
            .command(user_id, recover_command(session_id), Some(key))
            .await?
        {
            ResponsePayload::Recover(response) => Ok(response.recovered_count),
            payload => bail!("Mako recover returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn stats(&self, user_id: Option<&str>) -> Result<MakoRuntimeStats> {
        match self
            .command(user_id, Command::Stats, Some(unique_key("stats")))
            .await?
        {
            ResponsePayload::Stats(stats) => Ok(map_stats(stats)),
            payload => bail!("Mako stats returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn subscribe(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        after_sequence: Option<i64>,
        replay_limit: Option<usize>,
    ) -> Result<EventSubscription> {
        let command = subscribe_command(session_id, after_sequence, replay_limit);
        let mut request = RequestEnvelope::new(actor(user_id), command, 30_000);
        request.idempotency_key = unique_key(&format!("subscribe:{session_id}"));
        self.client
            .subscribe(request)
            .await
            .map_err(MakoDaemonError::from)
            .context("subscribing to Mako daemon events")
    }

    pub(super) async fn steer(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        pending_id: &str,
        content: serde_json::Value,
    ) -> Result<AckResponse> {
        match self
            .command(
                user_id,
                steer_command(session_id, pending_id, content),
                Some(stable_key("steer", pending_id)),
            )
            .await?
        {
            ResponsePayload::Ack(response) => Ok(response),
            payload => bail!("Mako steer returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn tool_approval(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        tool_call_id: &str,
        approved: bool,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            tool_approval_command(session_id, tool_call_id, approved),
            stable_key("approval", &format!("{session_id}:{tool_call_id}")),
            "submit tool approval",
        )
        .await
    }

    pub(super) async fn user_response(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        tool_call_id: &str,
        response: &str,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            user_response_command(session_id, tool_call_id, response),
            stable_key("response", &format!("{session_id}:{tool_call_id}")),
            "submit user response",
        )
        .await
    }

    async fn expect_ack(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: String,
        operation: &str,
    ) -> Result<()> {
        match self
            .command(user_id, command, Some(idempotency_key))
            .await?
        {
            ResponsePayload::Ack(ack) if ack.accepted => Ok(()),
            ResponsePayload::Ack(ack) => Err(MakoDaemonError::Remote {
                code: "conflict".to_string(),
                message: format!(
                    "Mako daemon declined to {operation}: {}",
                    ack.message
                        .unwrap_or_else(|| "no reason provided".to_string())
                ),
            }
            .into()),
            payload => bail!("Mako {operation} returned unexpected response {payload:?}"),
        }
    }

    async fn command(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: Option<String>,
    ) -> Result<ResponsePayload> {
        self.client
            .command(actor(user_id), command, idempotency_key)
            .await
            .map_err(MakoDaemonError::from)
            .context("Mako daemon command failed")
    }
}

fn actor(user_id: Option<&str>) -> Actor {
    Actor {
        user_id: user_id.map(ToOwned::to_owned),
        client_kind: CLIENT_ID.to_string(),
    }
}

fn start_command(session_id: &str) -> Command {
    Command::StartSession(SessionCommand {
        session_id: session_id.to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
fn dispatch_command(
    task: &str,
    working_dir: &str,
    project_dir: Option<&str>,
    model: Option<&str>,
    start_at_unix_ms: Option<i64>,
    priority: Option<&str>,
    crew_slug: Option<&str>,
) -> Command {
    Command::Dispatch(DispatchCommand {
        task: task.to_string(),
        working_dir: working_dir.to_string(),
        project_dir: project_dir.map(ToOwned::to_owned),
        model: model.map(ToOwned::to_owned),
        start_at_unix_ms,
        priority: priority.map(ToOwned::to_owned),
        crew_slug: crew_slug.map(ToOwned::to_owned),
    })
}

fn resume_command(session_id: &str) -> Command {
    Command::ResumeSession(SessionCommand {
        session_id: session_id.to_string(),
    })
}

fn pause_command(session_id: &str) -> Command {
    Command::PauseSession(SessionCommand {
        session_id: session_id.to_string(),
    })
}

fn schedule_command(session_id: &str, wake_at_unix_ms: i64, reason: &str) -> Command {
    Command::ScheduleSession(ScheduleCommand {
        session_id: session_id.to_string(),
        wake_at_unix_ms,
        reason: reason.to_string(),
    })
}

fn cancel_command(session_id: &str) -> Command {
    Command::CancelSession(SessionCommand {
        session_id: session_id.to_string(),
    })
}

fn delete_command(session_id: &str) -> Command {
    Command::DeleteSession(SessionCommand {
        session_id: session_id.to_string(),
    })
}

fn message_command(session_id: &str, message: &str) -> Command {
    Command::SendMessage(MessageCommand {
        session_id: session_id.to_string(),
        message: message.to_string(),
    })
}

fn priority_command(session_id: &str, priority: &str) -> Command {
    Command::SetPriority(SetPriorityCommand {
        session_id: session_id.to_string(),
        priority: priority.to_string(),
    })
}

fn crew_command(session_id: &str, crew_slug: Option<&str>) -> Command {
    Command::SetCrew(SetCrewCommand {
        session_id: session_id.to_string(),
        crew_slug: crew_slug.map(ToOwned::to_owned),
    })
}

fn recover_command(session_id: Option<&str>) -> Command {
    Command::Recover(RecoverCommand {
        session_id: session_id.map(ToOwned::to_owned),
    })
}

fn subscribe_command(
    session_id: &str,
    after_sequence: Option<i64>,
    replay_limit: Option<usize>,
) -> Command {
    Command::Subscribe(SubscribeCommand {
        session_id: session_id.to_string(),
        after_sequence,
        replay_limit,
    })
}

fn steer_command(session_id: &str, pending_id: &str, content: serde_json::Value) -> Command {
    Command::Steer(SteerCommand {
        session_id: session_id.to_string(),
        pending_id: Some(pending_id.to_string()),
        content,
    })
}

fn tool_approval_command(session_id: &str, tool_call_id: &str, approved: bool) -> Command {
    Command::ToolApproval(ToolApprovalCommand {
        session_id: session_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        approved,
    })
}

fn user_response_command(session_id: &str, tool_call_id: &str, response: &str) -> Command {
    Command::UserResponse(UserResponseCommand {
        session_id: session_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        response: response.to_string(),
    })
}

fn stable_key(operation: &str, identity: &str) -> String {
    let candidate = format!("{operation}:{identity}");
    if candidate.len() <= 256 {
        candidate
    } else {
        unique_key(operation)
    }
}

fn unique_key(operation: &str) -> String {
    format!("{operation}:{}", uuid::Uuid::new_v4())
}

fn map_stats(stats: DaemonStats) -> MakoRuntimeStats {
    MakoRuntimeStats {
        active_runtime_count: runtime_usize(&stats.runtime, "active_runtime_count"),
        scheduled_wake_count: runtime_usize(&stats.runtime, "scheduled_wake_count"),
        event_stream_count: runtime_usize(&stats.runtime, "event_stream_count"),
        uptime_secs: stats.uptime_secs,
    }
}

fn runtime_usize(runtime: &serde_json::Value, key: &str) -> usize {
    runtime
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}

pub(super) fn map_daemon_event(envelope: EventEnvelope) -> AgenticEvent {
    let EventEnvelope {
        session_id,
        run_id,
        sequence,
        emitted_at_unix_ms,
        event,
        ..
    } = envelope;
    match event {
        MakoEvent::Runtime(runtime) => {
            let event_type = runtime.event_type;
            let payload = runtime.payload;
            AgenticEvent::from_runtime_trace(RuntimeTraceEvent {
                run_id: run_id.clone().unwrap_or_default(),
                sequence: sequence.unwrap_or_default(),
                turn: payload
                    .get("turn")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default(),
                event_type: event_type.clone(),
                call_kind: None,
                operation: None,
                payload: payload.clone(),
                failure_category: None,
                stop_reason: None,
                created_at: chrono::DateTime::from_timestamp_millis(emitted_at_unix_ms)
                    .unwrap_or_else(chrono::Utc::now)
                    .to_rfc3339(),
            })
            .unwrap_or(AgenticEvent::MakoControllerEvent {
                session_id,
                run_id,
                sequence,
                emitted_at_unix_ms,
                event_type,
                payload,
            })
        }
        MakoEvent::Lagged(lagged) => AgenticEvent::Lagged {
            skipped: usize::try_from(lagged.skipped).unwrap_or(usize::MAX),
        },
        MakoEvent::ReplayGap(gap) => AgenticEvent::Error {
            error: format!(
                "Mako event replay gap: requested after {}, earliest available {}",
                gap.requested_after, gap.earliest_available
            ),
        },
        MakoEvent::DaemonShuttingDown { reason } => AgenticEvent::Error {
            error: format!(
                "Mako daemon is shutting down{}",
                reason
                    .map(|reason| format!(": {reason}"))
                    .unwrap_or_default()
            ),
        },
        MakoEvent::Extension(extension) if extension.name == "agentic_event" => {
            serde_json::from_value(extension.payload.clone()).unwrap_or(
                AgenticEvent::MakoControllerEvent {
                    session_id,
                    run_id,
                    sequence,
                    emitted_at_unix_ms,
                    event_type: "extension:agentic_event".to_string(),
                    payload: extension.payload,
                },
            )
        }
        MakoEvent::StateChanged(state) => AgenticEvent::MakoControllerEvent {
            session_id,
            run_id,
            sequence,
            emitted_at_unix_ms,
            event_type: "state_changed".to_string(),
            payload: serde_json::json!({
                "previous": state.previous,
                "current": state.current,
                "details": state.details,
            }),
        },
        MakoEvent::Extension(extension) => AgenticEvent::MakoControllerEvent {
            session_id,
            run_id,
            sequence,
            emitted_at_unix_ms,
            event_type: format!("extension:{}", extension.name),
            payload: extension.payload,
        },
    }
}

fn discover_socket_path() -> Result<PathBuf> {
    if let Some(path) = env_path("KRUSTY_MAKO_SOCKET") {
        return Ok(path);
    }
    if let Some(runtime_dir) = env_path("XDG_RUNTIME_DIR") {
        return Ok(runtime_dir.join("krusty").join("mako.sock"));
    }
    #[cfg(target_os = "macos")]
    if let Some(cache_dir) = dirs::cache_dir() {
        return Ok(cache_dir.join("krusty").join("run").join("mako.sock"));
    }
    #[cfg(unix)]
    {
        return Ok(std::env::temp_dir()
            .join(format!(
                "krusty-{}",
                krusty_mako_protocol::current_effective_uid()
            ))
            .join("mako.sock"));
    }
    #[cfg(not(unix))]
    bail!("Mako daemon IPC requires Unix-domain sockets")
}

fn discover_key_path() -> PathBuf {
    env_path("KRUSTY_MAKO_KEY").unwrap_or_else(|| {
        krusty_core::paths::config_dir()
            .join("run")
            .join("mako-ipc.key")
    })
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use krusty_mako_protocol::{
        read_frame, unix_time_millis, write_frame, AuthPolicy, ClientFrame, PongResponse,
        ProtocolErrorPayload, RecoverResponse, ResponseEnvelope, RuntimeEvent, ServerFrame,
        SubscriptionAccepted,
    };
    use krusty_mako_protocol::{IpcKey, MakoIpcClientConfig};
    #[cfg(unix)]
    use tokio::net::{UnixListener, UnixStream};

    use super::*;

    #[cfg(unix)]
    async fn accept_authenticated_request(
        listener: &UnixListener,
        key: &IpcKey,
    ) -> (UnixStream, RequestEnvelope) {
        let (mut stream, _) = listener.accept().await.expect("test daemon should accept");
        let frame: ClientFrame = read_frame(&mut stream)
            .await
            .expect("hello frame should decode")
            .expect("hello frame should exist");
        let ClientFrame::Hello(hello) = frame else {
            panic!("first client frame was not hello");
        };
        let version = key
            .verify_hello(&hello, AuthPolicy::default(), unix_time_millis())
            .expect("test hello should authenticate");
        let acknowledgement = key.hello_ack(
            version,
            "test-daemon-instance",
            "test-daemon-version",
            hello.nonce,
        );
        write_frame(&mut stream, &ServerFrame::HelloAck(acknowledgement))
            .await
            .expect("hello acknowledgement should write");

        let frame: ClientFrame = read_frame(&mut stream)
            .await
            .expect("request frame should decode")
            .expect("request frame should exist");
        let ClientFrame::Request(request) = frame else {
            panic!("second client frame was not a request");
        };
        (stream, request)
    }

    #[cfg(unix)]
    async fn respond(stream: &mut UnixStream, request: &RequestEnvelope, payload: ResponsePayload) {
        write_frame(
            stream,
            &ServerFrame::Response(ResponseEnvelope::success(
                request.request_id.clone(),
                payload,
            )),
        )
        .await
        .expect("test daemon response should write");
    }

    #[cfg(unix)]
    fn pong() -> ResponsePayload {
        ResponsePayload::Pong(PongResponse {
            instance_id: "test-daemon-instance".to_string(),
            daemon_version: "test-daemon-version".to_string(),
            uptime_secs: 1,
            server_time_unix_ms: unix_time_millis(),
        })
    }

    #[cfg(unix)]
    fn accepted() -> ResponsePayload {
        ResponsePayload::Ack(AckResponse {
            accepted: true,
            message: None,
        })
    }

    #[test]
    fn maps_every_session_control_to_a_typed_command() {
        assert!(matches!(start_command("s"), Command::StartSession(_)));
        assert!(matches!(
            dispatch_command(
                "task",
                "/work",
                Some("/project"),
                Some("model"),
                Some(42),
                Some("high"),
                Some("reviewers")
            ),
            Command::Dispatch(DispatchCommand {
                start_at_unix_ms: Some(42),
                ..
            })
        ));
        assert!(matches!(resume_command("s"), Command::ResumeSession(_)));
        assert!(matches!(pause_command("s"), Command::PauseSession(_)));
        assert!(matches!(
            schedule_command("s", 42, "test"),
            Command::ScheduleSession(_)
        ));
        assert!(matches!(cancel_command("s"), Command::CancelSession(_)));
        assert!(matches!(delete_command("s"), Command::DeleteSession(_)));
        assert!(matches!(
            message_command("s", "hello"),
            Command::SendMessage(_)
        ));
        assert!(matches!(
            priority_command("s", "high"),
            Command::SetPriority(_)
        ));
        assert!(matches!(crew_command("s", None), Command::SetCrew(_)));
        assert!(matches!(recover_command(Some("s")), Command::Recover(_)));
        assert!(matches!(
            subscribe_command("s", Some(9), Some(25)),
            Command::Subscribe(SubscribeCommand {
                after_sequence: Some(9),
                replay_limit: Some(25),
                ..
            })
        ));
        assert!(matches!(
            steer_command("s", "p", serde_json::json!([])),
            Command::Steer(_)
        ));
        assert!(matches!(
            tool_approval_command("s", "t", true),
            Command::ToolApproval(_)
        ));
        assert!(matches!(
            user_response_command("s", "t", "yes"),
            Command::UserResponse(_)
        ));
    }

    #[test]
    fn actor_preserves_the_authenticated_user_exactly() {
        assert_eq!(actor(Some("alice")).user_id.as_deref(), Some("alice"));
        assert_eq!(actor(None).user_id, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repeatable_controls_send_fresh_intent_keys_and_exact_actor() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("mako.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..12 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                let payload = match &request.command {
                    Command::Ping => pong(),
                    Command::Recover(_) => {
                        ResponsePayload::Recover(RecoverResponse { recovered_count: 1 })
                    }
                    _ => accepted(),
                };
                respond(&mut stream, &request, payload).await;
                requests.push(request);
            }
            requests
        });

        let control = MakoDaemonControl::connect_client(MakoIpcClient::new(
            MakoIpcClientConfig::new(socket_path, "control-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        control
            .pause(Some("alice"), "session-1")
            .await
            .expect("first pause should succeed");
        control
            .resume(Some("alice"), "session-1")
            .await
            .expect("resume should succeed");
        control
            .pause(Some("alice"), "session-1")
            .await
            .expect("second pause should succeed");
        control
            .set_priority(Some("alice"), "session-1", "high")
            .await
            .expect("first high priority should succeed");
        control
            .set_priority(Some("alice"), "session-1", "normal")
            .await
            .expect("normal priority should succeed");
        control
            .set_priority(Some("alice"), "session-1", "high")
            .await
            .expect("second high priority should succeed");
        control
            .cancel(Some("alice"), "session-1")
            .await
            .expect("first cancel should succeed");
        control
            .resume(Some("alice"), "session-1")
            .await
            .expect("second resume should succeed");
        control
            .cancel(Some("alice"), "session-1")
            .await
            .expect("second cancel should succeed");
        control
            .recover(Some("alice"), Some("session-1"))
            .await
            .expect("first recovery should succeed");
        control
            .recover(Some("alice"), Some("session-1"))
            .await
            .expect("second recovery should succeed");

        let requests = server.await.expect("test daemon should finish");
        let controls = &requests[1..];
        assert!(controls
            .iter()
            .all(|request| request.actor.user_id.as_deref() == Some("alice")));
        assert!(matches!(controls[0].command, Command::PauseSession(_)));
        assert!(matches!(controls[1].command, Command::ResumeSession(_)));
        assert!(matches!(controls[2].command, Command::PauseSession(_)));
        assert!(matches!(controls[3].command, Command::SetPriority(_)));
        assert!(matches!(controls[4].command, Command::SetPriority(_)));
        assert!(matches!(controls[5].command, Command::SetPriority(_)));
        assert_ne!(controls[0].idempotency_key, controls[2].idempotency_key);
        assert_ne!(controls[3].idempotency_key, controls[5].idempotency_key);
        assert!(matches!(controls[6].command, Command::CancelSession(_)));
        assert!(matches!(controls[7].command, Command::ResumeSession(_)));
        assert!(matches!(controls[8].command, Command::CancelSession(_)));
        assert_ne!(controls[6].idempotency_key, controls[8].idempotency_key);
        assert!(matches!(controls[9].command, Command::Recover(_)));
        assert!(matches!(controls[10].command, Command::Recover(_)));
        assert_ne!(controls[9].idempotency_key, controls[10].idempotency_key);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn two_subscribers_keep_independent_cursors_and_surface_lag() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("mako.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..3 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                if index == 0 {
                    respond(&mut stream, &request, pong()).await;
                } else {
                    respond(
                        &mut stream,
                        &request,
                        ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                            session_id: "session-1".to_string(),
                            high_water_sequence: Some(if index == 1 { 5 } else { 9 }),
                        }),
                    )
                    .await;
                    if index == 2 {
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(EventEnvelope {
                                version: krusty_mako_protocol::ProtocolVersion::CURRENT,
                                session_id: Some("session-1".to_string()),
                                run_id: Some("run-1".to_string()),
                                sequence: Some(8),
                                emitted_at_unix_ms: unix_time_millis(),
                                event: MakoEvent::Lagged(krusty_mako_protocol::LaggedEvent {
                                    skipped: 3,
                                    resume_after_sequence: Some(8),
                                }),
                            }),
                        )
                        .await
                        .expect("lag event should write");
                    }
                    let sequence = if index == 1 { 5 } else { 9 };
                    write_frame(
                        &mut stream,
                        &ServerFrame::Event(EventEnvelope {
                            version: krusty_mako_protocol::ProtocolVersion::CURRENT,
                            session_id: Some("session-1".to_string()),
                            run_id: Some("run-1".to_string()),
                            sequence: Some(sequence),
                            emitted_at_unix_ms: unix_time_millis(),
                            event: MakoEvent::Runtime(RuntimeEvent {
                                event_type: "run_started".to_string(),
                                payload: serde_json::json!({"run_id": "run-1"}),
                            }),
                        }),
                    )
                    .await
                    .expect("runtime event should write");
                }
                requests.push(request);
            }
            requests
        });

        let control = MakoDaemonControl::connect_client(MakoIpcClient::new(
            MakoIpcClientConfig::new(socket_path, "cursor-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let mut first = control
            .subscribe(Some("alice"), "session-1", Some(0), Some(50))
            .await
            .expect("first subscription should succeed");
        let first_event = map_daemon_event(
            first
                .next_event()
                .await
                .expect("first event should decode")
                .expect("first event should exist"),
        );
        assert!(matches!(
            first_event,
            AgenticEvent::MakoControllerEvent {
                sequence: Some(5),
                ..
            }
        ));

        let mut second = control
            .subscribe(Some("alice"), "session-1", Some(5), Some(10))
            .await
            .expect("second subscription should succeed");
        let lagged = map_daemon_event(
            second
                .next_event()
                .await
                .expect("lag event should decode")
                .expect("lag event should exist"),
        );
        assert!(matches!(lagged, AgenticEvent::Lagged { skipped: 3 }));
        let resumed = map_daemon_event(
            second
                .next_event()
                .await
                .expect("resumed event should decode")
                .expect("resumed event should exist"),
        );
        assert!(matches!(
            resumed,
            AgenticEvent::MakoControllerEvent {
                sequence: Some(9),
                ..
            }
        ));

        let requests = server.await.expect("test daemon should finish");
        let Command::Subscribe(first_command) = &requests[1].command else {
            panic!("first request was not subscribe");
        };
        let Command::Subscribe(second_command) = &requests[2].command else {
            panic!("second request was not subscribe");
        };
        assert_eq!(first_command.after_sequence, Some(0));
        assert_eq!(first_command.replay_limit, Some(50));
        assert_eq!(second_command.after_sequence, Some(5));
        assert_eq!(second_command.replay_limit, Some(10));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn manager_reauthenticates_each_subscriber_instead_of_reusing_owner_bridge() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("mako.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut alice_stream = None;
            for index in 0..3 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                if index == 0 {
                    respond(&mut stream, &request, pong()).await;
                } else if request.actor.user_id.as_deref() == Some("alice") {
                    respond(
                        &mut stream,
                        &request,
                        ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                            session_id: "session-1".to_string(),
                            high_water_sequence: None,
                        }),
                    )
                    .await;
                    alice_stream = Some(stream);
                } else {
                    write_frame(
                        &mut stream,
                        &ServerFrame::Response(ResponseEnvelope::failure(
                            request.request_id.clone(),
                            ProtocolErrorPayload::new(
                                "ownership_mismatch",
                                "session not found",
                                false,
                            ),
                        )),
                    )
                    .await
                    .expect("ownership failure should write");
                }
                requests.push(request);
            }
            drop(alice_stream);
            requests
        });

        let control = MakoDaemonControl::connect_client(MakoIpcClient::new(
            MakoIpcClientConfig::new(socket_path, "ownership-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::MakoRuntimeManager::build(Some(control));
        let _alice = manager
            .subscribe_for_user("session-1", Some("alice"))
            .await
            .expect("owner subscription should succeed");
        let bob = tokio::time::timeout(
            Duration::from_secs(2),
            manager.subscribe_for_user("session-1", Some("bob")),
        )
        .await
        .expect("Bob subscription must reach the daemon")
        .expect_err("non-owner subscription must fail");
        assert!(bob.to_string().contains("ownership_mismatch"));

        let requests = server.await.expect("test daemon should finish");
        assert_eq!(requests[1].actor.user_id.as_deref(), Some("alice"));
        assert_eq!(requests[2].actor.user_id.as_deref(), Some("bob"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn first_chat_message_sends_starts_then_replays_without_local_runtime() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("mako.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..4 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Ping => respond(&mut stream, &request, pong()).await,
                    Command::Subscribe(_) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: "session-1".to_string(),
                                high_water_sequence: Some(1),
                            }),
                        )
                        .await;
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(EventEnvelope {
                                version: krusty_mako_protocol::ProtocolVersion::CURRENT,
                                session_id: Some("session-1".to_string()),
                                run_id: Some("run-1".to_string()),
                                sequence: Some(1),
                                emitted_at_unix_ms: unix_time_millis(),
                                event: MakoEvent::Runtime(RuntimeEvent {
                                    event_type: "run_queued".to_string(),
                                    payload: serde_json::json!({"run_id": "run-1"}),
                                }),
                            }),
                        )
                        .await
                        .expect("replayed event should write");
                    }
                    _ => respond(&mut stream, &request, accepted()).await,
                }
                requests.push(request);
            }
            requests
        });

        let control = MakoDaemonControl::connect_client(MakoIpcClient::new(
            MakoIpcClientConfig::new(socket_path, "first-chat-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::MakoRuntimeManager::build(Some(control));
        let mut receiver = manager
            .begin_daemon_chat_turn_for_user("session-1", "hello", Some("alice"), true)
            .await
            .expect("first chat turn should be accepted");
        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("replayed event should arrive")
            .expect("event channel should remain readable");
        assert!(matches!(
            event,
            AgenticEvent::MakoControllerEvent {
                sequence: Some(1),
                ..
            }
        ));
        assert!(manager.runtimes.read().await.is_empty());

        let requests = server.await.expect("test daemon should finish");
        assert!(matches!(requests[1].command, Command::SendMessage(_)));
        assert!(matches!(requests[2].command, Command::StartSession(_)));
        let Command::Subscribe(subscribe) = &requests[3].command else {
            panic!("fourth request was not subscribe");
        };
        assert_eq!(subscribe.after_sequence, Some(0));
        assert_eq!(subscribe.replay_limit, Some(256));
        assert!(requests[1..]
            .iter()
            .all(|request| request.actor.user_id.as_deref() == Some("alice")));
    }

    #[tokio::test]
    async fn daemon_connect_fails_closed_when_socket_is_unavailable() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let client = MakoIpcClient::new(
            MakoIpcClientConfig::new(temp.path().join("missing.sock"), "outage-test"),
            IpcKey::generate(),
        );
        let error = MakoDaemonControl::connect_client(client)
            .await
            .expect_err("missing daemon must fail");
        assert!(error.to_string().contains("Mako daemon command failed"));
    }
}
