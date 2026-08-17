#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mitsuro_core::storage::RuntimeTraceEvent;
#[cfg(unix)]
use mitsuro_hive_protocol::HiveIpcClientConfig;
use mitsuro_hive_protocol::{
    AckResponse, Actor, ClientError, Command, CreateScheduleCommand, DaemonStats, DispatchCommand,
    EventEnvelope, EventSubscription, GroupMessageCommand, GroupStopCommand, GroupTurnResponse,
    HiveEvent, HiveIpcClient, MessageCommand, ModelKey, RecoverCommand, ReplaceScheduleCommand,
    RequestEnvelope, ResponsePayload, ScheduleCommand, ScheduleDefinition, ScheduleResponse,
    SessionCommand, SetCrewCommand, SetPriorityCommand, SetScheduleStatusCommand, SteerCommand,
    SubscribeCommand, ToolApprovalCommand, UserResponseCommand, MODEL_IDENTITY_PROTOCOL_MINOR,
    PROTOCOL_MAJOR,
};

use super::HiveRuntimeStats;
use crate::types::AgenticEvent;

const CLIENT_ID: &str = "mitsuro-server";
#[cfg(unix)]
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(super) enum HiveDaemonError {
    Remote { code: String, message: String },
    Unavailable(String),
}

impl std::fmt::Display for HiveDaemonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remote { code, message } => write!(formatter, "{code}: {message}"),
            Self::Unavailable(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for HiveDaemonError {}

impl From<ClientError> for HiveDaemonError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Remote { code, message } => Self::Remote { code, message },
            error => Self::Unavailable(error.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct HiveDaemonControl {
    client: HiveIpcClient,
}

impl HiveDaemonControl {
    #[cfg(unix)]
    pub(super) async fn connect_discovered() -> Result<Self> {
        Self::connect_paths(discover_socket_path(), discover_key_path()).await
    }

    #[cfg(unix)]
    async fn connect_paths(socket_path: PathBuf, key_path: PathBuf) -> Result<Self> {
        let mut config = HiveIpcClientConfig::new(socket_path.clone(), CLIENT_ID);
        config.request_timeout = DEFAULT_REQUEST_TIMEOUT;
        let client =
            HiveIpcClient::from_key_path_or_create(config, &key_path).with_context(|| {
                format!(
                    "loading or initializing Hive daemon IPC key at {}",
                    key_path.display()
                )
            })?;
        let control = Self { client };
        control
            .healthcheck()
            .await
            .with_context(|| format!("Hive daemon unavailable at {}", socket_path.display()))?;
        Ok(control)
    }

    #[cfg(not(unix))]
    pub(super) async fn connect_discovered() -> Result<Self> {
        bail!("Hive daemon IPC is unavailable: this platform has no Unix-domain socket support")
    }

    #[cfg(test)]
    pub(super) async fn connect_client(client: HiveIpcClient) -> Result<Self> {
        let control = Self { client };
        control.healthcheck().await?;
        Ok(control)
    }

    async fn healthcheck(&self) -> Result<()> {
        match self
            .command(None, Command::Stats, Some(unique_key("healthcheck")))
            .await?
        {
            ResponsePayload::Stats(stats) => {
                if stats.protocol.major != PROTOCOL_MAJOR
                    || stats.protocol.minor < MODEL_IDENTITY_PROTOCOL_MINOR
                {
                    bail!(
                        "Hive daemon protocol {}.{} cannot preserve exact model identity; upgrade and restart the daemon (requires {}.{})",
                        stats.protocol.major,
                        stats.protocol.minor,
                        PROTOCOL_MAJOR,
                        MODEL_IDENTITY_PROTOCOL_MINOR
                    )
                }
                let pump_alive = stats.runtime.pump_alive;
                let scheduler_ready = stats.runtime.scheduler_ready;
                if pump_alive && scheduler_ready {
                    Ok(())
                } else {
                    bail!(
                        "Hive scheduler is not ready (pump_alive={pump_alive}, scheduler_ready={scheduler_ready})"
                    )
                }
            }
            payload => bail!("Hive healthcheck returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn start(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        wake_reason: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            start_command(session_id),
            request_key(
                idempotency_key,
                unique_key(&format!("start:{session_id}:{wake_reason}")),
            ),
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
        model_key: Option<&ModelKey>,
        model_catalog_revision: Option<&str>,
        start_at_unix_ms: Option<i64>,
        priority: Option<&str>,
        crew_slug: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<(String, String)> {
        let command = dispatch_command(
            task,
            working_dir,
            project_dir,
            model,
            model_key,
            model_catalog_revision,
            start_at_unix_ms,
            priority,
            crew_slug,
        );
        match self
            .command(
                user_id,
                command,
                Some(request_key(idempotency_key, unique_key("dispatch"))),
            )
            .await?
        {
            ResponsePayload::Dispatch(response) => Ok((response.session_id, response.status)),
            payload => bail!("Hive dispatch returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn create_schedule(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        definition: ScheduleDefinition,
        idempotency_key: Option<&str>,
    ) -> Result<ScheduleResponse> {
        self.expect_schedule(
            user_id,
            Command::CreateSchedule(CreateScheduleCommand {
                session_id: session_id.to_string(),
                definition,
            }),
            request_key(idempotency_key, unique_key("create-schedule")),
            "create schedule",
        )
        .await
    }

    pub(super) async fn replace_schedule(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        schedule_id: &str,
        expected_revision: u64,
        definition: ScheduleDefinition,
        idempotency_key: Option<&str>,
    ) -> Result<ScheduleResponse> {
        self.expect_schedule(
            user_id,
            Command::ReplaceSchedule(ReplaceScheduleCommand {
                session_id: session_id.to_string(),
                schedule_id: schedule_id.to_string(),
                expected_revision,
                definition,
            }),
            request_key(idempotency_key, unique_key("replace-schedule")),
            "replace schedule",
        )
        .await
    }

    pub(super) async fn set_schedule_status(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        schedule_id: &str,
        expected_revision: u64,
        status: &str,
        idempotency_key: Option<&str>,
    ) -> Result<ScheduleResponse> {
        self.expect_schedule(
            user_id,
            Command::SetScheduleStatus(SetScheduleStatusCommand {
                session_id: session_id.to_string(),
                schedule_id: schedule_id.to_string(),
                expected_revision,
                status: status.to_string(),
            }),
            request_key(idempotency_key, unique_key("schedule-status")),
            "set schedule status",
        )
        .await
    }

    pub(super) async fn resume(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            resume_command(session_id),
            request_key(idempotency_key, unique_key(&format!("resume:{session_id}"))),
            "resume session",
        )
        .await
    }

    pub(super) async fn pause(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            pause_command(session_id),
            request_key(idempotency_key, unique_key(&format!("pause:{session_id}"))),
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
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            schedule_command(session_id, wake_at_unix_ms, reason),
            request_key(
                idempotency_key,
                unique_key(&format!("schedule:{session_id}:{wake_at_unix_ms}")),
            ),
            "schedule session",
        )
        .await
    }

    pub(super) async fn cancel(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            cancel_command(session_id),
            request_key(idempotency_key, unique_key(&format!("cancel:{session_id}"))),
            "cancel session",
        )
        .await
    }

    pub(super) async fn delete(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            delete_command(session_id),
            request_key(idempotency_key, stable_key("delete", session_id)),
            "delete session",
        )
        .await
    }

    pub(super) async fn send_message(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        message: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            message_command(session_id, message),
            request_key(
                idempotency_key,
                unique_key(&format!("message:{session_id}")),
            ),
            "send message",
        )
        .await
    }

    /// Fan one room message out as a durable group turn. The daemon appends
    /// the message, resolves targets, and queues member runs atomically.
    pub(super) async fn group_message(
        &self,
        user_id: Option<&str>,
        group_id: &str,
        message: &str,
        mentions_override: Option<Vec<String>>,
        idempotency_key: Option<&str>,
    ) -> Result<GroupTurnResponse> {
        match self
            .command(
                user_id,
                Command::GroupMessage(GroupMessageCommand {
                    group_id: group_id.to_string(),
                    message: message.to_string(),
                    mentions_override,
                }),
                Some(request_key(
                    idempotency_key,
                    unique_key(&format!("group-message:{group_id}")),
                )),
            )
            .await?
        {
            ResponsePayload::GroupTurn(turn) => Ok(turn),
            payload => bail!("Hive group message returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn group_stop(
        &self,
        user_id: Option<&str>,
        group_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            Command::GroupStop(GroupStopCommand {
                group_id: group_id.to_string(),
            }),
            request_key(
                idempotency_key,
                unique_key(&format!("group-stop:{group_id}")),
            ),
            "stop the group turn",
        )
        .await
    }

    pub(super) async fn set_priority(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        priority: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            priority_command(session_id, priority),
            request_key(
                idempotency_key,
                unique_key(&format!("priority:{session_id}:{priority}")),
            ),
            "set priority",
        )
        .await
    }

    pub(super) async fn set_crew(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        crew_slug: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            crew_command(session_id, crew_slug),
            request_key(
                idempotency_key,
                unique_key(&format!(
                    "crew:{session_id}:{}",
                    crew_slug.unwrap_or("none")
                )),
            ),
            "set crew",
        )
        .await
    }

    pub(super) async fn recover(
        &self,
        user_id: Option<&str>,
        session_id: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<usize> {
        let key = request_key(
            idempotency_key,
            unique_key(&format!("recover:{}", session_id.unwrap_or("all"))),
        );
        match self
            .command(user_id, recover_command(session_id), Some(key))
            .await?
        {
            ResponsePayload::Recover(response) => Ok(response.recovered_count),
            payload => bail!("Hive recover returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn stats(&self, user_id: Option<&str>) -> Result<HiveRuntimeStats> {
        match self
            .command(user_id, Command::Stats, Some(unique_key("stats")))
            .await?
        {
            ResponsePayload::Stats(stats) => Ok(map_stats(stats)),
            payload => bail!("Hive stats returned unexpected response {payload:?}"),
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
        let subscription = self
            .client
            .subscribe(request)
            .await
            .map_err(HiveDaemonError::from)
            .context("subscribing to Hive daemon events")?;
        if subscription.accepted.session_id != session_id {
            bail!("Hive daemon accepted an event subscription for an unexpected session");
        }
        Ok(subscription)
    }

    pub(super) async fn steer(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        pending_id: &str,
        content: serde_json::Value,
        idempotency_key: Option<&str>,
    ) -> Result<AckResponse> {
        match self
            .command(
                user_id,
                steer_command(session_id, pending_id, content),
                Some(request_key(
                    idempotency_key,
                    stable_key("steer", pending_id),
                )),
            )
            .await?
        {
            ResponsePayload::Ack(response) => Ok(response),
            payload => bail!("Hive steer returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn tool_approval(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        run_id: &str,
        tool_call_id: &str,
        approved: bool,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            tool_approval_command(session_id, run_id, tool_call_id, approved),
            request_key(
                idempotency_key,
                stable_key("approval", &format!("{session_id}:{run_id}:{tool_call_id}")),
            ),
            "submit tool approval",
        )
        .await
    }

    pub(super) async fn user_response(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        run_id: &str,
        tool_call_id: &str,
        response: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            user_response_command(session_id, run_id, tool_call_id, response),
            request_key(
                idempotency_key,
                stable_key("response", &format!("{session_id}:{run_id}:{tool_call_id}")),
            ),
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
            ResponsePayload::Ack(ack) => Err(HiveDaemonError::Remote {
                code: "conflict".to_string(),
                message: format!(
                    "Hive daemon declined to {operation}: {}",
                    ack.message
                        .unwrap_or_else(|| "no reason provided".to_string())
                ),
            }
            .into()),
            payload => bail!("Hive {operation} returned unexpected response {payload:?}"),
        }
    }

    async fn expect_schedule(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: String,
        operation: &str,
    ) -> Result<ScheduleResponse> {
        match self
            .command(user_id, command, Some(idempotency_key))
            .await?
        {
            ResponsePayload::Schedule(schedule) => Ok(schedule),
            payload => bail!("Hive {operation} returned unexpected response {payload:?}"),
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
            .map_err(HiveDaemonError::from)
            .context("Hive daemon command failed")
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
    model_key: Option<&ModelKey>,
    model_catalog_revision: Option<&str>,
    start_at_unix_ms: Option<i64>,
    priority: Option<&str>,
    crew_slug: Option<&str>,
) -> Command {
    Command::Dispatch(DispatchCommand {
        task: task.to_string(),
        working_dir: working_dir.to_string(),
        project_dir: project_dir.map(ToOwned::to_owned),
        model: model.map(ToOwned::to_owned),
        model_key: model_key.cloned(),
        model_catalog_revision: model_catalog_revision.map(ToOwned::to_owned),
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

fn tool_approval_command(
    session_id: &str,
    run_id: &str,
    tool_call_id: &str,
    approved: bool,
) -> Command {
    Command::ToolApproval(ToolApprovalCommand {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        approved,
    })
}

fn user_response_command(
    session_id: &str,
    run_id: &str,
    tool_call_id: &str,
    response: &str,
) -> Command {
    Command::UserResponse(UserResponseCommand {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
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

fn request_key(provided: Option<&str>, fallback: String) -> String {
    provided.map(ToOwned::to_owned).unwrap_or(fallback)
}

fn map_stats(stats: DaemonStats) -> HiveRuntimeStats {
    let runtime = stats.runtime;
    HiveRuntimeStats {
        active_controller_count: runtime.active_controllers,
        active_run_count: runtime.active_runs,
        queued_run_count: runtime.queued_runs,
        recovery_required_run_count: runtime.recovery_required,
        active_runtime_count: runtime.active_runs,
        scheduled_wake_count: runtime.queued_runs,
        // The daemon cannot observe HTTP/TUI subscribers owned by this server.
        event_stream_count: 0,
        uptime_secs: stats.uptime_secs,
    }
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
        HiveEvent::Runtime(runtime) => {
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
            .unwrap_or(AgenticEvent::HiveControllerEvent {
                session_id,
                run_id,
                sequence,
                emitted_at_unix_ms,
                event_type,
                payload,
            })
        }
        HiveEvent::Lagged(lagged) => AgenticEvent::Lagged {
            skipped: usize::try_from(lagged.skipped).unwrap_or(usize::MAX),
        },
        HiveEvent::ReplayGap(gap) => AgenticEvent::Error {
            error: format!(
                "Hive event replay gap: requested after {}, earliest available {}",
                gap.requested_after, gap.earliest_available
            ),
        },
        HiveEvent::DaemonShuttingDown { reason } => AgenticEvent::Error {
            error: format!(
                "Hive daemon is shutting down{}",
                reason
                    .map(|reason| format!(": {reason}"))
                    .unwrap_or_default()
            ),
        },
        HiveEvent::Extension(extension) if extension.name == "agentic_event" => {
            serde_json::from_value(extension.payload.clone()).unwrap_or(
                AgenticEvent::HiveControllerEvent {
                    session_id,
                    run_id,
                    sequence,
                    emitted_at_unix_ms,
                    event_type: "extension:agentic_event".to_string(),
                    payload: extension.payload,
                },
            )
        }
        HiveEvent::StateChanged(state) => AgenticEvent::HiveControllerEvent {
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
        HiveEvent::Extension(extension) => AgenticEvent::HiveControllerEvent {
            session_id,
            run_id,
            sequence,
            emitted_at_unix_ms,
            event_type: format!("extension:{}", extension.name),
            payload: extension.payload,
        },
    }
}

#[cfg(unix)]
fn discover_socket_path() -> PathBuf {
    if let Some(path) = env_path("MITSURO_HIVE_SOCKET") {
        return path;
    }
    if let Some(runtime_dir) = env_path("XDG_RUNTIME_DIR") {
        return runtime_dir.join("mitsuro").join("hive.sock");
    }
    #[cfg(target_os = "macos")]
    if let Some(cache_dir) = dirs::cache_dir() {
        return cache_dir.join("mitsuro").join("run").join("hive.sock");
    }
    std::env::temp_dir()
        .join(format!(
            "mitsuro-{}",
            mitsuro_hive_protocol::current_effective_uid()
        ))
        .join("hive.sock")
}

#[cfg(unix)]
fn discover_key_path() -> PathBuf {
    env_path("MITSURO_HIVE_KEY").unwrap_or_else(|| {
        mitsuro_core::paths::config_dir()
            .join("run")
            .join("hive-ipc.key")
    })
}

#[cfg(unix)]
fn env_path(name: &str) -> Option<PathBuf> {
    mitsuro_core::identity::env_var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use mitsuro_hive_protocol::{
        read_frame, unix_time_millis, write_frame, AuthPolicy, ClientFrame, DaemonRuntimeStats,
        ProtocolErrorPayload, ProtocolVersion, RecoverResponse, ResponseEnvelope, RuntimeEvent,
        ServerFrame, SubscriptionAccepted,
    };
    use mitsuro_hive_protocol::{HiveIpcClientConfig, IpcKey};
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
        (stream, *request)
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
    async fn send_runtime_event(stream: &mut UnixStream, session_id: &str, sequence: i64) {
        write_frame(
            stream,
            &ServerFrame::Event(EventEnvelope {
                version: ProtocolVersion::CURRENT,
                session_id: Some(session_id.to_string()),
                run_id: Some("run-1".to_string()),
                sequence: Some(sequence),
                emitted_at_unix_ms: unix_time_millis(),
                event: HiveEvent::Runtime(RuntimeEvent {
                    event_type: "run_started".to_string(),
                    payload: serde_json::json!({"run_id": "run-1"}),
                }),
            }),
        )
        .await
        .expect("runtime event should write");
    }

    #[cfg(unix)]
    fn scheduler_stats(pump_alive: bool, scheduler_ready: bool) -> ResponsePayload {
        scheduler_stats_at_protocol(pump_alive, scheduler_ready, ProtocolVersion::CURRENT)
    }

    #[cfg(unix)]
    fn scheduler_stats_at_protocol(
        pump_alive: bool,
        scheduler_ready: bool,
        protocol: ProtocolVersion,
    ) -> ResponsePayload {
        ResponsePayload::Stats(DaemonStats {
            instance_id: "test-daemon-instance".to_string(),
            daemon_version: "test-daemon-version".to_string(),
            protocol,
            uptime_secs: 1,
            active_connections: 1,
            handled_requests: 1,
            runtime: DaemonRuntimeStats {
                pump_alive,
                scheduler_ready,
                ..DaemonRuntimeStats::default()
            },
        })
    }

    #[cfg(unix)]
    fn ready_stats() -> ResponsePayload {
        scheduler_stats(true, true)
    }

    #[cfg(unix)]
    #[test]
    fn daemon_stats_mapping_preserves_authoritative_scheduler_counts() {
        let mapped = map_stats(DaemonStats {
            instance_id: "daemon".to_string(),
            daemon_version: "test".to_string(),
            protocol: ProtocolVersion::CURRENT,
            uptime_secs: 99,
            active_connections: 17,
            handled_requests: 23,
            runtime: DaemonRuntimeStats {
                active_controllers: 11,
                active_runs: 7,
                queued_runs: 5,
                recovery_required: 3,
                pump_alive: true,
                scheduler_ready: true,
            },
        });

        assert_eq!(mapped.active_controller_count, 11);
        assert_eq!(mapped.active_run_count, 7);
        assert_eq!(mapped.queued_run_count, 5);
        assert_eq!(mapped.recovery_required_run_count, 3);
        assert_eq!(mapped.active_runtime_count, 7);
        assert_eq!(mapped.scheduled_wake_count, 5);
        assert_eq!(mapped.event_stream_count, 0);
        assert_eq!(mapped.uptime_secs, 99);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_control_key_bootstraps_before_first_socket_activation_connection() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let key_directory = temp.path().join("config").join("run");
        let key_path = key_directory.join("hive-ipc.key");
        assert!(!key_path.exists());
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let server_key_path = key_path.clone();
        let server = tokio::spawn(async move {
            // Accepting the first connection models systemd socket activation:
            // the daemon starts only after the trusted client has connected.
            let (mut stream, _) = listener.accept().await.expect("test daemon should accept");
            let key = IpcKey::load(&server_key_path)
                .expect("client must initialize authority before connecting");
            let frame: ClientFrame = read_frame(&mut stream)
                .await
                .expect("hello frame should decode")
                .expect("hello frame should exist");
            let ClientFrame::Hello(hello) = frame else {
                panic!("first client frame was not hello");
            };
            let version = key
                .verify_hello(&hello, AuthPolicy::default(), unix_time_millis())
                .expect("bootstrapped key should authenticate the activating client");
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
            assert!(matches!(request.command, Command::Stats));
            respond(&mut stream, &request, ready_stats()).await;
        });

        let control = HiveDaemonControl::connect_paths(socket_path, key_path.clone())
            .await
            .expect("fresh trusted control client should bootstrap and authenticate");
        assert_eq!(
            std::fs::metadata(&key_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        IpcKey::load(&key_path).expect("persisted key should remain securely loadable");
        drop(control);
        server.await.expect("test daemon should finish");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn healthcheck_requires_scheduler_readiness_not_transport_only() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let (mut stream, request) = accept_authenticated_request(&listener, &server_key).await;
            assert!(matches!(request.command, Command::Stats));
            respond(&mut stream, &request, scheduler_stats(true, false)).await;
        });

        let error = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "health-test"),
            key,
        ))
        .await
        .expect_err("a live transport without the scheduler lease is not healthy");
        assert!(error.to_string().contains("scheduler_ready=false"));
        server.await.expect("test daemon should finish");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn healthcheck_rejects_daemon_that_cannot_preserve_exact_model_identity() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let (mut stream, request) = accept_authenticated_request(&listener, &server_key).await;
            assert!(matches!(request.command, Command::Stats));
            respond(
                &mut stream,
                &request,
                scheduler_stats_at_protocol(
                    true,
                    true,
                    ProtocolVersion {
                        major: PROTOCOL_MAJOR,
                        minor: MODEL_IDENTITY_PROTOCOL_MINOR - 1,
                    },
                ),
            )
            .await;
        });

        let error = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "protocol-health-test"),
            key,
        ))
        .await
        .expect_err("an old daemon must not silently discard exact model identity");
        assert!(error
            .to_string()
            .contains("cannot preserve exact model identity"));
        server.await.expect("test daemon should finish");
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
                None,
                None,
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
            tool_approval_command("s", "r", "t", true),
            Command::ToolApproval(_)
        ));
        assert!(matches!(
            user_response_command("s", "r", "t", "yes"),
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
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..12 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                let payload = match &request.command {
                    Command::Stats => ready_stats(),
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

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "control-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        control
            .pause(Some("alice"), "session-1", None)
            .await
            .expect("first pause should succeed");
        control
            .resume(Some("alice"), "session-1", None)
            .await
            .expect("resume should succeed");
        control
            .pause(Some("alice"), "session-1", None)
            .await
            .expect("second pause should succeed");
        control
            .set_priority(Some("alice"), "session-1", "high", None)
            .await
            .expect("first high priority should succeed");
        control
            .set_priority(Some("alice"), "session-1", "normal", None)
            .await
            .expect("normal priority should succeed");
        control
            .set_priority(Some("alice"), "session-1", "high", None)
            .await
            .expect("second high priority should succeed");
        control
            .cancel(Some("alice"), "session-1", None)
            .await
            .expect("first cancel should succeed");
        control
            .resume(Some("alice"), "session-1", None)
            .await
            .expect("second resume should succeed");
        control
            .cancel(Some("alice"), "session-1", None)
            .await
            .expect("second cancel should succeed");
        control
            .recover(Some("alice"), Some("session-1"), None)
            .await
            .expect("first recovery should succeed");
        control
            .recover(Some("alice"), Some("session-1"), None)
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
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..3 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                if index == 0 {
                    respond(&mut stream, &request, ready_stats()).await;
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
                                version: mitsuro_hive_protocol::ProtocolVersion::CURRENT,
                                session_id: Some("session-1".to_string()),
                                run_id: Some("run-1".to_string()),
                                sequence: Some(8),
                                emitted_at_unix_ms: unix_time_millis(),
                                event: HiveEvent::Lagged(mitsuro_hive_protocol::LaggedEvent {
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
                            version: mitsuro_hive_protocol::ProtocolVersion::CURRENT,
                            session_id: Some("session-1".to_string()),
                            run_id: Some("run-1".to_string()),
                            sequence: Some(sequence),
                            emitted_at_unix_ms: unix_time_millis(),
                            event: HiveEvent::Runtime(RuntimeEvent {
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

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "cursor-test"),
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
            AgenticEvent::HiveControllerEvent {
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
            AgenticEvent::HiveControllerEvent {
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
    async fn manager_reconnects_quiet_eof_from_last_delivered_cursor_without_duplicates() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();

            let (mut health_stream, health_request) =
                accept_authenticated_request(&listener, &server_key).await;
            respond(&mut health_stream, &health_request, ready_stats()).await;
            requests.push(health_request);
            drop(health_stream);

            let (mut first_stream, first_request) =
                accept_authenticated_request(&listener, &server_key).await;
            respond(
                &mut first_stream,
                &first_request,
                ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                    session_id: "session-1".to_string(),
                    high_water_sequence: Some(5),
                }),
            )
            .await;
            send_runtime_event(&mut first_stream, "session-1", 5).await;
            requests.push(first_request);
            // A clean socket close is still an unexpected outage for a live
            // subscription and must trigger cursor-aware recovery.
            drop(first_stream);

            let (mut second_stream, second_request) =
                accept_authenticated_request(&listener, &server_key).await;
            respond(
                &mut second_stream,
                &second_request,
                ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                    session_id: "session-1".to_string(),
                    high_water_sequence: Some(6),
                }),
            )
            .await;
            // Repeat the boundary event to prove the server bridge filters a
            // replay duplicate before forwarding the new durable event.
            send_runtime_event(&mut second_stream, "session-1", 5).await;
            send_runtime_event(&mut second_stream, "session-1", 6).await;
            requests.push(second_request);
            let _ = release_rx.await;
            drop(second_stream);
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "reconnect-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .subscribe_for_user_from("session-1", Some("alice"), Some(0), Some(256))
            .await
            .expect("initial subscription should succeed");

        let first = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("initial event should arrive")
            .expect("initial event channel should remain open");
        assert!(matches!(
            first,
            AgenticEvent::HiveControllerEvent {
                sequence: Some(5),
                ..
            }
        ));
        let resumed = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("resumed event should arrive")
            .expect("resumed event channel should remain open");
        assert!(matches!(
            resumed,
            AgenticEvent::HiveControllerEvent {
                sequence: Some(6),
                ..
            }
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), receiver.recv())
                .await
                .is_err()
        );

        drop(receiver);
        let _ = release_tx.send(());
        let requests = server.await.expect("test daemon should finish");
        assert_eq!(requests.len(), 3);
        for request in &requests[1..] {
            assert_eq!(request.actor.user_id.as_deref(), Some("alice"));
            let Command::Subscribe(command) = &request.command else {
                panic!("request was not subscribe");
            };
            assert_eq!(command.session_id, "session-1");
        }
        let Command::Subscribe(initial) = &requests[1].command else {
            unreachable!();
        };
        let Command::Subscribe(reconnected) = &requests[2].command else {
            unreachable!();
        };
        assert_eq!(initial.after_sequence, Some(0));
        assert_eq!(reconnected.after_sequence, Some(5));
        assert_eq!(reconnected.replay_limit, Some(256));
        drop(manager);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn manager_surfaces_daemon_unavailable_after_reconnect_budget_is_exhausted() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let (mut health_stream, health_request) =
                accept_authenticated_request(&listener, &server_key).await;
            respond(&mut health_stream, &health_request, ready_stats()).await;
            drop(health_stream);

            let (mut stream, request) = accept_authenticated_request(&listener, &server_key).await;
            respond(
                &mut stream,
                &request,
                ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                    session_id: "session-1".to_string(),
                    high_water_sequence: Some(11),
                }),
            )
            .await;
            assert_eq!(request.actor.user_id.as_deref(), Some("alice"));
            drop(stream);
            // Dropping the only listener makes every reconnect a transport
            // failure rather than another accepted-but-empty stream.
            drop(listener);
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "outage-reconnect-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .subscribe_for_user("session-1", Some("alice"))
            .await
            .expect("initial subscription should succeed");
        server.await.expect("test daemon should finish");

        let terminal = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("terminal outage event should arrive")
            .expect("terminal outage event should be readable");
        let AgenticEvent::Error { error } = terminal else {
            panic!("expected explicit terminal error, got {terminal:?}");
        };
        assert!(error.contains("Hive event stream is unavailable"));
        assert!(matches!(
            receiver.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
        drop(manager);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn manager_closes_subscription_when_client_receiver_is_dropped() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let (mut health_stream, health_request) =
                accept_authenticated_request(&listener, &server_key).await;
            respond(&mut health_stream, &health_request, ready_stats()).await;
            drop(health_stream);

            let (mut stream, request) = accept_authenticated_request(&listener, &server_key).await;
            respond(
                &mut stream,
                &request,
                ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                    session_id: "session-1".to_string(),
                    high_water_sequence: None,
                }),
            )
            .await;
            let closed = tokio::time::timeout(
                Duration::from_secs(1),
                read_frame::<_, ClientFrame>(&mut stream),
            )
            .await
            .expect("client drop should close the IPC stream");
            assert!(matches!(closed, Ok(None) | Err(_)));
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "client-cancel-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let receiver = manager
            .subscribe_for_user("session-1", Some("alice"))
            .await
            .expect("subscription should succeed");
        drop(receiver);
        server.await.expect("test daemon should observe EOF");
        drop(manager);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_manager_cancels_subscription_even_while_client_receiver_remains() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let (mut health_stream, health_request) =
                accept_authenticated_request(&listener, &server_key).await;
            respond(&mut health_stream, &health_request, ready_stats()).await;
            drop(health_stream);

            let (mut stream, request) = accept_authenticated_request(&listener, &server_key).await;
            respond(
                &mut stream,
                &request,
                ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                    session_id: "session-1".to_string(),
                    high_water_sequence: None,
                }),
            )
            .await;
            let closed = tokio::time::timeout(
                Duration::from_secs(1),
                read_frame::<_, ClientFrame>(&mut stream),
            )
            .await
            .expect("manager drop should close the IPC stream");
            assert!(matches!(closed, Ok(None) | Err(_)));
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "manager-cancel-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .subscribe_for_user("session-1", Some("alice"))
            .await
            .expect("subscription should succeed");
        drop(manager);
        server.await.expect("test daemon should observe EOF");
        assert!(matches!(
            receiver.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn manager_reauthenticates_each_subscriber_instead_of_reusing_owner_bridge() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
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
                    respond(&mut stream, &request, ready_stats()).await;
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

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "ownership-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
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
        assert!(
            format!("{bob:#}").contains("ownership_mismatch"),
            "unexpected non-owner subscription error: {bob:#}"
        );

        let requests = server.await.expect("test daemon should finish");
        assert_eq!(requests[1].actor.user_id.as_deref(), Some("alice"));
        assert_eq!(requests[2].actor.user_id.as_deref(), Some("bob"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn first_chat_message_sends_starts_then_replays_without_local_runtime() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..4 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Stats => respond(&mut stream, &request, ready_stats()).await,
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
                                version: mitsuro_hive_protocol::ProtocolVersion::CURRENT,
                                session_id: Some("session-1".to_string()),
                                run_id: Some("run-1".to_string()),
                                sequence: Some(1),
                                emitted_at_unix_ms: unix_time_millis(),
                                event: HiveEvent::Runtime(RuntimeEvent {
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

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "first-chat-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .begin_daemon_chat_turn_for_user("session-1", "hello", Some("alice"), true, None)
            .await
            .expect("first chat turn should be accepted");
        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("replayed event should arrive")
            .expect("event channel should remain readable");
        assert!(matches!(
            event,
            AgenticEvent::HiveControllerEvent {
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
        let client = HiveIpcClient::new(
            HiveIpcClientConfig::new(temp.path().join("missing.sock"), "outage-test"),
            IpcKey::generate(),
        );
        let error = HiveDaemonControl::connect_client(client)
            .await
            .expect_err("missing daemon must fail");
        assert!(error.to_string().contains("Hive daemon command failed"));
    }
}
