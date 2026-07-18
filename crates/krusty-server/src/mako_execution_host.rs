//! Agent execution surface hosted inside the standalone Mako process.
//!
//! This module deliberately exposes no HTTP router. It reuses Krusty's mature
//! provider/tool/orchestrator bootstrap while keeping process ownership in the
//! independently supervised daemon.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::storage::{ClaimedMakoRun, DaemonFence, Database, MakoRunStore};
use krusty_core::tools::registry::PermissionMode;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::AbortHandle;

use krusty_core::agent::LoopInput;

use crate::ai_bootstrap::{initialize_models, spawn_model_catalog_refresh};
use crate::mako_runtime::runner::{run_mako_session_inner, MakoExecutionEventSink};
use crate::types::AgenticEvent;
use crate::{build_app_state, AppState, MakoRuntimeMode, ServerConfig};

const DEFAULT_EVENT_CAPACITY: usize = 32;
const MAX_EVENT_CAPACITY: usize = 64;
const MAX_EXECUTION_ERROR_BYTES: usize = 8 * 1024;
const DEFAULT_INPUT_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_CANCEL_GRACE: Duration = Duration::from_secs(5);
const DAEMON_LEASE_NAME: &str = "mako-scheduler";

#[derive(Debug, Clone)]
pub struct MakoExecutionHostConfig {
    pub database_path: PathBuf,
    pub working_dir: PathBuf,
    pub event_capacity: usize,
    pub input_registration_timeout: Duration,
    pub cancel_grace: Duration,
}

