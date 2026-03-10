//! Build tool - Spawn parallel Opus builder agents (The Kraken)
//!
//! This tool spawns a team of Opus agents that work together to build code.
//! Builders coordinate via SharedBuildContext to share types, modules, and file locks.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::agent::subagent::{SubAgentPool, SubAgentTask};
use crate::agent::{AgentCancellation, SharedBuildContext};
use crate::ai::client::AiClient;
use crate::tools::registry::{DelegationPolicy, Tool};
use crate::tools::{parse_params, ToolContext, ToolResult};

/// Build tool for spawning parallel Opus builder agents
pub struct BuildTool {
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
}

impl BuildTool {
    pub fn new(client: Arc<AiClient>, cancellation: AgentCancellation) -> Self {
        Self {
            client,
            cancellation,
        }
    }
}

#[derive(Deserialize)]
struct Params {
    /// The overall build goal/requirements
    prompt: String,

    /// Components to build in parallel (one agent per component)
    #[serde(default)]
    components: Option<Vec<String>>,

    /// Coding conventions all builders must follow
    #[serde(default)]
    conventions: Option<Vec<String>>,

    /// Maximum concurrent builders (agent-controlled, defaults to component count)
    #[serde(default)]
    max_concurrency: Option<usize>,

    /// Plan task IDs corresponding to each component (for auto-marking)
    /// Index i maps to components[i]
    #[serde(default)]
    task_ids: Option<Vec<String>>,
}

#[async_trait]
impl Tool for BuildTool {
    fn name(&self) -> &str {
        "build"
    }

