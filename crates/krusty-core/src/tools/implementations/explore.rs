//! Explore tool - Spawn parallel sub-agents for deep codebase exploration
//!
//! This tool allows the main agent to spawn lightweight sub-agents (Haiku)
//! that search and analyze different parts of the codebase concurrently.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::agent::subagent::{SubAgentPool, SubAgentTask};
use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::ai::providers::ProviderId;
use crate::tools::registry::{DelegationPolicy, Tool};
use crate::tools::{parse_params, ToolContext, ToolResult};

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

fn provider_concurrency_cap(provider: ProviderId) -> usize {
    match provider {
        // MiniMax rate-limits parallel sub-agent calls aggressively; keep this low
        ProviderId::MiniMax => 3,
        ProviderId::ZAi => 5,
        _ => 10,
    }
}

fn provider_stagger(provider: ProviderId) -> Duration {
    match provider {
        // Give MiniMax extra spacing between launches to reduce burst failures
        ProviderId::MiniMax => Duration::from_millis(600),
        ProviderId::ZAi => Duration::from_millis(250),
        _ => Duration::from_millis(100),
    }
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

        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => {
                warn!("Explore tool parameter validation failed: {}", e.output);
                return e;
            }
        };

        // Build tasks based on input - all use Haiku (fast, cheap, effective for exploration)
        let mut tasks: Vec<SubAgentTask> = Vec::new();
        let delegation_policy =
            DelegationPolicy::for_subagent_explore(ctx.permission_mode, ctx.subagent_max_turns);

        if let Some(dirs) = params.directories {
            // One agent per directory - derive name from last path component
            for (i, dir) in dirs.iter().enumerate() {
                let name = dir.rsplit('/').find(|s| !s.is_empty()).unwrap_or("dir");
                let mut task = SubAgentTask::new(
                    format!("dir-{}", i),
                    format!("In directory '{}': {}", dir, params.prompt),
                )
                .with_name(name)
                .with_working_dir(ctx.working_dir.clone())
                .with_delegation_policy(delegation_policy.clone());
                if let Some(max_turns) = ctx.subagent_max_turns {
                    task = task.with_max_turns(max_turns);
                }
                tasks.push(task);
            }
        } else if let Some(files) = params.files {
            // One agent per file - derive name from filename without extension
            for (i, file) in files.iter().enumerate() {
                let name = file.rsplit('/').next().unwrap_or("file");
                let name = name.split('.').next().unwrap_or(name);
                let mut task = SubAgentTask::new(
                    format!("file-{}", i),
                    format!("Analyze file '{}': {}", file, params.prompt),
                )
                .with_name(name)
                .with_working_dir(ctx.working_dir.clone())
                .with_delegation_policy(delegation_policy.clone());
                if let Some(max_turns) = ctx.subagent_max_turns {
                    task = task.with_max_turns(max_turns);
                }
                tasks.push(task);
            }
        } else {
            // Single agent for general exploration
            let mut task = SubAgentTask::new("main", params.prompt.clone())
                .with_name("explore")
                .with_working_dir(ctx.working_dir.clone())
                .with_delegation_policy(delegation_policy.clone());
            if let Some(max_turns) = ctx.subagent_max_turns {
                task = task.with_max_turns(max_turns);
            }
            tasks.push(task);
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

        // Use session-scoped AI client when available so provider/model switching
        // immediately applies to explore sub-agents.
        let client = if let Some(ref session_client) = ctx.ai_client {
            if session_client.provider_id() != self.client.provider_id()
                || session_client.config().model != self.client.config().model
            {
                info!(
                    base_provider = %self.client.provider_id(),
                    session_provider = %session_client.provider_id(),
                    base_model = %self.client.config().model,
                    session_model = %session_client.config().model,
                    "Explore tool using session AI client instead of registration-time client"
                );
            }
            session_client.clone()
        } else {
            self.client.clone()
        };

        // Provider-aware throttling: some Anthropic-compatible providers (MiniMax)
        // are much less tolerant of parallel sub-agent bursts.
        let provider = client.provider_id();
        let cap = provider_concurrency_cap(provider);
        let requested = params.max_concurrency.max(1);
        let task_count = tasks.len().max(1);
        let effective_concurrency = requested.min(cap).min(task_count);

        if effective_concurrency < requested {
            warn!(
                provider = %provider,
                requested,
                effective = effective_concurrency,
                cap,
                "Explore tool concurrency clamped for provider stability"
            );
        }

        // Create pool and execute (with progress if channel available)
        let pool = SubAgentPool::new(client, self.cancellation.clone())
            .with_concurrency(effective_concurrency)
            .with_stagger_delay(provider_stagger(provider))
            .with_override_model(ctx.current_model.clone());

        info!(
            "Explore tool: Starting pool execution provider={} requested_concurrency={} effective_concurrency={}",
            provider,
            requested,
            effective_concurrency
        );
        let results = if let Some(ref progress_tx) = ctx.explore_progress_tx {
            pool.execute_with_progress(tasks, progress_tx.clone()).await
        } else {
            pool.execute(tasks).await
        };
        info!("Explore tool: Pool returned {} results", results.len());

        let mut all_files: Vec<String> = Vec::new();
        let mut total_turns = 0;
        let mut total_duration_ms = 0u64;
        let mut errors: Vec<String> = Vec::new();
        let mut agent_findings = Vec::new();

        for result in &results {
            agent_findings.push(result.evidence_json());

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
            "Explore completed: {} agents, {} turns, {} files examined",
            results.len(),
            total_turns,
            unique_files.len()
        );

        ToolResult::success_data(json!({
            "message": message,
            "agent_count": results.len(),
            "total_turns": total_turns,
            "total_duration_ms": total_duration_ms,
            "files_examined": unique_files,
            "agents": agent_findings,
            "errors": errors,
            "delegation_policy": delegation_policy.audit_json(),
        }))
    }
}
