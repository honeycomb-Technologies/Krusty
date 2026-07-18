//! Sub-agent system for parallel task execution
//!
//! Enables spawning lightweight agents to explore the codebase.
//! Sub-agents have read-only access: glob, grep, read.
//! They cannot modify files or execute arbitrary commands.
//!
//! ## Provider-Agnostic Design
//! Sub-agents use the user's current model by default. Set override_model
//! when creating SubAgentPool to use the same model as the main agent.
//!
//! ## Module Structure
//! - `build_context`: Shared builder coordination context
//! - `types`: Core data types (progress, models, tasks, results)
//! - `tools`: Tool implementations for explorers and builders
//! - `execution`: Agent loop and API communication

pub mod build_context;
mod execution;
mod identity;
mod scheduler;
mod tools;
mod types;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{info, warn};

/// Default stagger delay between spawning agents (prevents rate limit storms)
/// Same for all providers - users can override with with_stagger_delay() if needed
const DEFAULT_STAGGER_MS: u64 = 100;

use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;

use self::build_context::SharedBuildContext;

// Re-export public types
pub use identity::AgentIdentity;
pub use scheduler::{
    AdaptiveConcurrencyPolicy, AgentScheduler, BackpressureSignal, ScheduleRequest,
    SchedulerSnapshot, SchedulingClass,
};
pub use tools::BuilderTools;
pub use types::{
    AgentProgress, AgentProgressStatus, DelegatedProcessArtifact, SubAgentApiError, SubAgentResult,
    SubAgentTask,
};

// Re-export single agent entry points
pub use execution::execute_single_explorer;
pub(crate) use execution::{execute_single_agent, AgentConfig, SingleExplorerConfig};

// Internal execution functions
use execution::execute_builder_with_progress;

/// Pool for managing concurrent sub-agent execution (used by the Build tool)
pub struct SubAgentPool {
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
    /// Optional user ceiling. The default scheduler limit is derived from host
    /// capacity and adapts to observed provider health.
    concurrency_ceiling: Option<usize>,
    /// Override model for non-Anthropic providers (uses user's selected model)
    override_model: Option<String>,
    /// Delay between spawning agents (prevents rate limit storms)
    stagger_delay: Duration,
}

impl SubAgentPool {
    pub fn new(client: Arc<AiClient>, cancellation: AgentCancellation) -> Self {
        Self {
            client,
            cancellation,
            concurrency_ceiling: None,
            override_model: None,
            stagger_delay: Duration::from_millis(DEFAULT_STAGGER_MS),
        }
    }

    pub fn with_concurrency(mut self, max: usize) -> Self {
        self.concurrency_ceiling = Some(max.max(1));
        self
    }

    /// Set the model for sub-agent tasks
    ///
    /// This should be set to the user's current model for provider-agnostic behavior.
    /// If not set, falls back to the client's configured model.
    pub fn with_override_model(mut self, model: Option<String>) -> Self {
        self.override_model = model;
        self
    }

    /// Set custom stagger delay between agent spawns
    pub fn with_stagger_delay(mut self, delay: Duration) -> Self {
        self.stagger_delay = delay;
        self
    }

    /// Get the model to use for sub-agent tasks
    ///
    /// Returns the override_model (user's current model). This must be set
    /// when creating the SubAgentPool via `with_override_model()`.
    /// Falls back to the client's configured model if not set.
    fn resolve_model(&self) -> String {
        self.override_model
            .clone()
            .unwrap_or_else(|| self.client.config().model.clone())
    }

    fn scheduler(&self) -> AgentScheduler {
        let policy = self
            .concurrency_ceiling
            .map_or_else(AdaptiveConcurrencyPolicy::default, |ceiling| {
                AdaptiveConcurrencyPolicy::default().with_ceiling(ceiling)
            });
        AgentScheduler::new(policy)
    }

