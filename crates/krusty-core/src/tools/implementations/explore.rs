//! Explore tool - Spawn parallel sub-agents for deep codebase exploration
//!
//! This tool allows the main agent to spawn lightweight sub-agents (Haiku)
//! that search and analyze different parts of the codebase concurrently.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::agent::subagent::{SubAgentPool, SubAgentTask};
use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

/// Explore tool for spawning parallel sub-agents
pub struct ExploreTool {
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
}

impl ExploreTool {
    pub fn new(client: Arc<AiClient>, cancellation: AgentCancellation) -> Self {
        Self {
            client,
            cancellation,
        }
    }
}

#[derive(Deserialize)]
struct Params {
    /// The main question or task to investigate
    prompt: String,

    /// Optional: Specific directories to explore (spawns one agent per directory)
    #[serde(default)]
    directories: Option<Vec<String>>,

    /// Optional: Specific files to analyze (spawns one agent per file)
    #[serde(default)]
    files: Option<Vec<String>>,

    /// Maximum concurrent agents (default: 5)
    #[serde(default = "default_concurrency")]
    max_concurrency: usize,
}

fn default_concurrency() -> usize {
    10 // Balanced limit to prevent resource exhaustion while allowing parallelism
}

#[async_trait]
impl Tool for ExploreTool {
    fn name(&self) -> &str {
        "explore"
    }

    fn description(&self) -> &str {
        "Launch parallel sub-agents to explore the codebase autonomously. \
         IMPORTANT: Pass 'directories' array to spawn MULTIPLE parallel agents (one per directory). \
         Without directories, only ONE agent is spawned. \
         Example for comprehensive exploration: directories=['src/tui', 'src/agent', 'src/tools', 'src/ai']. \
         USE THIS TOOL when the user asks to 'explore', 'investigate', 'audit', 'analyze', \
         or 'understand' the codebase. Sub-agents work concurrently with glob, grep, and read tools. \
         Returns aggregated findings from all agents."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The question or task for the sub-agents to investigate"
                },
                "directories": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "RECOMMENDED: List of directories to explore in parallel. Each directory gets its own agent. For comprehensive exploration, pass main src subdirs like ['src/tui', 'src/agent', 'src/tools']"
                },
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Specific files to analyze (optional, spawns one agent per file)"
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        info!("Explore tool execute called with params: {:?}", params);

        let params: Params = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                warn!("Explore tool: Invalid parameters: {}", e);
                return ToolResult {
                    output: json!({"error": format!("Invalid parameters: {}", e)}).to_string(),
                    is_error: true,
                };
            }
        };

        // Build tasks based on input - all use Haiku (fast, cheap, effective for exploration)
        let mut tasks: Vec<SubAgentTask> = Vec::new();

        if let Some(dirs) = params.directories {
            // One agent per directory - derive name from last path component
            for (i, dir) in dirs.iter().enumerate() {
                let name = dir.rsplit('/').find(|s| !s.is_empty()).unwrap_or("dir");
                tasks.push(
                    SubAgentTask::new(
                        format!("dir-{}", i),
                        format!("In directory '{}': {}", dir, params.prompt),
                    )
                    .with_name(name)
                    .with_working_dir(ctx.working_dir.clone()),
                );
            }
        } else if let Some(files) = params.files {
            // One agent per file - derive name from filename without extension
            for (i, file) in files.iter().enumerate() {
                let name = file.rsplit('/').next().unwrap_or("file");
                let name = name.split('.').next().unwrap_or(name);
                tasks.push(
                    SubAgentTask::new(
                        format!("file-{}", i),
                        format!("Analyze file '{}': {}", file, params.prompt),
                    )
                    .with_name(name)
                    .with_working_dir(ctx.working_dir.clone()),
                );
            }
        } else {
            // Single agent for general exploration
            tasks.push(
                SubAgentTask::new("main", params.prompt.clone())
                    .with_name("explore")
                    .with_working_dir(ctx.working_dir.clone()),
            );
        }

        info!("Explore tool: Created {} tasks", tasks.len());
        for (i, task) in tasks.iter().enumerate() {
            debug!(
                "Task {}: id={}, name={}, prompt_len={}",
                i,
                task.id,
                task.name,
                task.prompt.len()
            );
        }

        // Create pool and execute (with progress if channel available)
        let pool = SubAgentPool::new(self.client.clone(), self.cancellation.clone())
            .with_concurrency(params.max_concurrency)
            .with_override_model(ctx.current_model.clone());

        info!(
            "Explore tool: Starting pool execution with max_concurrency={}",
            params.max_concurrency
        );
        let results = if let Some(ref progress_tx) = ctx.explore_progress_tx {
            pool.execute_with_progress(tasks, progress_tx.clone()).await
        } else {
            pool.execute(tasks).await
        };
        info!("Explore tool: Pool returned {} results", results.len());

        // Format results
        let mut output = String::new();
        let mut all_files: Vec<String> = Vec::new();
        let mut total_turns = 0;
        let mut total_duration_ms = 0u64;
        let mut errors: Vec<String> = Vec::new();

        for result in &results {
            if result.success {
                output.push_str(&format!("\n## Agent: {}\n", result.task_id));
                output.push_str(&result.output);
                output.push('\n');
            } else if let Some(err) = &result.error {
                errors.push(format!("{}: {}", result.task_id, err));
            }

            all_files.extend(result.files_examined.clone());
            total_turns += result.turns_used;
            total_duration_ms += result.duration_ms;
        }

        // Add summary
        let summary = format!(
            "\n---\n**Summary**: {} agents, {} turns total, {}ms, {} files examined",
            results.len(),
            total_turns,
            total_duration_ms,
            all_files.len()
        );
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
