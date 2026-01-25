//! Sub-agent system for parallel task execution
//!
//! Enables spawning lightweight agents (e.g., Haiku) to explore the codebase.
//! Sub-agents have read-only access: glob, grep, read.
//! They cannot modify files or execute arbitrary commands.
//!
//! ## Module Structure
//! - `types`: Core data types (progress, models, tasks, results)
//! - `tools`: Tool implementations for explorers and builders
//! - `execution`: Agent loop and API communication

mod execution;
mod tools;
mod types;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::agent::build_context::SharedBuildContext;
use crate::agent::cache::SharedExploreCache;
use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::ai::providers::ProviderId;

// Re-export public types
pub use tools::BuilderTools;
pub use types::{
    AgentProgress, AgentProgressStatus, SubAgentApiError, SubAgentModel, SubAgentResult,
    SubAgentTask,
};

// Internal execution functions
use execution::{
    execute_builder_with_progress, execute_subagent_with_progress, execute_subagent_with_tools,
};

/// Pool for managing concurrent sub-agent execution
pub struct SubAgentPool {
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
    max_concurrency: usize,
    cache: Arc<SharedExploreCache>,
    /// Override model for non-Anthropic providers (uses user's selected model)
    override_model: Option<String>,
    /// Delay between spawning agents (prevents rate limit storms)
    stagger_delay: Duration,
}

impl SubAgentPool {
    pub fn new(client: Arc<AiClient>, cancellation: AgentCancellation) -> Self {
        use crate::agent::constants::concurrency;

        // Provider-specific stagger delays to respect rate limits
        // Anthropic has high limits, others need more spacing
        let stagger_delay = match client.provider_id() {
            ProviderId::Anthropic => Duration::from_millis(50), // High limits
            ProviderId::OpenRouter => Duration::from_millis(100), // Reasonable limits
            _ => Duration::from_millis(200), // Conservative for Z.ai, MiniMax, Kimi, etc.
        };

        Self {
            client,
            cancellation,
            max_concurrency: concurrency::MAX_PARALLEL_TOOLS,
            cache: Arc::new(SharedExploreCache::new()),
            override_model: None,
            stagger_delay,
        }
    }