    /// Execute exploration tasks concurrently using the single-agent model
    ///
    /// Each task runs as an independent single explorer agent with read-only tools.
    pub async fn execute_with_progress(
        &self,
        tasks: Vec<SubAgentTask>,
        progress_tx: mpsc::UnboundedSender<AgentProgress>,
    ) -> Vec<SubAgentResult> {
        use crate::tools::registry::{DelegationPolicy, PermissionMode, ToolRegistry};

        let registry = Arc::new(ToolRegistry::new());
        let policy = DelegationPolicy::for_subagent_explore(PermissionMode::Autonomous, Some(20));
        let task_count = tasks.len();
        let stagger = self.stagger_delay;

        let scheduler = self.scheduler();
        info!(
            count = task_count,
            concurrency_ceiling = ?self.concurrency_ceiling,
            stagger_ms = stagger.as_millis() as u64,
            "SubAgentPool: Spawning explorer agents with stagger"
        );

        // JoinSet aborts every still-running child when this foreground pool
        // future is dropped (for example by LoopInput::Cancel). Plain detached
        // JoinHandles would allow delegated work to keep mutating afterward.
        let mut task_set = JoinSet::new();

        for (idx, mut task) in tasks.into_iter().enumerate() {
            if idx > 0 && !stagger.is_zero() {
                sleep(stagger).await;
            }

            task.ensure_identity("/root", "explorer", idx);
            let scheduler = scheduler.clone();
            let client = self.client.clone();
            let cancel = self.cancellation.child_token();
            let resolved_model = self.resolve_model();
            let registry = registry.clone();
            let policy = policy.clone();
            let task_id = task.id.clone();
            let progress_tx = progress_tx.clone();

            task_set.spawn(async move {
                let request = ScheduleRequest::new(
                    task.parent_session_id
                        .clone()
                        .or_else(|| task.delegated_run_id.clone())
                        .unwrap_or_else(|| task_id.clone()),
                    resolved_model.clone(),
                    SchedulingClass::ReadOnly,
                );
                let Some(permit) = scheduler.acquire(request, &cancel).await else {
                    return (idx, cancelled_result(&task));
                };

                if cancel.is_cancelled() {
                    return (idx, cancelled_result(&task));
                }

                let result = execute_single_explorer(
                    client,
                    task,
                    registry,
                    policy,
                    String::new(),
                    resolved_model,
                    cancel,
                    Some(progress_tx),
                )
                .await;
                permit.complete(BackpressureSignal::from_error(result.error.as_deref()));
                (idx, result)
            });
        }

        let mut indexed_results = Vec::with_capacity(task_count);
        let mut join_failures = Vec::new();
        while let Some(joined) = task_set.join_next().await {
            match joined {
                Ok(result) => indexed_results.push(result),
                Err(e) => {
                    warn!("Explorer task panicked: {}", e);
                    join_failures.push(SubAgentResult {
                        task_id: "unknown".to_string(),
                        agent_name: "unknown".to_string(),
                        delegated_run_id: None,
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some(format!("Task panicked: {}", e)),
                        policy_violations: vec![],
                        background_processes: vec![],
                    });
                }
            }
        }
        indexed_results.sort_by_key(|(index, _)| *index);
        let mut results = indexed_results
            .into_iter()
            .map(|(_, result)| result)
            .collect::<Vec<_>>();
        results.extend(join_failures);

        info!(
            "SubAgentPool: All explorers complete, {} results",
            results.len()
        );
        results
    }