impl MakoExecutionHostConfig {
    pub fn new(database_path: PathBuf, working_dir: PathBuf) -> Self {
        Self {
            database_path,
            working_dir,
            event_capacity: DEFAULT_EVENT_CAPACITY,
            input_registration_timeout: DEFAULT_INPUT_REGISTRATION_TIMEOUT,
            cancel_grace: DEFAULT_CANCEL_GRACE,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.event_capacity == 0 || self.event_capacity > MAX_EVENT_CAPACITY {
            bail!("Mako execution event capacity must be between 1 and {MAX_EVENT_CAPACITY}");
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ActiveExecution {
    spec: MakoExecutionSpec,
    abort: AbortHandle,
}

/// Immutable execution inputs captured by the durable scheduler claim. The
/// execution host must never re-resolve these fields from mutable session or
/// runtime-state rows after the run has been leased.
#[derive(Debug, Clone)]
pub(crate) struct MakoExecutionSpec {
    pub(crate) claim: ClaimedMakoRun,
    pub(crate) daemon_instance_id: String,
    pub(crate) working_dir: Option<PathBuf>,
    pub(crate) project_dir: Option<PathBuf>,
    pub(crate) model: String,
    pub(crate) crew_slug: Option<String>,
    pub(crate) permission_mode: PermissionMode,
}

impl MakoExecutionSpec {
    fn from_claim(claim: ClaimedMakoRun, daemon_instance_id: String) -> Result<Self> {
        let session_id = claim
            .run
            .session_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .context("claimed Mako run has no session")?;
        let lease_owner = claim
            .run
            .lease_owner
            .as_deref()
            .context("claimed Mako run has no lease owner")?;
        if lease_owner != daemon_instance_id {
            bail!("claimed Mako run belongs to a different daemon generation");
        }
        if claim.run.lease_token.as_deref() != Some(claim.lease_token.as_str())
            || claim.run.lease_epoch.is_none()
        {
            bail!("claimed Mako run has inconsistent worker lease metadata");
        }
        if claim.run.objective.trim().is_empty() {
            bail!("claimed Mako run objective is empty");
        }
        let working_dir = optional_config_path(&claim.run.config, "working_dir")?;
        let project_dir = optional_config_path(&claim.run.config, "project_dir")?;
        if working_dir.is_none() && project_dir.is_none() {
            bail!(
                "claimed Mako run has no explicit working_dir or project_dir; refusing daemon-default workspace"
            );
        }
        if working_dir.as_ref().is_some_and(|path| !path.is_absolute())
            || project_dir.as_ref().is_some_and(|path| !path.is_absolute())
        {
            bail!("every claimed Mako workspace path must be absolute");
        }
        let model = optional_config_string(&claim.run.config, "model")?
            .context("claimed Mako run has no explicit model")?;
        let crew_slug = optional_config_string(&claim.run.config, "crew_slug")?;
        let permission_mode = optional_config_string(&claim.run.config, "permission_mode")?
            .context("claimed Mako run has no explicit permission_mode")?
            .parse::<PermissionMode>()
            .map_err(|error| anyhow::anyhow!("invalid claimed Mako permission_mode: {error}"))?;
        tracing::debug!(
            run_id = %claim.run.id,
            session_id,
            model = ?model,
            project_dir = ?project_dir,
            crew_slug = ?crew_slug,
            "Captured immutable Mako execution inputs"
        );
        Ok(Self {
            claim,
            daemon_instance_id,
            working_dir,
            project_dir,
            model,
            crew_slug,
            permission_mode,
        })
    }

    pub(crate) fn session_id(&self) -> &str {
        self.claim
            .run
            .session_id
            .as_deref()
            .expect("Mako execution spec was validated with a session")
    }

    pub(crate) fn run_id(&self) -> &str {
        &self.claim.run.id
    }

    fn same_execution(&self, other: &Self) -> bool {
        self.run_id() == other.run_id()
            && self.claim.lease_token == other.claim.lease_token
            && self.claim.run.lease_epoch == other.claim.run.lease_epoch
            && self.daemon_instance_id == other.daemon_instance_id
    }

    fn fence(&self) -> DaemonFence {
        DaemonFence {
            lease_name: DAEMON_LEASE_NAME.to_string(),
            owner_id: self.daemon_instance_id.clone(),
            fencing_token: self
                .claim
                .run
                .lease_epoch
                .expect("Mako execution spec was validated with a lease epoch"),
        }
    }
}

fn optional_config_path(config: &serde_json::Value, key: &str) -> Result<Option<PathBuf>> {
    optional_config_string(config, key).map(|value| value.map(PathBuf::from))
}

fn optional_config_string(config: &serde_json::Value, key: &str) -> Result<Option<String>> {
    match config.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.clone()))
        }
        Some(serde_json::Value::String(_)) => bail!("claimed Mako {key} is empty"),
        Some(_) => bail!("claimed Mako {key} must be a string or null"),
    }
}

pub struct MakoExecutionHost {
    state: AppState,
    active: RwLock<HashMap<String, ActiveExecution>>,
    credential_refresh: Mutex<()>,
    config: MakoExecutionHostConfig,
}

/// Typed boundary between durable scheduling and process-local execution.
/// Deterministic claim/fence failures must not be mistaken for transient
/// provider failures and retried until their attempt budget is exhausted.
#[derive(Debug)]
pub enum MakoExecutionStartError {
    InvalidClaim(String),
    FenceLost(String),
    CredentialReload(String),
    AlreadyExecuting(String),
}

impl MakoExecutionStartError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::AlreadyExecuting(_))
    }
}

