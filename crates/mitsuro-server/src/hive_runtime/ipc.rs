use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mitsuro_core::storage::RuntimeTraceEvent;
#[cfg(unix)]
use mitsuro_hive_protocol::HiveIpcClientConfig;
use mitsuro_hive_protocol::{
    AckResponse, ActivateOrResumeWorkerWorkflowCommand, Actor, ClientError, Command,
    ConfirmWorkerIntroductionCommand, CreateScheduleCommand, CreateWorkerIntroductionCommand,
    DaemonStats, DispatchCommand, EventEnvelope, EventSubscription,
    GrantWorkerGovernorRecoveryCommand, GroupArchiveCommand, GroupMessageCommand, GroupStopCommand,
    GroupTurnResponse, HiveEvent, HiveIpcClient, MessageCommand, ModelKey, RecoverCommand,
    ReplaceScheduleCommand, RequestEnvelope, ResolveWorkerGoalAcceptanceCommand, ResponsePayload,
    ReturnWorkerIntroductionToContextCommand, ScheduleCommand, ScheduleDefinition,
    ScheduleResponse, SessionCommand, SetCrewCommand, SetPriorityCommand, SetScheduleStatusCommand,
    SetWorkerWorkspaceCommand, SteerCommand, SubscribeCommand, ToolApprovalCommand,
    UpdateWorkerCommand, UserResponseCommand, WorkerConversationInputDisposition,
    WorkerConversationInputResponse, WorkerGoalAcceptanceResponse, WorkerGovernorRecoveryResponse,
    WorkerIntroductionActionResponse, WorkerIntroductionCommand, WorkerIntroductionResponse,
    WorkerMutationResponse, WorkerTargetStatus, WorkerWorkflowLifecycleCommand,
    WorkerWorkflowResponse, WorkerWorkspaceResponse, MODEL_IDENTITY_PROTOCOL_MINOR, PROTOCOL_MAJOR,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DaemonInputAcceptance {
    Ack(AckResponse),
    Worker(WorkerConversationInputResponse),
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

    #[cfg(unix)]
    pub(super) async fn connect_explicit(socket_path: PathBuf, key_path: PathBuf) -> Result<Self> {
        let mut config = HiveIpcClientConfig::new(socket_path.clone(), CLIENT_ID);
        config.request_timeout = DEFAULT_REQUEST_TIMEOUT;
        let client = HiveIpcClient::from_key_path(config, &key_path).with_context(|| {
            format!(
                "loading explicit Hive daemon IPC key at {}",
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

    #[cfg(not(unix))]
    pub(super) async fn connect_explicit(
        _socket_path: PathBuf,
        _key_path: PathBuf,
    ) -> Result<Self> {
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
        self.expect_session_control(
            user_id,
            start_command(session_id),
            request_key(
                idempotency_key,
                unique_key(&format!("start:{session_id}:{wake_reason}")),
            ),
            session_id,
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

    /// Commit the Worker identity, private conversation, and its one-time
    /// Introduction run as one daemon-owned idempotent mutation.
    pub(super) async fn create_worker_introduction(
        &self,
        user_id: Option<&str>,
        command: CreateWorkerIntroductionCommand,
        idempotency_key: &str,
    ) -> Result<WorkerIntroductionResponse> {
        match self
            .command(
                user_id,
                Command::CreateWorkerIntroduction(command),
                Some(idempotency_key.to_string()),
            )
            .await?
        {
            ResponsePayload::WorkerIntroduction(response) => Ok(response),
            payload => bail!("Hive Worker Introduction returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn retry_worker_introduction(
        &self,
        user_id: Option<&str>,
        worker_id: &str,
        idempotency_key: &str,
    ) -> Result<WorkerIntroductionActionResponse> {
        self.worker_introduction_action(
            user_id,
            Command::RetryWorkerIntroduction(WorkerIntroductionCommand {
                worker_id: worker_id.to_string(),
            }),
            idempotency_key,
            "retry",
        )
        .await
    }

    pub(super) async fn skip_worker_introduction(
        &self,
        user_id: Option<&str>,
        worker_id: &str,
        idempotency_key: &str,
    ) -> Result<WorkerIntroductionActionResponse> {
        self.worker_introduction_action(
            user_id,
            Command::SkipWorkerIntroduction(WorkerIntroductionCommand {
                worker_id: worker_id.to_string(),
            }),
            idempotency_key,
            "skip",
        )
        .await
    }

    pub(super) async fn confirm_worker_introduction(
        &self,
        user_id: Option<&str>,
        command: ConfirmWorkerIntroductionCommand,
        idempotency_key: &str,
    ) -> Result<WorkerIntroductionActionResponse> {
        self.worker_introduction_action(
            user_id,
            Command::ConfirmWorkerIntroduction(command),
            idempotency_key,
            "confirm",
        )
        .await
    }

    pub(super) async fn return_worker_introduction_to_context(
        &self,
        user_id: Option<&str>,
        command: ReturnWorkerIntroductionToContextCommand,
        idempotency_key: &str,
    ) -> Result<WorkerIntroductionActionResponse> {
        self.worker_introduction_action(
            user_id,
            Command::ReturnWorkerIntroductionToContext(command),
            idempotency_key,
            "return to context",
        )
        .await
    }

    async fn worker_introduction_action(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: &str,
        action: &str,
    ) -> Result<WorkerIntroductionActionResponse> {
        match self
            .command(user_id, command, Some(idempotency_key.to_string()))
            .await?
        {
            ResponsePayload::WorkerIntroductionAction(response) => Ok(response),
            payload => {
                bail!("Hive Worker Introduction {action} returned unexpected response {payload:?}")
            }
        }
    }

    pub(super) async fn update_worker(
        &self,
        user_id: Option<&str>,
        command: UpdateWorkerCommand,
        idempotency_key: &str,
    ) -> Result<WorkerMutationResponse> {
        self.worker_mutation(
            user_id,
            Command::UpdateWorker(command),
            idempotency_key,
            "update Worker",
        )
        .await
    }

    pub(super) async fn set_worker_status(
        &self,
        user_id: Option<&str>,
        worker_id: &str,
        expected_revision: u64,
        status: WorkerTargetStatus,
        idempotency_key: &str,
    ) -> Result<WorkerMutationResponse> {
        self.worker_mutation(
            user_id,
            Command::SetWorkerStatus(mitsuro_hive_protocol::SetWorkerStatusCommand {
                worker_id: worker_id.to_string(),
                expected_revision,
                status,
            }),
            idempotency_key,
            "set Worker status",
        )
        .await
    }

    async fn worker_mutation(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: &str,
        action: &str,
    ) -> Result<WorkerMutationResponse> {
        match self
            .command(user_id, command, Some(idempotency_key.to_string()))
            .await?
        {
            ResponsePayload::WorkerMutation(response) => Ok(response),
            payload => bail!("Hive {action} returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn grant_worker_governor_recovery(
        &self,
        user_id: Option<&str>,
        worker_id: &str,
        idempotency_key: &str,
    ) -> Result<WorkerGovernorRecoveryResponse> {
        match self
            .command(
                user_id,
                Command::GrantWorkerGovernorRecovery(GrantWorkerGovernorRecoveryCommand {
                    worker_id: worker_id.to_string(),
                }),
                Some(idempotency_key.to_string()),
            )
            .await?
        {
            ResponsePayload::WorkerGovernorRecovery(response) => Ok(response),
            payload => {
                bail!("Hive Worker governor recovery returned unexpected response {payload:?}")
            }
        }
    }

    pub(super) async fn activate_or_resume_worker_workflow(
        &self,
        user_id: Option<&str>,
        command: ActivateOrResumeWorkerWorkflowCommand,
        idempotency_key: &str,
    ) -> Result<WorkerWorkflowResponse> {
        self.worker_workflow_mutation(
            user_id,
            Command::ActivateOrResumeWorkerWorkflow(command),
            idempotency_key,
            "activate or resume Worker Workflow",
        )
        .await
    }

    pub(super) async fn pause_worker_workflow(
        &self,
        user_id: Option<&str>,
        command: WorkerWorkflowLifecycleCommand,
        idempotency_key: &str,
    ) -> Result<WorkerWorkflowResponse> {
        self.worker_workflow_mutation(
            user_id,
            Command::PauseWorkerWorkflow(command),
            idempotency_key,
            "pause Worker Workflow",
        )
        .await
    }

    pub(super) async fn cancel_worker_workflow(
        &self,
        user_id: Option<&str>,
        command: WorkerWorkflowLifecycleCommand,
        idempotency_key: &str,
    ) -> Result<WorkerWorkflowResponse> {
        self.worker_workflow_mutation(
            user_id,
            Command::CancelWorkerWorkflow(command),
            idempotency_key,
            "cancel Worker Workflow",
        )
        .await
    }

    pub(super) async fn resolve_worker_goal_acceptance(
        &self,
        user_id: Option<&str>,
        command: ResolveWorkerGoalAcceptanceCommand,
        idempotency_key: &str,
    ) -> Result<WorkerGoalAcceptanceResponse> {
        match self
            .command(
                user_id,
                Command::ResolveWorkerGoalAcceptance(command),
                Some(idempotency_key.to_string()),
            )
            .await?
        {
            ResponsePayload::WorkerGoalAcceptance(response) => Ok(response),
            payload => bail!(
                "Hive resolve Worker Goal acceptance returned unexpected response {payload:?}"
            ),
        }
    }

    async fn worker_workflow_mutation(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: &str,
        action: &str,
    ) -> Result<WorkerWorkflowResponse> {
        match self
            .command(user_id, command, Some(idempotency_key.to_string()))
            .await?
        {
            ResponsePayload::WorkerWorkflow(response) => Ok(response),
            payload => bail!("Hive {action} returned unexpected response {payload:?}"),
        }
    }

    pub(super) async fn set_worker_workspace(
        &self,
        user_id: Option<&str>,
        command: SetWorkerWorkspaceCommand,
        idempotency_key: &str,
    ) -> Result<WorkerWorkspaceResponse> {
        match self
            .command(
                user_id,
                Command::SetWorkerWorkspace(command),
                Some(idempotency_key.to_string()),
            )
            .await?
        {
            ResponsePayload::WorkerWorkspace(response) => Ok(response),
            payload => bail!("Hive Worker workspace returned unexpected response {payload:?}"),
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
        self.expect_session_control(
            user_id,
            resume_command(session_id),
            request_key(idempotency_key, unique_key(&format!("resume:{session_id}"))),
            session_id,
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
        self.expect_session_control(
            user_id,
            pause_command(session_id),
            request_key(idempotency_key, unique_key(&format!("pause:{session_id}"))),
            session_id,
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
        self.expect_session_control(
            user_id,
            schedule_command(session_id, wake_at_unix_ms, reason),
            request_key(
                idempotency_key,
                unique_key(&format!("schedule:{session_id}:{wake_at_unix_ms}")),
            ),
            session_id,
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
        self.expect_session_control(
            user_id,
            cancel_command(session_id),
            request_key(idempotency_key, unique_key(&format!("cancel:{session_id}"))),
            session_id,
            "cancel session",
        )
        .await
    }

    pub(super) async fn stop_worker_conversation(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<WorkerMutationResponse> {
        match self
            .command(
                user_id,
                stop_worker_conversation_command(session_id),
                Some(request_key(
                    idempotency_key,
                    unique_key(&format!("worker-stop:{session_id}")),
                )),
            )
            .await?
        {
            ResponsePayload::WorkerMutation(response) => Ok(response),
            payload => bail!("Hive Worker Stop returned unexpected response {payload:?}"),
        }
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
    ) -> Result<DaemonInputAcceptance> {
        self.expect_input_acceptance(
            user_id,
            message_command(session_id, message),
            request_key(
                idempotency_key,
                unique_key(&format!("message:{session_id}")),
            ),
            session_id,
            "send message",
        )
        .await
    }

    pub(super) async fn send_worker_message(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        message: &str,
        idempotency_key: Option<&str>,
    ) -> Result<DaemonInputAcceptance> {
        self.expect_input_acceptance(
            user_id,
            worker_message_command(session_id, message),
            request_key(
                idempotency_key,
                unique_key(&format!("worker-message:{session_id}")),
            ),
            session_id,
            "send Worker message",
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

    pub(super) async fn group_archive(
        &self,
        user_id: Option<&str>,
        group_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<()> {
        self.expect_ack(
            user_id,
            Command::GroupArchive(GroupArchiveCommand {
                group_id: group_id.to_string(),
            }),
            request_key(
                idempotency_key,
                unique_key(&format!("group-archive:{group_id}")),
            ),
            "archive the group",
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
    ) -> Result<DaemonInputAcceptance> {
        self.expect_input_acceptance(
            user_id,
            steer_command(session_id, pending_id, content),
            request_key(idempotency_key, stable_key("steer", pending_id)),
            session_id,
            "steer",
        )
        .await
    }

    pub(super) async fn steer_worker(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        pending_id: &str,
        content: serde_json::Value,
        idempotency_key: Option<&str>,
    ) -> Result<DaemonInputAcceptance> {
        self.expect_input_acceptance(
            user_id,
            worker_steer_command(session_id, pending_id, content),
            request_key(idempotency_key, stable_key("worker-steer", pending_id)),
            session_id,
            "steer Worker",
        )
        .await
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
    ) -> Result<DaemonInputAcceptance> {
        self.expect_input_acceptance(
            user_id,
            user_response_command(session_id, run_id, tool_call_id, response),
            request_key(
                idempotency_key,
                stable_key("response", &format!("{session_id}:{run_id}:{tool_call_id}")),
            ),
            session_id,
            "submit user response",
        )
        .await
    }

    pub(super) async fn worker_user_response(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        run_id: &str,
        tool_call_id: &str,
        response: &str,
        idempotency_key: Option<&str>,
    ) -> Result<DaemonInputAcceptance> {
        self.expect_input_acceptance(
            user_id,
            worker_user_response_command(session_id, run_id, tool_call_id, response),
            request_key(
                idempotency_key,
                stable_key(
                    "worker-response",
                    &format!("{session_id}:{run_id}:{tool_call_id}"),
                ),
            ),
            session_id,
            "submit Worker user response",
        )
        .await
    }

    async fn expect_input_acceptance(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: String,
        session_id: &str,
        operation: &str,
    ) -> Result<DaemonInputAcceptance> {
        match self
            .command(user_id, command, Some(idempotency_key))
            .await?
        {
            ResponsePayload::Ack(ack) if ack.accepted => Ok(DaemonInputAcceptance::Ack(ack)),
            ResponsePayload::Ack(ack) => Err(HiveDaemonError::Remote {
                code: "conflict".to_string(),
                message: format!(
                    "Hive daemon declined to {operation}: {}",
                    ack.message
                        .unwrap_or_else(|| "no reason provided".to_string())
                ),
            }
            .into()),
            ResponsePayload::WorkerConversationInput(response) => {
                validate_worker_input_acceptance(&response, session_id)?;
                Ok(DaemonInputAcceptance::Worker(response))
            }
            payload => bail!("Hive {operation} returned unexpected response {payload:?}"),
        }
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

    async fn expect_session_control(
        &self,
        user_id: Option<&str>,
        command: Command,
        idempotency_key: String,
        session_id: &str,
        operation: &str,
    ) -> Result<()> {
        decode_session_control_response(
            self.command(user_id, command, Some(idempotency_key))
                .await?,
            session_id,
            operation,
        )
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

fn decode_session_control_response(
    payload: ResponsePayload,
    expected_session_id: &str,
    operation: &str,
) -> Result<()> {
    match payload {
        ResponsePayload::Session(response) if response.session_id == expected_session_id => Ok(()),
        ResponsePayload::Session(response) => bail!(
            "Hive {operation} returned session '{}' instead of '{expected_session_id}'",
            response.session_id
        ),
        // Pre-SessionProjection daemons acknowledged these controls without a
        // state projection. Preserve that exact legacy success shape only.
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

fn stop_worker_conversation_command(session_id: &str) -> Command {
    Command::StopWorkerConversation(SessionCommand {
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

fn worker_message_command(session_id: &str, message: &str) -> Command {
    Command::WorkerSendMessage(MessageCommand {
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

fn worker_steer_command(session_id: &str, pending_id: &str, content: serde_json::Value) -> Command {
    Command::WorkerSteer(SteerCommand {
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

fn worker_user_response_command(
    session_id: &str,
    run_id: &str,
    tool_call_id: &str,
    response: &str,
) -> Command {
    Command::WorkerUserResponse(UserResponseCommand {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        response: response.to_string(),
    })
}

fn validate_worker_input_acceptance(
    response: &WorkerConversationInputResponse,
    expected_session_id: &str,
) -> Result<()> {
    if response.session_id != expected_session_id
        || response.worker_id.trim().is_empty()
        || response.run_id.trim().is_empty()
    {
        bail!("Hive Worker input acceptance has an invalid durable binding");
    }
    match response.disposition {
        WorkerConversationInputDisposition::Queued
            if response.canonical_message_id.is_some_and(|id| id > 0)
                && response.staged_input_id.is_none() =>
        {
            Ok(())
        }
        WorkerConversationInputDisposition::Staged
            if response.canonical_message_id.is_none()
                && response
                    .staged_input_id
                    .as_deref()
                    .is_some_and(|id| !id.trim().is_empty()) =>
        {
            Ok(())
        }
        _ => bail!("Hive Worker input acceptance has an invalid disposition projection"),
    }
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
    use std::path::Path;

    #[cfg(unix)]
    use chrono::{TimeZone, Utc};
    #[cfg(unix)]
    use mitsuro_core::storage::{
        accept_worker_conversation_input_in_transaction, AcceptWorkerConversationInput,
        AcceptWorkerConversationInputResult, Database, HiveRunExecutionContextV1, HiveRunStore,
        HiveWorker, HiveWorkerStore, NewHiveWorker, WorkerConversationLane,
    };
    #[cfg(unix)]
    use mitsuro_hive_protocol::{
        read_frame, unix_time_millis, write_frame, AuthPolicy, ClientFrame, DaemonRuntimeStats,
        ExtensionEvent, LaggedEvent, ProtocolErrorPayload, ProtocolVersion, RecoverResponse,
        ReplayGapEvent, ResponseEnvelope, RuntimeEvent, ServerFrame, SubscriptionAccepted,
    };
    use mitsuro_hive_protocol::{HiveIpcClientConfig, IpcKey};
    #[cfg(unix)]
    use rusqlite::{params, Transaction, TransactionBehavior};
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
    fn worker_chat_fixture(database_path: &Path) -> HiveWorker {
        let database = Database::new(database_path).expect("database should initialize");
        database
            .conn()
            .execute(
                "INSERT INTO sessions (
                     id, title, created_at, updated_at, session_type,
                     workspace_mode, working_dir, project_dir
                 ) VALUES (
                     'worker-dm', 'Worker DM', '2026-08-27T00:00:00.000000Z',
                     '2026-08-27T00:00:00.000000Z', 'hive', 'neutral', NULL, NULL
                 )",
                [],
            )
            .expect("Worker DM session should exist");
        let worker = HiveWorkerStore::new(
            Database::new(database_path).expect("Worker store database should open"),
        )
        .create(&NewHiveWorker {
            dm_session_id: Some("worker-dm".into()),
            ..NewHiveWorker::new("worker-one")
        })
        .expect("Worker should bind its exact DM");
        database
            .conn()
            .execute(
                "INSERT INTO hive_controllers (
                     id, scope_key, session_id, status, timezone,
                     max_concurrent_runs, worker_id, created_at, updated_at
                 ) VALUES (
                     'worker-controller', 'worker:worker-one', 'worker-dm', 'active', 'UTC',
                     1, ?1, '2026-08-27T00:00:00.000000Z',
                     '2026-08-27T00:00:00.000000Z'
                 )",
                [&worker.id],
            )
            .expect("Worker controller should exist");
        worker
    }

    #[cfg(unix)]
    fn seed_worker_chat_run(
        database_path: &Path,
        worker: &HiveWorker,
        input_id: &str,
        run_id: &str,
        body: &str,
        event_sequence: i64,
    ) -> i64 {
        let database = Database::new(database_path).expect("Worker run database should open");
        let tx = Transaction::new_unchecked(database.conn(), TransactionBehavior::Immediate)
            .expect("Worker run transaction should start");
        let accepted = accept_worker_conversation_input_in_transaction(
            &tx,
            &AcceptWorkerConversationInput {
                input_id: input_id.to_string(),
                request_id: input_id.to_string(),
                worker_id: worker.id.clone(),
                owner_user_id: worker.user_id.clone(),
                session_id: "worker-dm".into(),
                controller_id: "worker-controller".into(),
                body: body.to_string(),
                accepted_at: Utc
                    .with_ymd_and_hms(2026, 8, 27, 0, 0, 1)
                    .single()
                    .expect("test timestamp should exist"),
                new_run_id: run_id.to_string(),
                run_config: serde_json::json!({
                    "model": worker.model,
                    "model_key": worker.model_key,
                    "model_catalog_revision": worker.model_catalog_revision,
                    "permission_mode": worker.permission_mode.as_str(),
                    "working_dir": null,
                    "project_dir": null,
                }),
                execution_context: HiveRunExecutionContextV1::worker_conversation_neutral(
                    worker.id.clone(),
                    worker.revision,
                    WorkerConversationLane::DirectMessage,
                )
                .expect("Worker DM context should be valid"),
                priority: 0,
                concurrency_key: Some(format!("worker:{}:dm", worker.id)),
                max_attempts: 2,
            },
        )
        .expect("Worker input should be accepted");
        let (accepted_run_id, message_id) = match accepted {
            AcceptWorkerConversationInputResult::Queued { run_id, message_id } => {
                (run_id, message_id)
            }
            AcceptWorkerConversationInputResult::Staged { .. } => {
                panic!("first Worker fixture input unexpectedly staged")
            }
        };
        assert_eq!(accepted_run_id, run_id);
        tx.execute(
            "INSERT INTO hive_controller_events (
                 controller_id, sequence, event_type, run_id, payload_json, created_at
             ) VALUES (
                 'worker-controller', ?1, 'run_queued', ?2, ?3,
                 '2026-08-27T00:00:01.000000Z'
             )",
            params![
                event_sequence,
                run_id,
                serde_json::json!({"run_id": run_id}).to_string(),
            ],
        )
        .expect("Worker fixture event should exist");
        tx.commit().expect("Worker run fixture should commit");
        message_id
    }

    #[cfg(unix)]
    fn worker_agentic_event(
        session_id: &str,
        run_id: &str,
        sequence: i64,
        payload: serde_json::Value,
    ) -> EventEnvelope {
        EventEnvelope {
            version: ProtocolVersion::CURRENT,
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
            sequence: Some(sequence),
            emitted_at_unix_ms: unix_time_millis(),
            event: HiveEvent::Extension(ExtensionEvent {
                name: "agentic_event".into(),
                payload,
            }),
        }
    }

    #[cfg(unix)]
    async fn send_worker_success_events(
        stream: &mut UnixStream,
        session_id: &str,
        worker_id: &str,
        run_id: &str,
        first_sequence: i64,
    ) {
        let payloads = [
            serde_json::json!({
                "type": "worker_response_pending",
                "worker_id": worker_id,
                "session_id": session_id,
                "run_id": run_id,
            }),
            serde_json::json!({
                "type": "worker_response_committed",
                "worker_id": worker_id,
                "session_id": session_id,
                "run_id": run_id,
            }),
            serde_json::json!({"type": "turn_complete", "turn": 1, "has_more": false}),
            serde_json::json!({
                "type": "finish",
                "session_id": session_id,
                "stop_reason": "completed",
            }),
        ];
        for (offset, payload) in payloads.into_iter().enumerate() {
            write_frame(
                stream,
                &ServerFrame::Event(worker_agentic_event(
                    session_id,
                    run_id,
                    first_sequence + offset as i64,
                    payload,
                )),
            )
            .await
            .expect("Worker success event should write");
        }
    }

    #[cfg(unix)]
    fn seed_materialized_staged_worker_run(
        database_path: &Path,
        worker: &HiveWorker,
        predecessor_run_id: &str,
        staged_input_id: &str,
        successor_run_id: &str,
        body: &str,
        successor_event_sequence: i64,
    ) -> i64 {
        let database = Database::new(database_path).expect("Worker staging database should open");
        let tx = Transaction::new_unchecked(database.conn(), TransactionBehavior::Immediate)
            .expect("Worker staging transaction should start");
        let staged = accept_worker_conversation_input_in_transaction(
            &tx,
            &AcceptWorkerConversationInput {
                input_id: staged_input_id.to_string(),
                request_id: staged_input_id.to_string(),
                worker_id: worker.id.clone(),
                owner_user_id: worker.user_id.clone(),
                session_id: "worker-dm".into(),
                controller_id: "worker-controller".into(),
                body: body.to_string(),
                accepted_at: Utc
                    .with_ymd_and_hms(2026, 8, 27, 0, 0, 2)
                    .single()
                    .expect("test timestamp should exist"),
                new_run_id: "unused-staged-run-id".into(),
                run_config: serde_json::json!({
                    "model": worker.model,
                    "model_key": worker.model_key,
                    "model_catalog_revision": worker.model_catalog_revision,
                    "permission_mode": worker.permission_mode.as_str(),
                    "working_dir": null,
                    "project_dir": null,
                }),
                execution_context: HiveRunExecutionContextV1::worker_conversation_neutral(
                    worker.id.clone(),
                    worker.revision,
                    WorkerConversationLane::DirectMessage,
                )
                .expect("Worker DM context should be valid"),
                priority: 0,
                concurrency_key: Some(format!("worker:{}:dm", worker.id)),
                max_attempts: 2,
            },
        )
        .expect("second Worker input should stage");
        let staged = match staged {
            AcceptWorkerConversationInputResult::Staged {
                active_run_id,
                input,
            } => {
                assert_eq!(active_run_id, predecessor_run_id);
                input
            }
            AcceptWorkerConversationInputResult::Queued { .. } => {
                panic!("second Worker fixture input unexpectedly queued")
            }
        };
        assert_eq!(staged.id, staged_input_id);

        tx.execute(
            "INSERT INTO messages (
                 session_id, role, content, created_at, idempotency_key
             ) VALUES (
                 'worker-dm', 'user', ?1, '2026-08-27T00:00:03.000000Z', ?2
             )",
            params![
                serde_json::json!([{ "type": "text", "text": body }]).to_string(),
                format!("worker-request:{staged_input_id}:canonical"),
            ],
        )
        .expect("materialized Worker input message should exist");
        let canonical_message_id = tx.last_insert_rowid();
        tx.commit()
            .expect("staged Worker fixture should commit before successor insert");

        let store = HiveRunStore::new(
            Database::new(database_path).expect("Worker successor store should open"),
        );
        let mut successor = store
            .get_run(predecessor_run_id)
            .expect("predecessor lookup should succeed")
            .expect("predecessor should exist");
        successor.id = successor_run_id.to_string();
        successor.objective = body.to_string();
        successor.objective_message_id = Some(canonical_message_id);
        successor.conversation_through_message_id = Some(canonical_message_id);
        successor.created_at = "2026-08-27T00:00:03.000000Z".into();
        successor.available_at = successor.created_at.clone();
        successor.updated_at = successor.created_at.clone();
        if let Some(governor) = successor.governor.as_mut() {
            governor.run_id = successor_run_id.to_string();
        }
        store
            .insert_run(&successor)
            .expect("materialized Worker successor should exist");

        let database =
            Database::new(database_path).expect("Worker projection database should open");
        let tx = Transaction::new_unchecked(database.conn(), TransactionBehavior::Immediate)
            .expect("Worker projection transaction should start");
        // This transport regression supplies the already-proven materialized
        // projection directly; response-commit provenance is exercised in the
        // core storage suite and is outside this IPC follower fixture.
        tx.execute_batch("DROP TRIGGER hive_worker_conversation_inputs_materialize_guard;")
            .expect("fixture may bypass the response-commit prerequisite");
        tx.execute(
            "UPDATE hive_worker_conversation_inputs
             SET state = 'materialized', canonical_message_id = ?2,
                 assigned_run_id = ?3,
                 materialized_at = '2026-08-27T00:00:03.000000Z'
             WHERE id = ?1",
            params![staged_input_id, canonical_message_id, successor_run_id],
        )
        .expect("staged Worker input should point at its exact successor");
        tx.execute(
            "INSERT INTO hive_controller_events (
                 controller_id, sequence, event_type, run_id, payload_json, created_at
             ) VALUES (
                 'worker-controller', ?1, 'run_queued', ?2, ?3,
                 '2026-08-27T00:00:03.000000Z'
             )",
            params![
                successor_event_sequence,
                successor_run_id,
                serde_json::json!({"run_id": successor_run_id}).to_string(),
            ],
        )
        .expect("successor Worker fixture event should exist");
        tx.commit()
            .expect("materialized Worker projection should commit");
        canonical_message_id
    }

    #[cfg(unix)]
    fn mark_worker_run_succeeded_for_replay(database_path: &Path, run_id: &str) {
        Database::new(database_path)
            .expect("terminal replay database should open")
            .conn()
            .execute(
                "UPDATE hive_runs
                 SET status = 'succeeded', attempt_count = 1,
                     started_at = '2026-08-27T00:00:04.000000Z',
                     finished_at = '2026-08-27T00:00:05.000000Z',
                     updated_at = '2026-08-27T00:00:05.000000Z'
                 WHERE id = ?1",
                [run_id],
            )
            .expect("transport replay fixture should be durably terminal");
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
        assert!(matches!(
            stop_worker_conversation_command("worker-dm"),
            Command::StopWorkerConversation(_)
        ));
        assert!(matches!(delete_command("s"), Command::DeleteSession(_)));
        assert!(matches!(
            message_command("s", "hello"),
            Command::SendMessage(_)
        ));
        assert!(matches!(
            worker_message_command("worker-dm", "hello"),
            Command::WorkerSendMessage(_)
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
            worker_steer_command("worker-dm", "p", serde_json::json!([])),
            Command::WorkerSteer(_)
        ));
        assert!(matches!(
            tool_approval_command("s", "r", "t", true),
            Command::ToolApproval(_)
        ));
        assert!(matches!(
            user_response_command("s", "r", "t", "yes"),
            Command::UserResponse(_)
        ));
        assert!(matches!(
            worker_user_response_command("worker-dm", "r", "t", "yes"),
            Command::WorkerUserResponse(_)
        ));
    }

    #[test]
    fn session_control_decoder_accepts_exact_projection_and_legacy_ack_only() {
        let projection = ResponsePayload::Session(mitsuro_hive_protocol::SessionResponse {
            session_id: "session-1".to_string(),
            state: serde_json::json!({"status": "paused"}),
        });
        decode_session_control_response(projection, "session-1", "pause session")
            .expect("exact committed session projection must be accepted");
        decode_session_control_response(
            ResponsePayload::Ack(AckResponse {
                accepted: true,
                message: None,
            }),
            "session-1",
            "pause session",
        )
        .expect("legacy accepted acknowledgement remains compatible");

        let wrong_projection = ResponsePayload::Session(mitsuro_hive_protocol::SessionResponse {
            session_id: "session-2".to_string(),
            state: serde_json::json!({"status": "paused"}),
        });
        assert!(
            decode_session_control_response(wrong_projection, "session-1", "pause session")
                .is_err()
        );
        assert!(decode_session_control_response(
            ResponsePayload::Ack(AckResponse {
                accepted: false,
                message: Some("declined".to_string()),
            }),
            "session-1",
            "pause session"
        )
        .is_err());
    }

    #[test]
    fn worker_input_acceptance_is_exactly_bound_and_shape_checked() {
        let queued = WorkerConversationInputResponse {
            worker_id: "worker-1".into(),
            session_id: "worker-dm".into(),
            disposition: WorkerConversationInputDisposition::Queued,
            run_id: "run-1".into(),
            canonical_message_id: Some(41),
            staged_input_id: None,
        };
        assert!(validate_worker_input_acceptance(&queued, "worker-dm").is_ok());

        let staged = WorkerConversationInputResponse {
            disposition: WorkerConversationInputDisposition::Staged,
            canonical_message_id: None,
            staged_input_id: Some("input-2".into()),
            ..queued
        };
        assert!(validate_worker_input_acceptance(&staged, "worker-dm").is_ok());
        assert!(validate_worker_input_acceptance(&staged, "other-dm").is_err());

        let malformed = WorkerConversationInputResponse {
            canonical_message_id: Some(42),
            ..staged
        };
        assert!(validate_worker_input_acceptance(&malformed, "worker-dm").is_err());
    }

    #[test]
    fn actor_preserves_the_authenticated_user_exactly() {
        assert_eq!(actor(Some("alice")).user_id.as_deref(), Some("alice"));
        assert_eq!(actor(None).user_id, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn introduction_actions_preserve_exact_worker_actor_and_replay_key() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..5 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                let payload = if index == 0 {
                    ready_stats()
                } else {
                    ResponsePayload::WorkerIntroductionAction(WorkerIntroductionActionResponse {
                        worker_id: "worker-1".into(),
                        session_id: "worker-dm".into(),
                        run_id: (index == 1).then(|| "retry-run".into()),
                        status: match index {
                            1 => "queued",
                            2 => "skipped",
                            3 => "confirmed",
                            _ => "awaiting_context",
                        }
                        .into(),
                        autonomy_eligible: matches!(index, 2 | 3),
                        cancellation_requested: false,
                    })
                };
                respond(&mut stream, &request, payload).await;
                requests.push(request);
            }
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "introduction-action-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let retried = control
            .retry_worker_introduction(Some("alice"), "worker-1", "retry-key")
            .await
            .expect("retry should decode");
        assert_eq!(retried.run_id.as_deref(), Some("retry-run"));
        let skipped = control
            .skip_worker_introduction(Some("alice"), "worker-1", "skip-key")
            .await
            .expect("skip should decode");
        assert!(skipped.autonomy_eligible);
        let confirmed = control
            .confirm_worker_introduction(
                Some("alice"),
                ConfirmWorkerIntroductionCommand {
                    worker_id: "worker-1".into(),
                    proposal_id: "proposal-1".into(),
                    proposal_revision: 3,
                    selected_facts: vec![mitsuro_hive_protocol::WorkerIntroductionSelectedFact {
                        fact_id: "fact-1".into(),
                        final_statement: "Help with runtime reliability.".into(),
                    }],
                },
                "confirm-key",
            )
            .await
            .expect("confirm should decode");
        assert!(confirmed.autonomy_eligible);
        let returned = control
            .return_worker_introduction_to_context(
                Some("alice"),
                ReturnWorkerIntroductionToContextCommand {
                    worker_id: "worker-1".into(),
                    proposal_id: "proposal-1".into(),
                    proposal_revision: 3,
                    decision: mitsuro_hive_protocol::WorkerIntroductionReturnDecision::KeepTalking,
                },
                "keep-key",
            )
            .await
            .expect("keep talking should decode");
        assert_eq!(returned.status, "awaiting_context");

        let requests = server.await.expect("test daemon should finish");
        let retry = &requests[1];
        assert_eq!(retry.actor.user_id.as_deref(), Some("alice"));
        assert_eq!(retry.idempotency_key, "retry-key");
        assert!(matches!(
            &retry.command,
            Command::RetryWorkerIntroduction(command) if command.worker_id == "worker-1"
        ));
        let skip = &requests[2];
        assert_eq!(skip.actor.user_id.as_deref(), Some("alice"));
        assert_eq!(skip.idempotency_key, "skip-key");
        assert!(matches!(
            &skip.command,
            Command::SkipWorkerIntroduction(command) if command.worker_id == "worker-1"
        ));
        let confirm = &requests[3];
        assert_eq!(confirm.actor.user_id.as_deref(), Some("alice"));
        assert_eq!(confirm.idempotency_key, "confirm-key");
        assert!(matches!(
            &confirm.command,
            Command::ConfirmWorkerIntroduction(command)
                if command.worker_id == "worker-1"
                    && command.proposal_id == "proposal-1"
                    && command.proposal_revision == 3
                    && command.selected_facts.len() == 1
        ));
        let keep = &requests[4];
        assert_eq!(keep.actor.user_id.as_deref(), Some("alice"));
        assert_eq!(keep.idempotency_key, "keep-key");
        assert!(matches!(
            &keep.command,
            Command::ReturnWorkerIntroductionToContext(command)
                if command.worker_id == "worker-1"
                    && command.decision
                        == mitsuro_hive_protocol::WorkerIntroductionReturnDecision::KeepTalking
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worker_mutations_preserve_actor_revision_and_replay_key() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..4 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                let payload = match index {
                    0 => ready_stats(),
                    1 | 2 => ResponsePayload::WorkerMutation(WorkerMutationResponse {
                        worker_id: "worker-1".into(),
                        revision: index as u64 + 1,
                        status: if index == 2 { "paused" } else { "active" }.into(),
                        cancellation_requests: Vec::new(),
                        attention: Vec::new(),
                    }),
                    3 => ResponsePayload::WorkerGovernorRecovery(WorkerGovernorRecoveryResponse {
                        worker_id: "worker-1".into(),
                        grant_id: Some("recovery-grant-1".into()),
                        expires_at: Some("2026-08-25T01:05:00.000000Z".into()),
                        status: "granted".into(),
                        bypass_unresolved_provider_call: true,
                    }),
                    _ => unreachable!(),
                };
                respond(&mut stream, &request, payload).await;
                requests.push(request);
            }
            requests
        });
        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "worker-mutation-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        control
            .update_worker(
                Some("alice"),
                UpdateWorkerCommand {
                    worker_id: "worker-1".into(),
                    expected_revision: 1,
                    display_name: "Worker One".into(),
                    avatar_color: None,
                    model: None,
                    model_key: None,
                    model_catalog_revision: None,
                    permission_mode: "supervised".into(),
                    autonomy: "manual".into(),
                    heartbeat_interval_secs: None,
                    identity: None,
                    soul: None,
                },
                "worker-update-key",
            )
            .await
            .expect("update response should decode");
        control
            .set_worker_status(
                Some("alice"),
                "worker-1",
                2,
                WorkerTargetStatus::Paused,
                "worker-pause-key",
            )
            .await
            .expect("pause response should decode");
        let recovery = control
            .grant_worker_governor_recovery(Some("alice"), "worker-1", "worker-recovery-key")
            .await
            .expect("recovery response should decode");
        assert_eq!(recovery.grant_id.as_deref(), Some("recovery-grant-1"));

        let requests = server.await.expect("test daemon should finish");
        assert_eq!(requests[1].actor.user_id.as_deref(), Some("alice"));
        assert_eq!(requests[1].idempotency_key, "worker-update-key");
        assert!(matches!(
            &requests[1].command,
            Command::UpdateWorker(command)
                if command.worker_id == "worker-1" && command.expected_revision == 1
        ));
        assert_eq!(requests[2].actor.user_id.as_deref(), Some("alice"));
        assert_eq!(requests[2].idempotency_key, "worker-pause-key");
        assert!(matches!(
            &requests[2].command,
            Command::SetWorkerStatus(command)
                if command.worker_id == "worker-1"
                    && command.expected_revision == 2
                    && command.status == WorkerTargetStatus::Paused
        ));
        assert_eq!(requests[3].actor.user_id.as_deref(), Some("alice"));
        assert_eq!(requests[3].idempotency_key, "worker-recovery-key");
        assert!(matches!(
            &requests[3].command,
            Command::GrantWorkerGovernorRecovery(command)
                if command.worker_id == "worker-1"
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worker_workflow_and_workspace_commands_preserve_actor_revisions_and_keys() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..5 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                let payload = match index {
                    0 => ready_stats(),
                    1 => ResponsePayload::WorkerWorkflow(WorkerWorkflowResponse {
                        disposition: mitsuro_hive_protocol::WorkerWorkflowDisposition::Created,
                        worker_id: "worker-1".into(),
                        worker_revision: 7,
                        session_id: "worker-dm".into(),
                        goal_id: "goal-1".into(),
                        goal_revision: 11,
                        goal_status: "active".into(),
                        active: Some(mitsuro_hive_protocol::WorkerWorkflowRunProjection {
                            run_id: "run-1".into(),
                            run_status: "queued".into(),
                            attempt_id: "attempt-1".into(),
                            attempt_status: "running".into(),
                        }),
                        affected_run_ids: Vec::new(),
                        affected_attempt_ids: Vec::new(),
                        cancellation_requests: Vec::new(),
                    }),
                    2 => ResponsePayload::WorkerWorkflow(WorkerWorkflowResponse {
                        disposition: mitsuro_hive_protocol::WorkerWorkflowDisposition::Paused,
                        worker_id: "worker-1".into(),
                        worker_revision: 7,
                        session_id: "worker-dm".into(),
                        goal_id: "goal-1".into(),
                        goal_revision: 12,
                        goal_status: "paused".into(),
                        active: None,
                        affected_run_ids: vec!["run-1".into()],
                        affected_attempt_ids: vec!["attempt-1".into()],
                        cancellation_requests: Vec::new(),
                    }),
                    3 => ResponsePayload::WorkerWorkflow(WorkerWorkflowResponse {
                        disposition: mitsuro_hive_protocol::WorkerWorkflowDisposition::Cancelled,
                        worker_id: "worker-1".into(),
                        worker_revision: 7,
                        session_id: "worker-dm".into(),
                        goal_id: "goal-1".into(),
                        goal_revision: 13,
                        goal_status: "cancelled".into(),
                        active: None,
                        affected_run_ids: Vec::new(),
                        affected_attempt_ids: Vec::new(),
                        cancellation_requests: Vec::new(),
                    }),
                    _ => ResponsePayload::WorkerWorkspace(WorkerWorkspaceResponse {
                        worker_id: "worker-1".into(),
                        revision: 8,
                        session_id: "worker-dm".into(),
                        workspace_mode: mitsuro_hive_protocol::WorkerWorkspaceMode::Selected,
                        working_dir: Some("/work/project".into()),
                        project_dir: Some("/work/project".into()),
                    }),
                };
                respond(&mut stream, &request, payload).await;
                requests.push(request);
            }
            requests
        });
        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "worker-workflow-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        control
            .activate_or_resume_worker_workflow(
                Some("alice"),
                ActivateOrResumeWorkerWorkflowCommand {
                    worker_id: "worker-1".into(),
                    expected_worker_revision: 7,
                    goal_id: "goal-1".into(),
                    expected_goal_revision: 11,
                },
                "workflow-activate-key",
            )
            .await
            .expect("activation response should decode");
        for (pause, key, revision) in [
            (true, "workflow-pause-key", 11),
            (false, "workflow-cancel-key", 12),
        ] {
            let command = WorkerWorkflowLifecycleCommand {
                worker_id: "worker-1".into(),
                expected_worker_revision: 7,
                goal_id: "goal-1".into(),
                expected_goal_revision: revision,
                reason: if pause { "Pause" } else { "Cancel" }.into(),
            };
            if pause {
                control
                    .pause_worker_workflow(Some("alice"), command, key)
                    .await
                    .expect("pause response should decode");
            } else {
                control
                    .cancel_worker_workflow(Some("alice"), command, key)
                    .await
                    .expect("cancel response should decode");
            }
        }
        control
            .set_worker_workspace(
                Some("alice"),
                SetWorkerWorkspaceCommand {
                    worker_id: "worker-1".into(),
                    expected_worker_revision: 7,
                    workspace_mode: mitsuro_hive_protocol::WorkerWorkspaceMode::Selected,
                    working_dir: Some("/work/project".into()),
                    project_dir: Some("/work/project".into()),
                },
                "worker-workspace-key",
            )
            .await
            .expect("workspace response should decode");

        let requests = server.await.expect("test daemon should finish");
        for request in &requests[1..] {
            assert_eq!(request.actor.user_id.as_deref(), Some("alice"));
        }
        assert_eq!(requests[1].idempotency_key, "workflow-activate-key");
        assert_eq!(requests[2].idempotency_key, "workflow-pause-key");
        assert_eq!(requests[3].idempotency_key, "workflow-cancel-key");
        assert_eq!(requests[4].idempotency_key, "worker-workspace-key");
        assert!(matches!(
            &requests[1].command,
            Command::ActivateOrResumeWorkerWorkflow(command)
                if command.expected_worker_revision == 7
                    && command.expected_goal_revision == 11
        ));
        assert!(matches!(
            &requests[2].command,
            Command::PauseWorkerWorkflow(command) if command.expected_goal_revision == 11
        ));
        assert!(matches!(
            &requests[3].command,
            Command::CancelWorkerWorkflow(command) if command.expected_goal_revision == 12
        ));
        assert!(matches!(
            &requests[4].command,
            Command::SetWorkerWorkspace(command)
                if command.expected_worker_revision == 7
                    && command.workspace_mode
                        == mitsuro_hive_protocol::WorkerWorkspaceMode::Selected
        ));
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
    async fn daemon_chat_routes_exact_worker_dm_through_typed_input_before_sse() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let database_path = temp.path().join("runtime.db");
        let worker = worker_chat_fixture(&database_path);
        let canonical_message_id = seed_worker_chat_run(
            &database_path,
            &worker,
            "worker-chat-key",
            "worker-run-1",
            "hello Worker",
            1,
        );
        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let expected_worker_id = worker.id.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut live_subscription = None;
            for _ in 0..3 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Stats => respond(&mut stream, &request, ready_stats()).await,
                    Command::Subscribe(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: Some(0),
                            }),
                        )
                        .await;
                        live_subscription = Some(stream);
                        requests.push(request);
                        continue;
                    }
                    Command::WorkerSendMessage(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::WorkerConversationInput(
                                WorkerConversationInputResponse {
                                    worker_id: expected_worker_id.clone(),
                                    session_id: command.session_id.clone(),
                                    disposition: WorkerConversationInputDisposition::Queued,
                                    run_id: "worker-run-1".into(),
                                    canonical_message_id: Some(canonical_message_id),
                                    staged_input_id: None,
                                },
                            ),
                        )
                        .await;
                        let mut live_subscription = live_subscription
                            .take()
                            .expect("Worker send must follow its live subscription");
                        send_worker_success_events(
                            &mut live_subscription,
                            "worker-dm",
                            &expected_worker_id,
                            "worker-run-1",
                            2,
                        )
                        .await;
                    }
                    unexpected => panic!(
                        "Worker /api/chat path emitted an unexpected daemon command: {unexpected:?}"
                    ),
                }
                requests.push(request);
            }
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "worker-chat-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .begin_daemon_chat_turn_for_user(
                database_path,
                "worker-dm",
                "hello Worker",
                None,
                true,
                Some("worker-chat-key"),
            )
            .await
            .expect("Worker chat turn should use the typed daemon path");
        let mut saw_pending = false;
        let mut saw_committed = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .expect("fresh Worker response should reach SSE")
                .expect("fresh Worker response channel should remain open");
            match event {
                AgenticEvent::WorkerResponsePending { run_id, .. } => {
                    assert_eq!(run_id, "worker-run-1");
                    saw_pending = true;
                }
                AgenticEvent::WorkerResponseCommitted { run_id, .. } => {
                    assert_eq!(run_id, "worker-run-1");
                    saw_committed = true;
                }
                AgenticEvent::Finish {
                    session_id,
                    stop_reason,
                } => {
                    assert_eq!(session_id, "worker-dm");
                    assert_eq!(stop_reason, "completed");
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_pending && saw_committed);
        drop(manager);

        let requests = server.await.expect("test daemon should finish");
        let Command::Subscribe(subscribe) = &requests[1].command else {
            panic!("Worker chat must subscribe before its typed send");
        };
        assert_eq!(subscribe.session_id, "worker-dm");
        assert_eq!(subscribe.after_sequence, None);
        assert_eq!(subscribe.replay_limit, Some(0));
        assert_eq!(requests[2].idempotency_key, "worker-chat-key");
        assert!(matches!(
            &requests[2].command,
            Command::WorkerSendMessage(command)
                if command.session_id == "worker-dm" && command.message == "hello Worker"
        ));
        assert!(requests
            .iter()
            .all(|request| !matches!(&request.command, Command::StartSession(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_chat_replays_only_the_exact_queued_worker_run_after_transport_retry() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let database_path = temp.path().join("runtime.db");
        let worker = worker_chat_fixture(&database_path);
        let canonical_message_id = seed_worker_chat_run(
            &database_path,
            &worker,
            "worker-replay-key",
            "worker-replay-run",
            "retry this Worker message",
            1,
        );
        mark_worker_run_succeeded_for_replay(&database_path, "worker-replay-run");

        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let expected_worker_id = worker.id.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut held_live_subscription = None;
            for _ in 0..4 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Stats => respond(&mut stream, &request, ready_stats()).await,
                    Command::Subscribe(command) if command.after_sequence.is_none() => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: Some(8),
                            }),
                        )
                        .await;
                        held_live_subscription = Some(stream);
                        requests.push(request);
                        continue;
                    }
                    Command::WorkerSendMessage(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::WorkerConversationInput(
                                WorkerConversationInputResponse {
                                    worker_id: expected_worker_id.clone(),
                                    session_id: command.session_id.clone(),
                                    disposition: WorkerConversationInputDisposition::Queued,
                                    run_id: "worker-replay-run".into(),
                                    canonical_message_id: Some(canonical_message_id),
                                    staged_input_id: None,
                                },
                            ),
                        )
                        .await;
                    }
                    Command::Subscribe(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: Some(8),
                            }),
                        )
                        .await;
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(EventEnvelope {
                                version: ProtocolVersion::CURRENT,
                                session_id: Some("worker-dm".into()),
                                run_id: None,
                                sequence: None,
                                emitted_at_unix_ms: unix_time_millis(),
                                event: HiveEvent::Lagged(LaggedEvent {
                                    skipped: 3,
                                    resume_after_sequence: Some(1),
                                }),
                            }),
                        )
                        .await
                        .expect("session-scoped lag signal should write");
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(worker_agentic_event(
                                "worker-dm",
                                "unrelated-worker-run",
                                3,
                                serde_json::json!({
                                    "type": "worker_response_pending",
                                    "worker_id": expected_worker_id,
                                    "session_id": "worker-dm",
                                    "run_id": "unrelated-worker-run",
                                }),
                            )),
                        )
                        .await
                        .expect("foreign pending boundary should write");
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(worker_agentic_event(
                                "worker-dm",
                                "unrelated-worker-run",
                                4,
                                serde_json::json!({
                                    "type": "finish",
                                    "session_id": "worker-dm",
                                    "stop_reason": "completed",
                                }),
                            )),
                        )
                        .await
                        .expect("foreign finish boundary should write");
                        send_worker_success_events(
                            &mut stream,
                            "worker-dm",
                            &expected_worker_id,
                            "worker-replay-run",
                            5,
                        )
                        .await;
                    }
                    unexpected => panic!("unexpected queued replay command: {unexpected:?}"),
                }
                requests.push(request);
            }
            drop(held_live_subscription);
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "worker-replay-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .begin_daemon_chat_turn_for_user(
                database_path,
                "worker-dm",
                "retry this Worker message",
                None,
                false,
                Some("worker-replay-key"),
            )
            .await
            .expect("idempotent Worker replay should attach to its exact run");
        let mut lagged = Vec::new();
        let mut observed_run_ids = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .expect("historical Worker response should replay")
                .expect("historical Worker replay channel should remain open");
            match event {
                AgenticEvent::Lagged { skipped } => lagged.push(skipped),
                AgenticEvent::WorkerResponsePending { run_id, .. }
                | AgenticEvent::WorkerResponseCommitted { run_id, .. } => {
                    observed_run_ids.push(run_id);
                }
                AgenticEvent::Finish { stop_reason, .. } => {
                    assert_eq!(stop_reason, "completed");
                    break;
                }
                _ => {}
            }
        }
        assert!(lagged.contains(&1), "replay must force a canonical reload");
        assert!(
            lagged.contains(&3),
            "session-scoped daemon lag must reach the exact follower"
        );
        assert_eq!(observed_run_ids, ["worker-replay-run", "worker-replay-run"]);
        drop(manager);

        let requests = server.await.expect("test daemon should finish");
        assert!(matches!(&requests[1].command, Command::Subscribe(command)
            if command.after_sequence.is_none() && command.replay_limit == Some(0)));
        assert!(matches!(
            &requests[2].command,
            Command::WorkerSendMessage(_)
        ));
        assert!(matches!(&requests[3].command, Command::Subscribe(command)
            if command.after_sequence == Some(0) && command.replay_limit == Some(256)));
        assert!(requests
            .iter()
            .all(|request| !matches!(&request.command, Command::StartSession(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_chat_fails_closed_on_session_scoped_worker_replay_gap() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let database_path = temp.path().join("runtime.db");
        let worker = worker_chat_fixture(&database_path);
        let canonical_message_id = seed_worker_chat_run(
            &database_path,
            &worker,
            "worker-gap-key",
            "worker-gap-run",
            "retry pruned Worker message",
            1,
        );

        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let expected_worker_id = worker.id.clone();
        let server = tokio::spawn(async move {
            let mut held_live_subscription = None;
            let mut requests = Vec::new();
            for _ in 0..4 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Stats => respond(&mut stream, &request, ready_stats()).await,
                    Command::Subscribe(command) if command.after_sequence.is_none() => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: Some(5),
                            }),
                        )
                        .await;
                        held_live_subscription = Some(stream);
                        requests.push(request);
                        continue;
                    }
                    Command::WorkerSendMessage(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::WorkerConversationInput(
                                WorkerConversationInputResponse {
                                    worker_id: expected_worker_id.clone(),
                                    session_id: command.session_id.clone(),
                                    disposition: WorkerConversationInputDisposition::Queued,
                                    run_id: "worker-gap-run".into(),
                                    canonical_message_id: Some(canonical_message_id),
                                    staged_input_id: None,
                                },
                            ),
                        )
                        .await;
                    }
                    Command::Subscribe(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: Some(5),
                            }),
                        )
                        .await;
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(EventEnvelope {
                                version: ProtocolVersion::CURRENT,
                                session_id: Some("worker-dm".into()),
                                run_id: None,
                                sequence: None,
                                emitted_at_unix_ms: unix_time_millis(),
                                event: HiveEvent::ReplayGap(ReplayGapEvent {
                                    requested_after: 0,
                                    earliest_available: 3,
                                }),
                            }),
                        )
                        .await
                        .expect("session-scoped replay gap should write");
                    }
                    unexpected => panic!("unexpected replay-gap command: {unexpected:?}"),
                }
                requests.push(request);
            }
            drop(held_live_subscription);
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "worker-gap-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .begin_daemon_chat_turn_for_user(
                database_path,
                "worker-dm",
                "retry pruned Worker message",
                None,
                false,
                Some("worker-gap-key"),
            )
            .await
            .expect("typed Worker acceptance should establish its follower");
        assert!(matches!(
            receiver.recv().await.expect("reload marker should arrive"),
            AgenticEvent::Lagged { skipped: 1 }
        ));
        let terminal = receiver.recv().await.expect("replay gap should terminate");
        let AgenticEvent::Error { error } = terminal else {
            panic!("expected replay-gap error, got {terminal:?}");
        };
        assert!(error.contains("replay gap"));
        assert!(matches!(
            receiver.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
        drop(manager);

        let requests = server.await.expect("test daemon should finish");
        assert!(matches!(&requests[3].command, Command::Subscribe(command)
            if command.after_sequence == Some(0) && command.replay_limit == Some(256)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_chat_replays_the_exact_materialized_successor_for_a_staged_retry() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let database_path = temp.path().join("runtime.db");
        let worker = worker_chat_fixture(&database_path);
        seed_worker_chat_run(
            &database_path,
            &worker,
            "worker-predecessor-key",
            "worker-predecessor-run",
            "first Worker message",
            1,
        );
        let successor_message_id = seed_materialized_staged_worker_run(
            &database_path,
            &worker,
            "worker-predecessor-run",
            "worker-staged-key",
            "worker-successor-run",
            "second Worker message",
            2,
        );
        mark_worker_run_succeeded_for_replay(&database_path, "worker-successor-run");

        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let expected_worker_id = worker.id.clone();
        let server = tokio::spawn(async move {
            let mut held_live_subscription = None;
            let mut requests = Vec::new();
            for _ in 0..4 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Stats => respond(&mut stream, &request, ready_stats()).await,
                    Command::Subscribe(command) if command.after_sequence.is_none() => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: Some(8),
                            }),
                        )
                        .await;
                        held_live_subscription = Some(stream);
                        requests.push(request);
                        continue;
                    }
                    Command::WorkerSendMessage(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::WorkerConversationInput(
                                WorkerConversationInputResponse {
                                    worker_id: expected_worker_id.clone(),
                                    session_id: command.session_id.clone(),
                                    disposition: WorkerConversationInputDisposition::Staged,
                                    run_id: "worker-predecessor-run".into(),
                                    canonical_message_id: None,
                                    staged_input_id: Some("worker-staged-key".into()),
                                },
                            ),
                        )
                        .await;
                    }
                    Command::Subscribe(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: Some(8),
                            }),
                        )
                        .await;
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(worker_agentic_event(
                                "worker-dm",
                                "unrelated-worker-run",
                                3,
                                serde_json::json!({
                                    "type": "worker_response_pending",
                                    "worker_id": expected_worker_id,
                                    "session_id": "worker-dm",
                                    "run_id": "unrelated-worker-run",
                                }),
                            )),
                        )
                        .await
                        .expect("foreign staged-replay event should write");
                        write_frame(
                            &mut stream,
                            &ServerFrame::Event(worker_agentic_event(
                                "worker-dm",
                                "unrelated-worker-run",
                                4,
                                serde_json::json!({
                                    "type": "finish",
                                    "session_id": "worker-dm",
                                    "stop_reason": "completed",
                                }),
                            )),
                        )
                        .await
                        .expect("foreign staged-replay finish should write");
                        send_worker_success_events(
                            &mut stream,
                            "worker-dm",
                            &expected_worker_id,
                            "worker-successor-run",
                            5,
                        )
                        .await;
                    }
                    unexpected => panic!("unexpected staged replay command: {unexpected:?}"),
                }
                requests.push(request);
            }
            drop(held_live_subscription);
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "worker-staged-replay-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let mut receiver = manager
            .begin_daemon_chat_turn_for_user(
                database_path.clone(),
                "worker-dm",
                "second Worker message",
                None,
                false,
                Some("worker-staged-key"),
            )
            .await
            .expect("staged Worker retry should establish its exact successor follower");
        let mut staged_successors = Vec::new();
        let mut observed_run_ids = Vec::new();
        let mut saw_reload = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .expect("materialized successor response should replay")
                .expect("materialized successor replay channel should remain open");
            match event {
                AgenticEvent::WorkerInputStaged {
                    active_run_id,
                    staged_input_id,
                    successor_run_id,
                    ..
                } => {
                    assert_eq!(active_run_id, "worker-predecessor-run");
                    assert_eq!(staged_input_id, "worker-staged-key");
                    staged_successors.push(successor_run_id);
                }
                AgenticEvent::Lagged { skipped: 1 } => saw_reload = true,
                AgenticEvent::WorkerResponsePending { run_id, .. }
                | AgenticEvent::WorkerResponseCommitted { run_id, .. } => {
                    observed_run_ids.push(run_id);
                }
                AgenticEvent::Finish { stop_reason, .. } => {
                    assert_eq!(stop_reason, "completed");
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(
            staged_successors,
            [None, Some("worker-successor-run".to_string())]
        );
        assert!(
            saw_reload,
            "historical successor replay must reload transcript"
        );
        assert_eq!(
            observed_run_ids,
            ["worker-successor-run", "worker-successor-run"]
        );
        let projection: (String, i64, String) = Database::new(&database_path)
            .expect("projection database should open")
            .conn()
            .query_row(
                "SELECT state, canonical_message_id, assigned_run_id
                 FROM hive_worker_conversation_inputs WHERE id = 'worker-staged-key'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("materialized projection should remain durable");
        assert_eq!(
            projection,
            (
                "materialized".to_string(),
                successor_message_id,
                "worker-successor-run".to_string()
            )
        );
        drop(manager);

        let requests = server.await.expect("test daemon should finish");
        assert!(matches!(&requests[3].command, Command::Subscribe(command)
            if command.after_sequence == Some(1) && command.replay_limit == Some(256)));
        assert!(requests
            .iter()
            .all(|request| !matches!(&request.command, Command::StartSession(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_chat_rejects_a_foreign_worker_response_for_the_requested_dm() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let database_path = temp.path().join("runtime.db");
        worker_chat_fixture(&database_path);
        let database = Database::new(&database_path).expect("foreign Worker database should open");
        database
            .conn()
            .execute(
                "INSERT INTO sessions (
                     id, title, created_at, updated_at, session_type,
                     workspace_mode, working_dir, project_dir
                 ) VALUES (
                     'worker-dm-b', 'Worker DM B', '2026-08-27T00:00:00.000000Z',
                     '2026-08-27T00:00:00.000000Z', 'hive', 'neutral', NULL, NULL
                 )",
                [],
            )
            .expect("foreign Worker DM session should exist");
        let foreign_worker = HiveWorkerStore::new(
            Database::new(&database_path).expect("foreign Worker store should open"),
        )
        .create(&NewHiveWorker {
            dm_session_id: Some("worker-dm-b".into()),
            ..NewHiveWorker::new("worker-two")
        })
        .expect("foreign same-owner Worker should exist");

        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let foreign_worker_id = foreign_worker.id.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut held_live_subscription = None;
            for _ in 0..3 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Stats => respond(&mut stream, &request, ready_stats()).await,
                    Command::Subscribe(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: None,
                            }),
                        )
                        .await;
                        held_live_subscription = Some(stream);
                        requests.push(request);
                        continue;
                    }
                    Command::WorkerSendMessage(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::WorkerConversationInput(
                                WorkerConversationInputResponse {
                                    worker_id: foreign_worker_id.clone(),
                                    // Echoing A's session gets through the wire-shape
                                    // validator; the classified Worker identity must
                                    // still reject B before any follower can spawn.
                                    session_id: command.session_id.clone(),
                                    disposition: WorkerConversationInputDisposition::Queued,
                                    run_id: "foreign-worker-run".into(),
                                    canonical_message_id: Some(1),
                                    staged_input_id: None,
                                },
                            ),
                        )
                        .await;
                    }
                    unexpected => panic!("unexpected foreign Worker command: {unexpected:?}"),
                }
                requests.push(request);
            }
            drop(held_live_subscription);
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "foreign-worker-response-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let error = manager
            .begin_daemon_chat_turn_for_user(
                database_path,
                "worker-dm",
                "message for Worker A",
                None,
                false,
                Some("worker-a-key"),
            )
            .await
            .expect_err("Worker B response must not attach to Worker A's DM");
        assert!(format!("{error:#}").contains("requested durable DM"));
        drop(manager);

        let requests = server.await.expect("test daemon should finish");
        assert!(matches!(&requests[1].command, Command::Subscribe(_)));
        assert!(matches!(
            &requests[2].command,
            Command::WorkerSendMessage(_)
        ));
        assert!(requests
            .iter()
            .all(|request| !matches!(&request.command, Command::StartSession(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn first_worker_chat_rejects_untyped_ack_without_starting_a_generic_session() {
        let temp = tempfile::tempdir().expect("temp directory should exist");
        let database_path = temp.path().join("runtime.db");
        worker_chat_fixture(&database_path);

        let socket_path = temp.path().join("hive.sock");
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let key = IpcKey::generate();
        let server_key = key.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut held_live_subscription = None;
            for _ in 0..3 {
                let (mut stream, request) =
                    accept_authenticated_request(&listener, &server_key).await;
                match &request.command {
                    Command::Stats => respond(&mut stream, &request, ready_stats()).await,
                    Command::Subscribe(command) => {
                        respond(
                            &mut stream,
                            &request,
                            ResponsePayload::SubscriptionAccepted(SubscriptionAccepted {
                                session_id: command.session_id.clone(),
                                high_water_sequence: None,
                            }),
                        )
                        .await;
                        held_live_subscription = Some(stream);
                        requests.push(request);
                        continue;
                    }
                    Command::WorkerSendMessage(_) => {
                        respond(&mut stream, &request, accepted()).await;
                    }
                    unexpected => panic!("unexpected first Worker command: {unexpected:?}"),
                }
                requests.push(request);
            }
            drop(held_live_subscription);
            requests
        });

        let control = HiveDaemonControl::connect_client(HiveIpcClient::new(
            HiveIpcClientConfig::new(socket_path, "worker-ack-fence-test"),
            key,
        ))
        .await
        .expect("healthcheck should succeed");
        let manager = super::super::HiveRuntimeManager::build(Some(control), false);
        let error = manager
            .begin_daemon_chat_turn_for_user(
                database_path,
                "worker-dm",
                "first Worker message",
                None,
                true,
                Some("worker-first-key"),
            )
            .await
            .expect_err("Worker ACK must not cross into generic session startup");
        assert!(format!("{error:#}").contains("refusing generic session startup"));
        drop(manager);

        let requests = server.await.expect("test daemon should finish");
        assert!(requests
            .iter()
            .all(|request| !matches!(&request.command, Command::StartSession(_))));
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
            for _ in 0..5 {
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
            .begin_daemon_chat_turn_for_user(
                temp.path().join("runtime.db"),
                "session-1",
                "hello",
                Some("alice"),
                true,
                None,
            )
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
        let Command::Subscribe(live_cursor) = &requests[1].command else {
            panic!("second request was not the pre-accept live cursor");
        };
        assert_eq!(live_cursor.after_sequence, None);
        assert_eq!(live_cursor.replay_limit, Some(0));
        assert!(matches!(requests[2].command, Command::SendMessage(_)));
        assert!(matches!(requests[3].command, Command::StartSession(_)));
        let Command::Subscribe(subscribe) = &requests[4].command else {
            panic!("fifth request was not the replay subscription");
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