    /// Execute builder tasks with write access, shared context, and staggered spawning
    pub async fn execute_builders(
        &self,
        tasks: Vec<SubAgentTask>,
        context: Arc<SharedBuildContext>,
        progress_tx: mpsc::UnboundedSender<AgentProgress>,
    ) -> Vec<SubAgentResult> {
        let scheduler = self.scheduler();
        let client = self.client.clone();
        let cancellation = self.cancellation.clone();
        let task_count = tasks.len();
        let stagger = self.stagger_delay;

        info!(
            count = task_count,
            concurrency_ceiling = ?self.concurrency_ceiling,
            stagger_ms = stagger.as_millis() as u64,
            "SubAgentPool: Spawning builder agents with stagger"
        );

        // Per-invocation ownership is the cancellation boundary: dropping this
        // foreground pool aborts its builders without touching another session's
        // independently-owned JoinSet. Background pools keep owning their set.
        let mut task_set = JoinSet::new();

        for (idx, mut task) in tasks.into_iter().enumerate() {
            // Stagger delay between spawns (skip first)
            if idx > 0 && !stagger.is_zero() {
                sleep(stagger).await;
            }

            task.ensure_identity("/root", "builder", idx);
            let scheduler = scheduler.clone();
            let client = client.clone();
            let cancel = cancellation.child_token();
            let context = context.clone();
            let task_id = task.id.clone();
            let progress_tx = progress_tx.clone();
            let resolved_model = self.resolve_model();

            task_set.spawn(async move {
                let request = ScheduleRequest::new(
                    task.parent_session_id
                        .clone()
                        .or_else(|| task.delegated_run_id.clone())
                        .unwrap_or_else(|| task_id.clone()),
                    format!("{}:{}", resolved_model, task_id),
                    SchedulingClass::WriteShared,
                );
                let Some(permit) = scheduler.acquire(request, &cancel).await else {
                    return (idx, cancelled_result(&task));
                };

                if cancel.is_cancelled() {
                    return (idx, cancelled_result(&task));
                }

                let result = execute_builder_with_progress(
                    &client,
                    task,
                    &resolved_model,
                    cancel,
                    context,
                    progress_tx,
                )
                .await;
                permit.complete(BackpressureSignal::from_error(result.error.as_deref()));
                (idx, result)
            });
        }

        // Collect results in input order even though JoinSet completes them in
        // readiness order, preserving the existing public result contract.
        let mut indexed_results = Vec::with_capacity(task_count);
        let mut join_failures = Vec::new();
        while let Some(joined) = task_set.join_next().await {
            match joined {
                Ok(result) => indexed_results.push(result),
                Err(e) => {
                    warn!("Builder task panicked: {}", e);
                    join_failures.push(SubAgentResult {
                        task_id: "unknown".to_string(),
                        agent_name: "unknown".to_string(),
                        delegated_run_id: None,
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some(format!("Task panicked: {}", e)),
                        policy_violations: vec![],
                        background_processes: vec![],
                    });
                }
            }
        }
        indexed_results.sort_by_key(|(index, _)| *index);
        let mut results = indexed_results
            .into_iter()
            .map(|(_, result)| result)
            .collect::<Vec<_>>();
        results.extend(join_failures);

        let stats = context.stats();
        info!("SubAgentPool: Builders complete | {}", stats);
        results
    }
}

fn cancelled_result(task: &SubAgentTask) -> SubAgentResult {
    SubAgentResult {
        task_id: task.id.clone(),
        agent_name: task.name.clone(),
        delegated_run_id: task.delegated_run_id.clone(),
        success: false,
        output: String::new(),
        files_examined: vec![],
        duration_ms: 0,
        turns_used: 0,
        error: Some("Cancelled".to_string()),
        policy_violations: vec![],
        background_processes: vec![],
    }
}

#[cfg(test)]
mod cancellation_tests {
    use std::time::Duration;

    use tokio::task::JoinSet;

    #[tokio::test]
    async fn dropping_foreground_task_set_stops_delayed_write_without_affecting_other_run() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let cancelled_write = temp_dir.path().join("cancelled-session.txt");
        let unaffected_write = temp_dir.path().join("other-session.txt");

        let mut cancelled_session = JoinSet::new();
        cancelled_session.spawn({
            let path = cancelled_write.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(80)).await;
                tokio::fs::write(path, "late mutation")
                    .await
                    .expect("delayed write");
            }
        });

        let mut other_session = JoinSet::new();
        other_session.spawn({
            let path = unaffected_write.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                tokio::fs::write(path, "completed")
                    .await
                    .expect("unaffected write");
            }
        });

        // This is the ownership transition performed when the foreground agent
        // tool future is dropped after LoopInput::Cancel.
        drop(cancelled_session);

        tokio::time::timeout(Duration::from_secs(1), other_session.join_next())
            .await
            .expect("other session should not be cancelled")
            .expect("other session task should complete")
            .expect("other session task should succeed");
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(!cancelled_write.exists());
        assert_eq!(
            std::fs::read_to_string(unaffected_write).expect("other session output"),
            "completed"
        );
    }
}
