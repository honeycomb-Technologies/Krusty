//! Build tool - Spawn parallel Opus builder agents (The Kraken)
//!
//! This tool spawns a team of Opus agents that work together to build code.
//! Builders coordinate via SharedBuildContext to share types, modules, and file locks.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::agent::subagent::{SubAgentModel, SubAgentPool, SubAgentTask};
use crate::agent::{AgentCancellation, SharedBuildContext};
use crate::ai::client::AiClient;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

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

        let params: Params = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                warn!("Build tool: Invalid parameters: {}", e);
                return ToolResult {
                    output: json!({"error": format!("Invalid parameters: {}", e)}).to_string(),
                    is_error: true,
                };
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
                    .with_model(SubAgentModel::Opus)
                    .with_working_dir(ctx.working_dir.clone());

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
                    .with_model(SubAgentModel::Opus)
                    .with_working_dir(ctx.working_dir.clone()),
            );
        }

        info!("Build tool: Created {} builder tasks", tasks.len());
        for (i, task) in tasks.iter().enumerate() {
            debug!(
                "Builder {}: id={}, name={}, model={:?}",
                i, task.id, task.name, task.model
            );
        }

        // Create pool and execute with build context
        let pool = SubAgentPool::new(self.client.clone(), self.cancellation.clone())
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

        // Format results
        let mut output = String::new();
        let mut all_files: Vec<String> = Vec::new();
        let mut total_turns = 0;
        let mut total_duration_ms = 0u64;
        let mut errors: Vec<String> = Vec::new();

        for result in &results {
            if result.success {
                output.push_str(&format!("\n## Builder: {}\n", result.task_id));
                output.push_str(&result.output);
                output.push('\n');
            } else if let Some(err) = &result.error {
                errors.push(format!("{}: {}", result.task_id, err));
            }

            all_files.extend(result.files_examined.clone());
            total_turns += result.turns_used;
            total_duration_ms += result.duration_ms;
        }

        // Add summary with build stats
        let mut summary = format!(
            "\n---\n**Build Complete**: {} builders, {} turns, {}ms\n\
             **Changes**: +{} -{} lines, {} files\n\
             **Locks**: {} contentions",
            results.len(),
            total_turns,
            total_duration_ms,
            stats.lines_added,
            stats.lines_removed,
            stats.files_modified,
            stats.lock_contentions,
        );

        // Add lock wait time info if significant
        if stats.total_lock_wait_ms > 0 {
            summary.push_str(&format!(
                ", {:.1}s total wait",
                stats.total_lock_wait_ms as f64 / 1000.0
            ));
        }

        // Report high contention files
        if !stats.high_contention_files.is_empty() {
            summary.push_str("\n**High Contention Files**:");
            for (path, duration) in &stats.high_contention_files {
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                summary.push_str(&format!(" {} ({:.1}s)", filename, duration.as_secs_f64()));
            }
        }

        output.push_str(&summary);

        if !errors.is_empty() {
            output.push_str("\n**Errors**: ");
            output.push_str(&errors.join(", "));
        }

        ToolResult {
            output,
            is_error: false,
        }
    }
}