    fn description(&self) -> &str {
        "Launch parallel builder agents to implement code. \
         USE THIS TOOL ONLY when the user explicitly asks for: \
         'unleash the kraken', 'release the kraken', 'team of agents', 'squad of builders', \
         'agent swarm', 'parallel agents', 'builder swarm', or 'multiple agents working together'. \
         Pass 'components' array to assign work (e.g., ['auth module', 'api endpoints', 'database layer']). \
         Use 'max_concurrency' to control parallelism: \
         2-3 for tightly coupled components (shared files), \
         5-10 for independent components (separate files). \
         Default: matches component count (natural parallelism). \
         Builders coordinate via file locking - more concurrency is fine if components don't share files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Overall build goal and requirements for the builder team"
                },
                "components": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Components to build in parallel. Each gets its own builder agent. Example: ['auth module', 'api endpoints', 'database models']"
                },
                "conventions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Coding conventions all builders must follow. Example: ['Use anyhow for errors', 'Add tracing logs']"
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Max parallel builders. Default: component count. Use 2-3 for tightly coupled code (shared files), 5-10 for independent modules.",
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        info!(
            "Build tool (Kraken) execute called with params: {:?}",
            params
        );

        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => {
                warn!("Build tool parameter validation failed: {}", e.output);
                return e;
            }
        };

        // Create shared build context
        let context = Arc::new(SharedBuildContext::new());

        // Set conventions if provided
        if let Some(conventions) = &params.conventions {
            context.set_conventions(conventions.clone());
        }

        // Smart concurrency default: match component count, clamped to reasonable range
        let num_components = params.components.as_ref().map(|c| c.len()).unwrap_or(1);
        let concurrency = params.max_concurrency.unwrap_or_else(|| {
            // Default: match component count, capped at reasonable limit
            num_components.clamp(2, 10)
        });

        // Build tasks - all use Opus for high-quality code generation
        let mut tasks: Vec<SubAgentTask> = Vec::new();
        let delegation_policy =
            DelegationPolicy::for_subagent_build(ctx.permission_mode, ctx.subagent_max_turns);

        if let Some(ref components) = params.components {
            let total = components.len();
            let other_components: Vec<_> = components.iter().map(|c| c.as_str()).collect();

            // One agent per component - each gets their own file for TRUE parallelism
            for (i, component) in components.iter().enumerate() {
                let name = component.split_whitespace().next().unwrap_or("builder");
                let others: Vec<_> = other_components
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(j, c)| format!("  - Builder {}: {}", j, c))
                    .collect();

                // Create detailed prompt emphasizing SEPARATE FILES
                let task_prompt = format!(
                    "You are Builder {} of {} in a parallel build team.\n\n\
                     YOUR COMPONENT: {}\n\n\
                     OVERALL GOAL:\n{}\n\n\
                     OTHER BUILDERS (working in parallel):\n{}\n\n\
                     PARALLEL BUILD STRATEGY:\n\
                     1. Create YOUR OWN file(s) for your component - don't wait for others\n\
                     2. Name files clearly: {}_something.ext (e.g., game_engine.py, snake_logic.py)\n\
                     3. If you need to import from another builder's module, assume it exists\n\
                     4. Export clear interfaces (functions, classes) others can import\n\
                     5. At the end, if a main.py/main.rs is needed, Builder 0 creates it and imports all modules\n\n\
                     COORDINATION:\n\
                     - Check [SHARED TYPES] for interfaces other builders registered\n\
                     - Register YOUR public functions/classes so others can import them\n\
                     - File locks are automatic - but you shouldn't need them if using separate files\n\n\
                     BUILD YOUR COMPONENT NOW. Create your file(s) and implement fully.",
                    i, total,
                    component,
                    params.prompt,
                    if others.is_empty() { "  (none - you're solo)".to_string() } else { others.join("\n") },
                    name.to_lowercase().replace(' ', "_")
                );

                let mut task = SubAgentTask::new(format!("builder-{}", i), task_prompt)
                    .with_name(name)
                    .with_working_dir(ctx.working_dir.clone())
                    .with_delegation_policy(delegation_policy.clone());
                if let Some(max_turns) = ctx.subagent_max_turns {
                    task = task.with_max_turns(max_turns);
                }

                // Attach plan task ID if provided for auto-completion
                if let Some(ref task_ids) = params.task_ids {
                    if let Some(plan_task_id) = task_ids.get(i) {
                        task = task.with_plan_task_id(plan_task_id);
                    }
                }

                tasks.push(task);
            }
        } else {
            // Single builder for the whole task
            tasks.push(
                SubAgentTask::new("builder-main", params.prompt.clone())
                    .with_name("main")
                    .with_working_dir(ctx.working_dir.clone())
                    .with_delegation_policy(delegation_policy.clone()),
            );
            if let Some(max_turns) = ctx.subagent_max_turns {
                if let Some(last) = tasks.last_mut() {
                    last.max_turns_override = Some(max_turns);
                }
            }
        }

        info!("Build tool: Created {} builder tasks", tasks.len());
        for (i, task) in tasks.iter().enumerate() {
            debug!("Builder {}: id={}, name={}", i, task.id, task.name);
        }

        // Use session-scoped AI client when available so provider/model switching
        // immediately applies to builder sub-agents.
        let client = if let Some(ref session_client) = ctx.ai_client {
            if session_client.provider_id() != self.client.provider_id()
                || session_client.config().model != self.client.config().model
            {
                info!(
                    base_provider = %self.client.provider_id(),
                    session_provider = %session_client.provider_id(),
                    base_model = %self.client.config().model,
                    session_model = %session_client.config().model,
                    "Build tool using session AI client instead of registration-time client"
                );
            }
            session_client.clone()
        } else {
            self.client.clone()
        };

        // Create pool and execute with build context
        let pool = SubAgentPool::new(client, self.cancellation.clone())
            .with_concurrency(concurrency)
            .with_override_model(ctx.current_model.clone());

        info!(
            "Build tool: Starting Kraken with max_concurrency={} (components={})",
            concurrency, num_components
        );

        // Execute builders with progress channel if available
        let results = if let Some(ref progress_tx) = ctx.build_progress_tx {
            pool.execute_builders(tasks, context.clone(), progress_tx.clone())
                .await
        } else {
            // Fallback: create a dummy channel and discard progress
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            pool.execute_builders(tasks, context.clone(), tx).await
        };

        info!("Build tool: Kraken returned {} results", results.len());

        // Get final stats from context
        let stats = context.stats();

        let mut all_files: Vec<String> = Vec::new();
        let mut total_turns = 0;
        let mut total_duration_ms = 0u64;
        let mut errors: Vec<String> = Vec::new();
        let mut builders = Vec::new();

        for result in &results {
            builders.push(result.evidence_json());

            if let Some(err) = &result.error {
                errors.push(format!("{}: {}", result.task_id, err));
            }

            all_files.extend(result.files_examined.clone());
            total_turns += result.turns_used;
            total_duration_ms += result.duration_ms;
        }

        let mut unique_files = Vec::new();
        for file in all_files {
            if !unique_files.iter().any(|existing| existing == &file) {
                unique_files.push(file);
            }
        }

        let message = format!(
            "Build completed: {} builders, {} turns, +{} -{} lines across {} files",
            results.len(),
            total_turns,
            stats.lines_added,
            stats.lines_removed,
            stats.files_modified,
        );

        let high_contention_files = stats
            .high_contention_files
            .iter()
            .map(|(path, duration)| {
                json!({
                    "file": path.display().to_string(),
                    "wait_secs": duration.as_secs_f64(),
                })
            })
            .collect::<Vec<_>>();

        ToolResult::success_data(json!({
            "message": message,
            "builder_count": results.len(),
            "total_turns": total_turns,
            "total_duration_ms": total_duration_ms,
            "files_examined": unique_files,
            "builders": builders,
            "lines_added": stats.lines_added,
            "lines_removed": stats.lines_removed,
            "files_modified": stats.files_modified,
            "lock_contentions": stats.lock_contentions,
            "total_lock_wait_ms": stats.total_lock_wait_ms,
            "high_contention_files": high_contention_files,
            "errors": errors,
            "delegation_policy": delegation_policy.audit_json(),
        }))
    }
}