impl std::fmt::Display for MakoExecutionStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidClaim(error) => write!(formatter, "invalid Mako execution claim: {error}"),
            Self::FenceLost(error) => write!(formatter, "Mako execution fence rejected: {error}"),
            Self::CredentialReload(error) => {
                write!(
                    formatter,
                    "Mako credential snapshot could not be reloaded: {error}"
                )
            }
            Self::AlreadyExecuting(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for MakoExecutionStartError {}

/// Keeps a hosted run owned by its caller. Dropping the guard aborts the
/// underlying agent task, so a lost scheduler lease cannot leave detached
/// side effects running in the daemon.
pub struct MakoExecutionGuard {
    abort: AbortHandle,
}

impl Drop for MakoExecutionGuard {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

pub struct MakoExecutionRun {
    pub events: mpsc::Receiver<AgenticEvent>,
    pub completion: oneshot::Receiver<std::result::Result<(), String>>,
    guard: MakoExecutionGuard,
}

impl MakoExecutionRun {
    pub fn into_parts(
        self,
    ) -> (
        mpsc::Receiver<AgenticEvent>,
        oneshot::Receiver<std::result::Result<(), String>>,
        MakoExecutionGuard,
    ) {
        (self.events, self.completion, self.guard)
    }
}

impl MakoExecutionHost {
    pub async fn build(config: MakoExecutionHostConfig) -> Result<Arc<Self>> {
        // AgenticEvent has variable-sized tool/web variants. Keep the ingress
        // count small; the scheduler's next bounded hop replaces encoded live
        // payloads above 32 KiB with a fixed safe summary.
        config.validate()?;
        let server_config = ServerConfig {
            port: 0,
            working_dir: config.working_dir.clone(),
        };
        let state = build_app_state(
            &server_config,
            MakoRuntimeMode::ExecutionHost,
            Some(config.database_path.clone()),
        )
        .await
        .context("initializing the Mako execution host")?;
        Ok(Arc::new(Self {
            state,
            active: RwLock::new(HashMap::new()),
            credential_refresh: Mutex::new(()),
            config,
        }))
    }

    pub async fn start(
        self: &Arc<Self>,
        claim: ClaimedMakoRun,
        daemon_instance_id: String,
        wake_reason: String,
    ) -> std::result::Result<MakoExecutionRun, MakoExecutionStartError> {
        let spec = MakoExecutionSpec::from_claim(claim, daemon_instance_id)
            .map_err(|error| MakoExecutionStartError::InvalidClaim(error.to_string()))?;
        self.validate_execution(&spec)
            .await
            .map_err(|error| MakoExecutionStartError::FenceLost(error.to_string()))?;
        self.refresh_credentials_and_catalog_if_needed(&spec.model)
            .await
            .map_err(|error| MakoExecutionStartError::CredentialReload(error.to_string()))?;
        if self
            .state
            .model_registry
            .get_model(&spec.model)
            .await
            .is_none()
        {
            return Err(MakoExecutionStartError::InvalidClaim(format!(
                "claimed model {} is absent from the refreshed model catalog",
                spec.model
            )));
        }
        // Dynamic catalog refresh can involve network I/O. Close the window
        // by validating the exact durable lease again before spawning any
        // model or tool execution.
        self.validate_execution(&spec)
            .await
            .map_err(|error| MakoExecutionStartError::FenceLost(error.to_string()))?;
        let session_id = spec.session_id().to_string();
        let run_id = spec.run_id().to_string();
        let mut active = self.active.write().await;
        if let Some(existing) = active.get(&session_id) {
            return Err(MakoExecutionStartError::AlreadyExecuting(format!(
                "Mako session {session_id} is already executing run {}",
                existing.spec.run_id()
            )));
        }

        let (event_tx, event_rx) = mpsc::channel(self.config.event_capacity);
        let sink = MakoExecutionEventSink::Bounded(event_tx);
        let state = self.state.clone();
        let manager = state.mako_runtime.clone();
        let session_for_task = session_id.clone();
        let run_for_task = run_id.clone();
        let spec_for_task = spec.clone();
        let error_sink = sink.clone();
        let runner = tokio::spawn(async move {
            let result = run_mako_session_inner(
                state,
                session_for_task,
                run_for_task,
                wake_reason,
                Some(spec_for_task),
                sink,
                manager,
                false,
            )
            .await;
            if let Err(error) = &result {
                let _ = error_sink
                    .send(AgenticEvent::Error {
                        error: bounded_execution_error(error.to_string()),
                    })
                    .await;
            }
            result.map_err(|error| bounded_execution_error(error.to_string()))
        });
        let abort = runner.abort_handle();
        active.insert(
            session_id.clone(),
            ActiveExecution {
                spec,
                abort: abort.clone(),
            },
        );
        drop(active);

        let (completion_tx, completion_rx) = oneshot::channel();
        let weak_host = Arc::downgrade(self);
        tokio::spawn(async move {
            let result = match runner.await {
                Ok(result) => result,
                Err(error) if error.is_cancelled() => {
                    Err("Mako execution task was cancelled".to_string())
                }
                Err(error) => Err(bounded_execution_error(format!(
                    "Mako execution task failed: {error}"
                ))),
            };
            if let Some(host) = weak_host.upgrade() {
                let mut active = host.active.write().await;
                if active
                    .get(&session_id)
                    .is_some_and(|entry| entry.spec.run_id() == run_id)
                {
                    active.remove(&session_id);
                }
            }
            let _ = completion_tx.send(result);
        });

        Ok(MakoExecutionRun {
            events: event_rx,
            completion: completion_rx,
            guard: MakoExecutionGuard { abort },
        })
    }

    /// Reload static credentials before every durable claim. The server
    /// process writes the file atomically; this host loads a complete snapshot
    /// off-thread, swaps it under one write lock, and refreshes catalogs only
    /// when credential content actually changed. No credential values or
    /// fingerprints are logged.
    async fn refresh_credentials_and_catalog_if_needed(&self, claimed_model: &str) -> Result<bool> {
        let _refresh_guard = self.credential_refresh.lock().await;
        let loaded = tokio::task::spawn_blocking(CredentialStore::load)
            .await
            .context("joining Mako credential snapshot reload")?
            .context("loading Mako credential snapshot")?;
        let changed = {
            let mut current = self.state.credential_store.write().await;
            replace_credential_snapshot_if_changed(&mut current, loaded.clone())
        };
        let claimed_model_unknown = self
            .state
            .model_registry
            .get_model(claimed_model)
            .await
            .is_none();
        if changed || claimed_model_unknown {
            initialize_models(&self.state.model_registry, self.state.db_path.as_path()).await;
            spawn_model_catalog_refresh(
                self.state.model_registry.clone(),
                self.state.credential_store.clone(),
                self.state.db_path.clone(),
            );
            tracing::info!(
                credentials_changed = changed,
                claimed_model_was_unknown = claimed_model_unknown,
                "Refreshed Mako credential/model execution snapshot"
            );
        }
        Ok(changed)
    }

    /// Deliver an already-governed input to the active orchestrator. The short
    /// registration wait closes the startup race between scheduler claim and
    /// the runner publishing its input channel.
    pub async fn send_input(&self, session_id: &str, input: LoopInput) -> Result<()> {
        self.send_input_to_execution(session_id, None, input).await
    }

    /// Deliver a control only to the exact durable run that owns it. This is
    /// required for approval outbox items: a late decision for run A must
    /// never be accepted by a replacement run B in the same session.
    pub async fn send_input_for_run(
        &self,
        session_id: &str,
        run_id: &str,
        input: LoopInput,
    ) -> Result<()> {
        self.send_input_to_execution(session_id, Some(run_id), input)
            .await
    }

    async fn send_input_to_execution(
        &self,
        session_id: &str,
        expected_run_id: Option<&str>,
        input: LoopInput,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.config.input_registration_timeout;
        loop {
            let execution = self.active_execution(session_id, expected_run_id).await?;
            if let Some(sender) = self
                .state
                .session_inputs
                .read()
                .await
                .get(session_id)
                .cloned()
            {
                // Revalidate after the channel becomes visible, not merely
                // before waiting for registration. This closes the lease-loss
                // window during host startup.
                self.validate_execution(&execution.spec).await?;
                let current = self.active_execution(session_id, expected_run_id).await?;
                return deliver_input_to_exact_execution(
                    &current.spec,
                    &execution.spec,
                    expected_run_id,
                    &sender,
                    input,
                );
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("Mako execution input channel was not registered in time");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn active_execution(
        &self,
        session_id: &str,
        expected_run_id: Option<&str>,
    ) -> Result<ActiveExecution> {
        let active = self.active.read().await;
        let execution = active
            .get(session_id)
            .cloned()
            .context("Mako session has no active execution")?;
        if expected_run_id.is_some_and(|run_id| run_id != execution.spec.run_id()) {
            bail!("Mako control targets a stale run");
        }
        Ok(execution)
    }

    async fn validate_execution(&self, spec: &MakoExecutionSpec) -> Result<()> {
        validate_execution_spec(self.state.db_path.as_ref().clone(), spec.clone()).await
    }

    /// Cooperatively cancel, then enforce a finite grace period. This is used
    /// both for user cancellation and scheduler fencing loss.
    pub async fn cancel(&self, session_id: &str, expected_run_id: Option<&str>) -> Result<()> {
        let execution = match self.active_execution(session_id, expected_run_id).await {
            Ok(execution) => execution,
            Err(_) if expected_run_id.is_some() => return Ok(()),
            Err(_) => return Ok(()),
        };
        let sender = self
            .state
            .session_inputs
            .read()
            .await
            .get(session_id)
            .cloned();
        if let Some(sender) = sender {
            let _ = sender.send(LoopInput::Cancel);
        }
        if !self
            .active
            .read()
            .await
            .get(session_id)
            .is_some_and(|entry| entry.spec.same_execution(&execution.spec))
        {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + self.config.cancel_grace;
        loop {
            if !self
                .active
                .read()
                .await
                .get(session_id)
                .is_some_and(|entry| entry.spec.same_execution(&execution.spec))
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                execution.abort.abort();
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn abort(&self, session_id: &str, expected_run_id: Option<&str>) {
        if let Ok(execution) = self.active_execution(session_id, expected_run_id).await {
            execution.abort.abort();
        }
    }
}

fn bounded_execution_error(mut error: String) -> String {
    if error.len() <= MAX_EXECUTION_ERROR_BYTES {
        return error;
    }
    let mut cutoff = MAX_EXECUTION_ERROR_BYTES.saturating_sub(" [truncated]".len());
    while cutoff > 0 && !error.is_char_boundary(cutoff) {
        cutoff -= 1;
    }
    error.truncate(cutoff);
    error.push_str(" [truncated]");
    error
}

fn replace_credential_snapshot_if_changed(
    current: &mut CredentialStore,
    loaded: CredentialStore,
) -> bool {
    if *current == loaded {
        return false;
    }
    *current = loaded;
    true
}

fn deliver_input_to_exact_execution(
    current: &MakoExecutionSpec,
    initially_observed: &MakoExecutionSpec,
    expected_run_id: Option<&str>,
    sender: &tokio::sync::mpsc::UnboundedSender<LoopInput>,
    input: LoopInput,
) -> Result<()> {
    if expected_run_id.is_some_and(|run_id| run_id != current.run_id()) {
        bail!("Mako control targets a stale run");
    }
    if !current.same_execution(initially_observed) {
        bail!("Mako execution changed while delivering input");
    }
    sender
        .send(input)
        .map_err(|_| anyhow::anyhow!("Mako execution no longer accepts input"))
}

pub(crate) async fn validate_execution_spec(
    database_path: PathBuf,
    spec: MakoExecutionSpec,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let store = MakoRunStore::new(Database::new(&database_path)?);
        anyhow::ensure!(
            store.validate_claimed_execution_fenced(&spec.claim, &spec.fence(), Utc::now())?,
            "Mako execution fence is no longer current"
        );
        Ok(())
    })
    .await
    .context("joining Mako execution fence validation")?
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use krusty_core::agent::LoopInput;
    use krusty_core::ai::providers::ProviderId;
    use krusty_core::ai::types::Content;
    use krusty_core::mako::MakoRunStatus;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::{ClaimedMakoRun, MakoRun, MakoRunKind};
    use krusty_core::tools::registry::PermissionMode;

    use super::{
        bounded_execution_error, deliver_input_to_exact_execution,
        replace_credential_snapshot_if_changed, MakoExecutionHostConfig, MakoExecutionSpec,
        DEFAULT_EVENT_CAPACITY, MAX_EVENT_CAPACITY, MAX_EXECUTION_ERROR_BYTES,
    };

    #[test]
    fn execution_host_buffers_and_errors_are_bounded() {
        let config = MakoExecutionHostConfig::new("runtime.db".into(), "/work".into());
        assert_eq!(config.event_capacity, DEFAULT_EVENT_CAPACITY);
        assert!(config.validate().is_ok());

        let mut oversized = config;
        oversized.event_capacity = MAX_EVENT_CAPACITY + 1;
        assert!(oversized.validate().is_err());

        let bounded = bounded_execution_error("error".repeat(MAX_EXECUTION_ERROR_BYTES));
        assert!(bounded.len() <= MAX_EXECUTION_ERROR_BYTES);
        assert!(bounded.ends_with(" [truncated]"));
    }

    fn execution_spec_for_test(run_id: &str, config: serde_json::Value) -> MakoExecutionSpec {
        let lease_token = format!("lease-{run_id}");
        let run = MakoRun {
            id: run_id.into(),
            controller_id: "controller-1".into(),
            session_id: Some("session-1".into()),
            schedule_id: None,
            occurrence_id: None,
            kind: MakoRunKind::Dispatch,
            objective: "ship it".into(),
            config,
            status: MakoRunStatus::Leased,
            priority: 0,
            concurrency_key: None,
            scheduled_for: None,
            available_at: Utc::now().to_rfc3339(),
            wake_at: None,
            attempt_count: 1,
            max_attempts: 5,
            lease_owner: Some("daemon:boot:one".into()),
            lease_token: Some(lease_token.clone()),
            lease_epoch: Some(9),
            lease_expires_at: Some(Utc::now().to_rfc3339()),
            heartbeat_at: Some(Utc::now().to_rfc3339()),
            last_stop_reason: None,
            last_error: None,
            outcome: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now().to_rfc3339(),
        };
        MakoExecutionSpec::from_claim(
            ClaimedMakoRun {
                run,
                attempt_id: format!("attempt-{run_id}"),
                attempt_no: 1,
                lease_token,
            },
            "daemon:boot:one".into(),
        )
        .expect("test execution spec should be valid")
    }

    #[test]
    fn claimed_config_is_frozen_into_execution_spec() {
        let run = MakoRun {
            id: "run-1".into(),
            controller_id: "controller-1".into(),
            session_id: Some("session-1".into()),
            schedule_id: None,
            occurrence_id: None,
            kind: MakoRunKind::Scheduled,
            objective: "ship it".into(),
            config: serde_json::json!({
                "working_dir": "/claimed/work",
                "project_dir": "/claimed/project",
                "model": "provider:claimed",
                "crew_slug": "release",
                "permission_mode": "supervised"
            }),
            status: MakoRunStatus::Leased,
            priority: 0,
            concurrency_key: None,
            scheduled_for: None,
            available_at: Utc::now().to_rfc3339(),
            wake_at: None,
            attempt_count: 1,
            max_attempts: 5,
            lease_owner: Some("daemon:boot:one".into()),
            lease_token: Some("lease-token".into()),
            lease_epoch: Some(9),
            lease_expires_at: Some(Utc::now().to_rfc3339()),
            heartbeat_at: Some(Utc::now().to_rfc3339()),
            last_stop_reason: None,
            last_error: None,
            outcome: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now().to_rfc3339(),
        };
        let claim = ClaimedMakoRun {
            run,
            attempt_id: "attempt-1".into(),
            attempt_no: 1,
            lease_token: "lease-token".into(),
        };
        let spec = MakoExecutionSpec::from_claim(claim, "daemon:boot:one".into()).unwrap();
        assert_eq!(
            spec.working_dir.as_deref(),
            Some(std::path::Path::new("/claimed/work"))
        );
        assert_eq!(
            spec.project_dir.as_deref(),
            Some(std::path::Path::new("/claimed/project"))
        );
        assert_eq!(spec.model, "provider:claimed");
        assert_eq!(spec.crew_slug.as_deref(), Some("release"));
        assert_eq!(spec.permission_mode, PermissionMode::Supervised);
    }

    #[test]
    fn claimed_execution_without_workspace_fails_closed() {
        let mut run = MakoRun {
            id: "run-1".into(),
            controller_id: "controller-1".into(),
            session_id: Some("session-1".into()),
            schedule_id: None,
            occurrence_id: None,
            kind: MakoRunKind::Scheduled,
            objective: "ship it".into(),
            config: serde_json::json!({
                "model": "provider:claimed",
                "permission_mode": "autonomous"
            }),
            status: MakoRunStatus::Leased,
            priority: 0,
            concurrency_key: None,
            scheduled_for: None,
            available_at: Utc::now().to_rfc3339(),
            wake_at: None,
            attempt_count: 1,
            max_attempts: 5,
            lease_owner: Some("daemon:boot:one".into()),
            lease_token: Some("lease-token".into()),
            lease_epoch: Some(9),
            lease_expires_at: Some(Utc::now().to_rfc3339()),
            heartbeat_at: Some(Utc::now().to_rfc3339()),
            last_stop_reason: None,
            last_error: None,
            outcome: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now().to_rfc3339(),
        };
        let claim = ClaimedMakoRun {
            run: run.clone(),
            attempt_id: "attempt-1".into(),
            attempt_no: 1,
            lease_token: "lease-token".into(),
        };
        let error = MakoExecutionSpec::from_claim(claim, "daemon:boot:one".into())
            .expect_err("missing claimed workspace must be rejected");
        assert!(error.to_string().contains("no explicit working_dir"));

        run.config = serde_json::json!({
            "project_dir": "/claimed/project",
            "model": "provider:claimed",
            "permission_mode": "autonomous"
        });
        let claim = ClaimedMakoRun {
            run,
            attempt_id: "attempt-2".into(),
            attempt_no: 1,
            lease_token: "lease-token".into(),
        };
        assert!(MakoExecutionSpec::from_claim(claim, "daemon:boot:one".into()).is_ok());
    }

    #[test]
    fn claimed_execution_without_model_fails_closed() {
        let run = MakoRun {
            id: "run-no-model".into(),
            controller_id: "controller-1".into(),
            session_id: Some("session-1".into()),
            schedule_id: None,
            occurrence_id: None,
            kind: MakoRunKind::Dispatch,
            objective: "ship it".into(),
            config: serde_json::json!({
                "working_dir": "/claimed/work",
                "permission_mode": "autonomous"
            }),
            status: MakoRunStatus::Leased,
            priority: 0,
            concurrency_key: None,
            scheduled_for: None,
            available_at: Utc::now().to_rfc3339(),
            wake_at: None,
            attempt_count: 1,
            max_attempts: 5,
            lease_owner: Some("daemon:boot:one".into()),
            lease_token: Some("lease-token".into()),
            lease_epoch: Some(9),
            lease_expires_at: Some(Utc::now().to_rfc3339()),
            heartbeat_at: Some(Utc::now().to_rfc3339()),
            last_stop_reason: None,
            last_error: None,
            outcome: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now().to_rfc3339(),
        };
        let claim = ClaimedMakoRun {
            run,
            attempt_id: "attempt-1".into(),
            attempt_no: 1,
            lease_token: "lease-token".into(),
        };
        let error = MakoExecutionSpec::from_claim(claim, "daemon:boot:one".into())
            .expect_err("missing claimed model must be rejected");
        assert!(error.to_string().contains("no explicit model"));
    }

    #[test]
    fn claimed_execution_without_permission_mode_fails_closed() {
        let mut claim = execution_spec_for_test(
            "run-with-permission",
            serde_json::json!({
                "working_dir": "/claimed/work",
                "model": "provider:claimed",
                "permission_mode": "supervised"
            }),
        )
        .claim;
        claim
            .run
            .config
            .as_object_mut()
            .unwrap()
            .remove("permission_mode");
        let error = MakoExecutionSpec::from_claim(claim, "daemon:boot:one".into())
            .expect_err("missing claimed permission mode must be rejected");
        assert!(error.to_string().contains("no explicit permission_mode"));
    }

    #[test]
    fn every_present_claimed_workspace_path_must_be_absolute() {
        let run = MakoRun {
            id: "run-relative-project".into(),
            controller_id: "controller-1".into(),
            session_id: Some("session-1".into()),
            schedule_id: None,
            occurrence_id: None,
            kind: MakoRunKind::Dispatch,
            objective: "ship it".into(),
            config: serde_json::json!({
                "working_dir": "/claimed/work",
                "project_dir": "relative/project",
                "model": "provider:claimed",
                "permission_mode": "autonomous"
            }),
            status: MakoRunStatus::Leased,
            priority: 0,
            concurrency_key: None,
            scheduled_for: None,
            available_at: Utc::now().to_rfc3339(),
            wake_at: None,
            attempt_count: 1,
            max_attempts: 5,
            lease_owner: Some("daemon:boot:one".into()),
            lease_token: Some("lease-token".into()),
            lease_epoch: Some(9),
            lease_expires_at: Some(Utc::now().to_rfc3339()),
            heartbeat_at: Some(Utc::now().to_rfc3339()),
            last_stop_reason: None,
            last_error: None,
            outcome: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now().to_rfc3339(),
        };
        let claim = ClaimedMakoRun {
            run,
            attempt_id: "attempt-1".into(),
            attempt_no: 1,
            lease_token: "lease-token".into(),
        };
        let error = MakoExecutionSpec::from_claim(claim, "daemon:boot:one".into())
            .expect_err("relative project_dir must be rejected even with absolute working_dir");
        assert!(error
            .to_string()
            .contains("every claimed Mako workspace path"));
    }

    #[test]
    fn credential_snapshot_is_replaced_only_when_content_changes() {
        let mut current = CredentialStore::default();
        current.set(ProviderId::OpenAI, "first-value".into());

        let unchanged = current.clone();
        assert!(!replace_credential_snapshot_if_changed(
            &mut current,
            unchanged
        ));

        let mut rotated = current.clone();
        rotated.set(ProviderId::OpenAI, "rotated-value".into());
        assert!(replace_credential_snapshot_if_changed(
            &mut current,
            rotated
        ));
        assert_eq!(
            current.get(&ProviderId::OpenAI).map(String::as_str),
            Some("rotated-value")
        );
    }

    #[test]
    fn stale_exact_run_input_cannot_reach_replacement_execution() {
        let replacement = execution_spec_for_test(
            "run-b",
            serde_json::json!({
                "working_dir": "/claimed/work",
                "model": "provider:claimed",
                "permission_mode": "supervised"
            }),
        );
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let error = deliver_input_to_exact_execution(
            &replacement,
            &replacement,
            Some("run-a"),
            &sender,
            LoopInput::Steer {
                pending_id: Some("pending-a".into()),
                content: vec![Content::Text {
                    text: "Response to question-a:\nyes".into(),
                }],
            },
        )
        .expect_err("run A response must not be delivered to replacement run B");
        assert!(error.to_string().contains("stale run"));
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
}