    pub fn with_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max;
        self
    }

    /// Set an override model for non-Anthropic providers
    /// When set and provider isn't Anthropic, subagents use this model instead of SubAgentModel
    pub fn with_override_model(mut self, model: Option<String>) -> Self {
        self.override_model = model;
        self
    }

    /// Set custom stagger delay between agent spawns
    pub fn with_stagger_delay(mut self, delay: Duration) -> Self {
        self.stagger_delay = delay;
        self
    }

    /// Resolve which model to use for a task
    /// - Anthropic provider: use the task's specialized model (Haiku for explore, Opus for build)
    /// - Other providers: use the override model (user's selected model)
    fn resolve_model(&self, task: &SubAgentTask) -> String {
        if self.client.provider_id() == ProviderId::Anthropic {
            // Anthropic: use specialized subagent models
            task.model.model_id().to_string()
        } else {
            // Non-Anthropic: use the user's selected model (or fall back to task model)
            self.override_model
                .clone()
                .unwrap_or_else(|| task.model.model_id().to_string())
        }
    }

    /// Execute multiple sub-agent tasks concurrently with staggered spawning
    ///
    /// Agents are spawned with small delays between them to avoid rate limit storms.
    /// The stagger delay is provider-specific (lower for Anthropic, higher for others).
    pub async fn execute(&self, tasks: Vec<SubAgentTask>) -> Vec<SubAgentResult> {
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let client = self.client.clone();
        let cancellation = self.cancellation.clone();
        let cache = self.cache.clone();
        let task_count = tasks.len();
        let stagger = self.stagger_delay;

        info!(
            count = task_count,
            concurrency = self.max_concurrency,
            stagger_ms = stagger.as_millis() as u64,
            "SubAgentPool: Spawning sub-agents with stagger"
        );

        // Spawn tasks with staggered delays to avoid rate limit storms
        let mut handles = Vec::with_capacity(task_count);

        for (idx, task) in tasks.into_iter().enumerate() {
            // Stagger delay between spawns (skip first)
            if idx > 0 && !stagger.is_zero() {
                sleep(stagger).await;
            }

            let sem = semaphore.clone();
            let client = client.clone();
            let cancel = cancellation.child_token();
            let cache = cache.clone();
            let task_id = task.id.clone();
            let resolved_model = self.resolve_model(&task);

            let handle = tokio::spawn(async move {
                debug!(task_id = %task_id, "SubAgent: Acquiring semaphore permit");
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(task_id = %task_id, error = %e, "SubAgent: Failed to acquire semaphore");
                        return SubAgentResult {
                            task_id,
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some(format!("Semaphore error: {}", e)),
                        };
                    }
                };
                debug!(task_id = %task_id, "SubAgent: Got permit, checking cancellation");

                if cancel.is_cancelled() {
                    info!(task_id = %task_id, "SubAgent: Cancelled before execution");
                    return SubAgentResult {
                        task_id,
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some("Cancelled".to_string()),
                    };
                }

                info!(task_id = %task_id, model = %resolved_model, "SubAgent: Starting execution");
                let result =
                    execute_subagent_with_tools(&client, task, &resolved_model, cancel, cache)
                        .await;
                info!(task_id = %result.task_id, success = result.success, "SubAgent: Execution complete");
                result
            });

            handles.push(handle);
        }

        info!("SubAgentPool: Waiting for {} spawned tasks", handles.len());

        // Collect results from all spawned tasks
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!("SubAgent task panicked: {}", e);
                    results.push(SubAgentResult {
                        task_id: "unknown".to_string(),
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some(format!("Task panicked: {}", e)),
                    });
                }
            }
        }

        let stats = cache.stats();
        info!(
            "SubAgentPool: All futures complete, {} results | {}",
            results.len(),
            stats
        );
        results
    }

    /// Execute with real-time progress updates and staggered spawning
    pub async fn execute_with_progress(
        &self,
        tasks: Vec<SubAgentTask>,
        progress_tx: mpsc::UnboundedSender<AgentProgress>,
    ) -> Vec<SubAgentResult> {
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let client = self.client.clone();
        let cancellation = self.cancellation.clone();
        let cache = self.cache.clone();
        let task_count = tasks.len();
        let stagger = self.stagger_delay;

        info!(
            count = task_count,
            concurrency = self.max_concurrency,
            stagger_ms = stagger.as_millis() as u64,
            "SubAgentPool: Spawning sub-agents with progress and stagger"
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
            let cache = cache.clone();
            let task_id = task.id.clone();
            let progress_tx = progress_tx.clone();
            let resolved_model = self.resolve_model(&task);

            let handle = tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(task_id = %task_id, error = %e, "SubAgent: Failed to acquire semaphore");
                        return SubAgentResult {
                            task_id,
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some(format!("Semaphore error: {}", e)),
                        };
                    }
                };

                if cancel.is_cancelled() {
                    return SubAgentResult {
                        task_id,
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some("Cancelled".to_string()),
                    };
                }

                execute_subagent_with_progress(
                    &client,
                    task,
                    &resolved_model,
                    cancel,
                    cache,
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
                    warn!("SubAgent task panicked: {}", e);
                    results.push(SubAgentResult {
                        task_id: "unknown".to_string(),
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some(format!("Task panicked: {}", e)),
                    });
                }
            }
        }

        let stats = cache.stats();
        info!("SubAgentPool: Complete | {}", stats);
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
            let resolved_model = self.resolve_model(&task);

            let handle = tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(task_id = %task_id, error = %e, "Builder: Failed to acquire semaphore");
                        return SubAgentResult {
                            task_id,
                            success: false,
                            output: String::new(),
                            files_examined: vec![],
                            duration_ms: 0,
                            turns_used: 0,
                            error: Some(format!("Semaphore error: {}", e)),
                        };
                    }
                };

                if cancel.is_cancelled() {
                    return SubAgentResult {
                        task_id,
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some("Cancelled".to_string()),
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
                        success: false,
                        output: String::new(),
                        files_examined: vec![],
                        duration_ms: 0,
                        turns_used: 0,
                        error: Some(format!("Task panicked: {}", e)),
                    });
                }
            }
        }

        let stats = context.stats();
        info!("SubAgentPool: Builders complete | {}", stats);
        results
    }
}
