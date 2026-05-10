//! Unified agent tool — dispatches explore, plan, verify, and build agents.
//!
//! Replaces the separate `explore` and `build` tools with a single tool that
//! accepts an `agent_type` parameter to select the sub-agent flavor.

mod build;
mod helpers;
mod single;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, warn};

use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use helpers::*;

/// Unified agent tool — dispatches explore, plan, verify, and build sub-agents.
pub struct AgentTool {
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
}

impl AgentTool {
    pub fn new(client: Arc<AiClient>, cancellation: AgentCancellation) -> Self {
        Self {
            client,
            cancellation,
        }
    }
}

#[derive(Deserialize)]
struct Params {
    /// Sub-agent type: "explore", "plan", "verify", "build"
    agent_type: String,

    /// The main question or task for the sub-agent
    prompt: String,

    /// Optional: path hint to scope exploration (explore only)
    #[serde(default)]
    scope: Option<String>,

    /// Components to build in parallel (build only, one agent per component)
    #[serde(default)]
    components: Option<Vec<String>>,

    /// Coding conventions all builders must follow (build only)
    #[serde(default)]
    conventions: Option<Vec<String>>,

    /// Maximum concurrent builders (build only, defaults to component count)
    #[serde(default)]
    max_concurrency: Option<usize>,

    /// Plan task IDs corresponding to each component (build only, for auto-marking)
    #[serde(default)]
    task_ids: Option<Vec<String>>,

    /// Run the agent in the background. Returns immediately with delegated_run_id.
    #[serde(default)]
    run_in_background: Option<bool>,

    /// Optional stable label for a background agent run in Mako mode.
    /// This is used for progress/status visibility, not mailbox routing.
    #[serde(default)]
    name: Option<String>,

    /// Short description of what this agent does (for status display).
    #[serde(default)]
    description: Option<String>,
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "Launch a specialized sub-agent. Types: 'explore' (read-only codebase investigation), \
         'plan' (implementation planning), 'verify' (test and validate changes), \
         'build' (parallel code implementation)."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Dispatches work to specialized sub-agents by type:

- **explore**: Deep codebase investigation with read-only tools. Use for multi-file analysis or understanding unfamiliar code. Pass 'scope' to narrow focus. Inherits parent conversation context.
- **plan**: Generate implementation plans — steps, critical files, trade-offs, dependencies. Read-only. Fresh context.
- **verify**: Run tests, builds, linters and validate changes. Outputs VERDICT: PASS|FAIL|PARTIAL. Fresh context.
- **build**: Parallel code implementation. Pass 'components' array and optionally 'conventions'. Fresh context.

**Background mode:** Pass `run_in_background: true` to spawn the agent asynchronously. You get back a `delegated_run_id` immediately and can continue working. The agent writes its result to the delegated run store when finished — you will see it in your delegated context on the next turn. Use this for long-running tasks where you don't need the result right away.

**Named background agents:** Pass `name` to give a background run a stable label in status/progress views. Use with `run_in_background: true`. Example: `agent(agent_type: "build", prompt: "Implement task T-12", name: "builder-1", run_in_background: true)`.

For simple file lookups, use Glob/Grep/Read directly — agent is for deeper multi-step work."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_type": {
                    "type": "string",
                    "enum": ["explore", "plan", "verify", "build"],
                    "description": "Sub-agent type to launch"
                },
                "prompt": {
                    "type": "string",
                    "description": "The question or task for the sub-agent"
                },
                "scope": {
                    "type": "string",
                    "description": "Optional: path to a directory or file to focus exploration on (explore only)"
                },
                "components": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Components to build in parallel. Each gets its own builder agent. (build only)"
                },
                "conventions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Coding conventions all builders must follow. (build only)"
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Max parallel builders. Default: component count. Use 2-3 for tightly coupled code, 5-10 for independent modules. (build only)",
                    "minimum": 1,
                    "maximum": 20
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Run agent in background. Returns immediately with delegated_run_id. Check status via delegated run store. You will see results in the delegated context on your next turn."
                },
                "name": {
                    "type": "string",
                    "description": "Optional stable label for a background agent run. Use with run_in_background: true so progress is easier to track."
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what this agent does (3-5 words, for status display)."
                }
            },
            "required": ["agent_type", "prompt"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        info!("Agent tool execute called with params: {:?}", params);

        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => {
                warn!("Agent tool parameter validation failed: {}", e.output);
                return e;
            }
        };

        if let Some(ref name) = params.name {
            info!(
                name = %name,
                description = ?params.description,
                agent_type = %params.agent_type,
                "Named background agent requested"
            );
        }

        match params.agent_type.as_str() {
            "explore" => self.execute_explore(params, ctx).await,
            "plan" => self.execute_plan(params, ctx).await,
            "verify" => self.execute_verify(params, ctx).await,
            "build" => self.execute_build(params, ctx).await,
            other => ToolResult::error_with_code(
                "invalid_parameters",
                format!(
                    "Unknown agent_type '{}'. Must be one of: explore, plan, verify, build",
                    other
                ),
            ),
        }
    }
}

impl AgentTool {
    /// Resolve the session-scoped AI client, falling back to registration-time client.
    fn resolve_client(&self, ctx: &ToolContext) -> Arc<AiClient> {
        if let Some(ref session_client) = ctx.ai_client {
            if session_client.provider_id() != self.client.provider_id()
                || session_client.config().model != self.client.config().model
            {
                info!(
                    base_provider = %self.client.provider_id(),
                    session_provider = %session_client.provider_id(),
                    base_model = %self.client.config().model,
                    session_model = %session_client.config().model,
                    "Agent tool using session AI client instead of registration-time client"
                );
            }
            session_client.clone()
        } else {
            self.client.clone()
        }
    }

    /// Resolve the model — use the user's current model or the provider default.
    fn resolve_model(&self, ctx: &ToolContext, client: &AiClient) -> String {
        ctx.current_model
            .clone()
            .unwrap_or_else(|| client.config().model.clone())
    }

    /// Resolve a fast/cheap model for lightweight agent tasks (e.g., explore).
    /// Only downgrades for providers that have a known compatible fast tier — otherwise
    /// inherits the parent model. OpenAI ChatGPT/Codex accounts do not support every
    /// "mini" model on the websocket tool path, so keep OpenAI delegated runs on the
    /// selected parent model instead of silently switching to a mini variant.
    fn resolve_fast_model(&self, ctx: &ToolContext, client: &AiClient) -> String {
        use crate::ai::providers::ProviderId;
        match client.provider_id() {
            ProviderId::Anthropic => "claude-haiku-4-5-20251001".to_string(),
            _ => self.resolve_model(ctx, client),
        }
    }
}
