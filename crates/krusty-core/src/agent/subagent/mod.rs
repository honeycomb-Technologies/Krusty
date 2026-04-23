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
mod tools;
mod types;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use tokio::time::{sleep, timeout};
use tracing::{info, warn};

/// Timeout for acquiring semaphore permit (prevents deadlock on hung agents)
const SEMAPHORE_TIMEOUT: Duration = Duration::from_secs(300);

/// Default stagger delay between spawning agents (prevents rate limit storms)
/// Same for all providers - users can override with with_stagger_delay() if needed
const DEFAULT_STAGGER_MS: u64 = 100;

use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;

use self::build_context::SharedBuildContext;

// Re-export public types
pub use tools::BuilderTools;
pub use types::{
    AgentProgress, AgentProgressStatus, SubAgentApiError, SubAgentResult, SubAgentTask,
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
    max_concurrency: usize,
    /// Override model for non-Anthropic providers (uses user's selected model)
    override_model: Option<String>,
    /// Delay between spawning agents (prevents rate limit storms)
    stagger_delay: Duration,
}

impl SubAgentPool {
    pub fn new(client: Arc<AiClient>, cancellation: AgentCancellation) -> Self {
        use crate::agent::constants::concurrency;

        Self {
            client,
            cancellation,
            max_concurrency: concurrency::MAX_PARALLEL_TOOLS,
            override_model: None,
            stagger_delay: Duration::from_millis(DEFAULT_STAGGER_MS),
        }
    }

    pub fn with_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max;
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

        info!(
            count = task_count,
            concurrency = self.max_concurrency,
            stagger_ms = stagger.as_millis() as u64,
            "SubAgentPool: Spawning explorer agents with stagger"
        );

        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let mut handles = Vec::with_capacity(task_count);

        for (idx, task) in tasks.into_iter().enumerate() {
            if idx > 0 && !stagger.is_zero() {
                sleep(stagger).await;
            }

            let sem = semaphore.clone();
            let client = self.client.clone();
            let cancel = self.cancellation.child_token();
            let resolved_model = self.resolve_model();
            let registry = registry.clone();
            let policy = policy.clone();
            let task_id = task.id.clone();
            let progress_tx = progress_tx.clone();

            let handle = tokio::spawn(async move {
                let _permit = match timeout(SEMAPHORE_TIMEOUT, sem.acquire()).await {
                    Ok(Ok(p)) => p,
                    Ok(Err(e)) => {
                        warn!(task_id = %task_id, error = %e, "Explorer: Failed to acquire semaphore");
                        return SubAgentResult {
                            task_id,
                            agent_name: task.name.clone(),
                            delegated_run_id: task.delegated_run_id.clone(),
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some(format!("Semaphore error: {}", e)),
                            policy_violations: vec![],
                        };
                    }
                    Err(_) => {
                        warn!(task_id = %task_id, "Explorer: Semaphore acquire timed out");
                        return SubAgentResult {
                            task_id,
                            agent_name: task.name.clone(),
                            delegated_run_id: task.delegated_run_id.clone(),
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some(format!(
                                "Semaphore acquire timed out after {:?}",
                                SEMAPHORE_TIMEOUT
                            )),
                            policy_violations: vec![],
                        };
                    }
                };

                if cancel.is_cancelled() {
                    return SubAgentResult {
                        task_id,
                        agent_name: task.name.clone(),
                        delegated_run_id: task.delegated_run_id.clone(),
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some("Cancelled".to_string()),
                        policy_violations: vec![],
                    };
                }

                execute_single_explorer(
                    client,
                    task,
                    registry,
                    policy,
                    String::new(),
                    resolved_model,
                    cancel,
                    Some(progress_tx),
                )
                .await
            });

            handles.push(handle);
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!("Explorer task panicked: {}", e);
                    results.push(SubAgentResult {
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
                    });
                }
            }
        }

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
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let client = self.client.clone();
        let cancellation = self.cancellation.clone();
        let task_count = tasks.len();
        let stagger = self.stagger_delay;

        info!(
            count = task_count,
            concurrency = self.max_concurrency,
            stagger_ms = stagger.as_millis() as u64,
            "SubAgentPool: Spawning builder agents with stagger"
        );

        // Spawn tasks with staggered delays
        let mut handles = Vec::with_capacity(task_count);

        for (idx, task) in tasks.into_iter().enumerate() {
            // Stagger delay between spawns (skip first)
            if idx > 0 && !stagger.is_zero() {
                sleep(stagger).await;
            }

            let sem = semaphore.clone();
            let client = client.clone();
            let cancel = cancellation.child_token();
            let context = context.clone();
            let task_id = task.id.clone();
            let progress_tx = progress_tx.clone();
            let resolved_model = self.resolve_model();

            let handle = tokio::spawn(async move {
                let _permit = match timeout(SEMAPHORE_TIMEOUT, sem.acquire()).await {
                    Ok(Ok(p)) => p,
                    Ok(Err(e)) => {
                        warn!(task_id = %task_id, error = %e, "Builder: Failed to acquire semaphore");
                        return SubAgentResult {
                            task_id,
                            agent_name: task.name.clone(),
                            delegated_run_id: task.delegated_run_id.clone(),
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some(format!("Semaphore error: {}", e)),
                            policy_violations: vec![],
                        };
                    }
                    Err(_) => {
                        warn!(task_id = %task_id, "Builder: Semaphore acquire timed out after {:?}", SEMAPHORE_TIMEOUT);
                        return SubAgentResult {
                            task_id,
                            agent_name: task.name.clone(),
                            delegated_run_id: task.delegated_run_id.clone(),
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some(format!(
                                "Semaphore acquire timed out after {:?}",
                                SEMAPHORE_TIMEOUT
                            )),
                            policy_violations: vec![],
                        };
                    }
                };

                if cancel.is_cancelled() {
                    return SubAgentResult {
                        task_id,
                        agent_name: task.name.clone(),
                        delegated_run_id: task.delegated_run_id.clone(),
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some("Cancelled".to_string()),
                        policy_violations: vec![],
                    };
                }

                execute_builder_with_progress(
                    &client,
                    task,
                    &resolved_model,
                    cancel,
                    context,
                    progress_tx,
                )
                .await
            });

            handles.push(handle);
        }

        // Collect results
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!("Builder task panicked: {}", e);
                    results.push(SubAgentResult {
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
                    });
                }
            }
        }

        let stats = context.stats();
        info!("SubAgentPool: Builders complete | {}", stats);
        results
    }
}
