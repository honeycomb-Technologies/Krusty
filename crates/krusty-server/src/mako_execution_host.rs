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
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::AbortHandle;

use krusty_core::agent::LoopInput;

use crate::mako_runtime::runner::{run_mako_session_inner, MakoExecutionEventSink};
use crate::types::AgenticEvent;
use crate::{build_app_state, AppState, MakoRuntimeMode, ServerConfig};

const DEFAULT_EVENT_CAPACITY: usize = 256;
const DEFAULT_INPUT_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_CANCEL_GRACE: Duration = Duration::from_secs(5);

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
}

#[derive(Clone)]
struct ActiveExecution {
    run_id: String,
    abort: AbortHandle,
}

pub struct MakoExecutionHost {
    state: AppState,
    active: RwLock<HashMap<String, ActiveExecution>>,
    config: MakoExecutionHostConfig,
}

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
        if config.event_capacity == 0 {
            bail!("Mako execution event capacity must be greater than zero");
        }
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
            config,
        }))
    }

    pub async fn start(
        self: &Arc<Self>,
        session_id: String,
        run_id: String,
        wake_reason: String,
    ) -> Result<MakoExecutionRun> {
        let mut active = self.active.write().await;
        if let Some(existing) = active.get(&session_id) {
            bail!(
                "Mako session {session_id} is already executing run {}",
                existing.run_id
            );
        }

        let (event_tx, event_rx) = mpsc::channel(self.config.event_capacity);
        let sink = MakoExecutionEventSink::Bounded(event_tx);
        let state = self.state.clone();
        let manager = state.mako_runtime.clone();
        let session_for_task = session_id.clone();
        let run_for_task = run_id.clone();
        let error_sink = sink.clone();
        let runner = tokio::spawn(async move {
            let result = run_mako_session_inner(
                state,
                session_for_task,
                run_for_task,
                wake_reason,
                sink,
                manager,
                false,
            )
            .await;
            if let Err(error) = &result {
                let _ = error_sink
                    .send(AgenticEvent::Error {
                        error: error.to_string(),
                    })
                    .await;
            }
            result.map_err(|error| error.to_string())
        });
        let abort = runner.abort_handle();
        active.insert(
            session_id.clone(),
            ActiveExecution {
                run_id: run_id.clone(),
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
                Err(error) => Err(format!("Mako execution task failed: {error}")),
            };
            if let Some(host) = weak_host.upgrade() {
                let mut active = host.active.write().await;
                if active
                    .get(&session_id)
                    .is_some_and(|entry| entry.run_id == run_id)
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

    /// Deliver an already-governed input to the active orchestrator. The short
    /// registration wait closes the startup race between scheduler claim and
    /// the runner publishing its input channel.
    pub async fn send_input(&self, session_id: &str, input: LoopInput) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.config.input_registration_timeout;
        loop {
            if !self.active.read().await.contains_key(session_id) {
                bail!("Mako session has no active execution");
            }
            if let Some(sender) = self.state.session_inputs.read().await.get(session_id).cloned() {
                return sender
                    .send(input)
                    .map_err(|_| anyhow::anyhow!("Mako execution no longer accepts input"));
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("Mako execution input channel was not registered in time");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Cooperatively cancel, then enforce a finite grace period. This is used
    /// both for user cancellation and scheduler fencing loss.
    pub async fn cancel(&self, session_id: &str) -> Result<()> {
        if !self.active.read().await.contains_key(session_id) {
            return Ok(());
        }
        let _ = self.send_input(session_id, LoopInput::Cancel).await;
        let deadline = tokio::time::Instant::now() + self.config.cancel_grace;
        loop {
            if !self.active.read().await.contains_key(session_id) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let execution = self.active.read().await.get(session_id).cloned();
                if let Some(execution) = execution {
                    execution.abort.abort();
                }
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn abort(&self, session_id: &str) {
        if let Some(execution) = self.active.read().await.get(session_id).cloned() {
            execution.abort.abort();
        }
    }
}
