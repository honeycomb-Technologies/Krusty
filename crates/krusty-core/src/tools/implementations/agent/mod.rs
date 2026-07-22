//! Unified agent tool — dispatches explore, plan, verify, and build agents.
//!
//! Replaces separate rigid agent tools with one dynamic profile and capability
//! contract. Legacy agent_type input remains internal compatibility only.

mod build;
mod control;
mod helpers;
mod single;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, warn};

use crate::agent::subagent::{AgentExecutionProfile, AgentRuntimeManager, AgentSpec};
use crate::agent::AgentCancellation;
use crate::ai::client::AiClient;
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use helpers::*;

/// Unified agent tool — dispatches explore, plan, verify, and build sub-agents.
pub struct AgentTool {
    client: Arc<AiClient>,
    cancellation: AgentCancellation,
    runtime: AgentRuntimeManager,
}

impl AgentTool {
    pub fn new(
        client: Arc<AiClient>,
        cancellation: AgentCancellation,
        runtime: AgentRuntimeManager,
    ) -> Self {
        Self {
            client,
            cancellation,
            runtime,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentAction {
    #[default]
    Spawn,
    List,
    Status,
    Wait,
    Message,
    Followup,
    Interrupt,
    Resume,
}

#[derive(Deserialize, Default)]
struct Params {
    /// Lifecycle operation. Spawn is the default.
    #[serde(default)]
    action: AgentAction,

    /// Existing delegated run targeted by lifecycle operations.
    #[serde(default)]
    delegated_run_id: Option<String>,

    /// Parent steering for message/followup.
    #[serde(default)]
    message: Option<String>,

    /// Bounded wait duration.
    #[serde(default)]
    wait_timeout_ms: Option<u64>,

    /// Maximum records returned by list.
    #[serde(default)]
    limit: Option<usize>,

    /// Backward-compatible built-in profile.
    #[serde(default)]
    agent_type: Option<String>,

    /// Optional built-in or custom profile.
    #[serde(default)]
    profile: Option<String>,

    /// The main objective for the sub-agent.
    #[serde(default)]
    prompt: String,

    /// Requested result shape or proof contract.
    #[serde(default)]
    expected_output: Option<String>,

    /// Concise reason delegation improves this task.
    #[serde(default)]
    delegation_reason: Option<String>,

    /// Requested capabilities, clamped by the parent policy.
    #[serde(default)]
    capabilities: Vec<String>,

    /// Parent context strategy: auto, project, brief, or full.
    #[serde(default)]
    context: Option<String>,

    /// Optional per-agent turn budget. Parent ceiling still wins.
    #[serde(default)]
    max_turns: Option<usize>,

    /// Internal marker preventing duplicate parent context injection.
    #[serde(skip)]
    parent_context_applied: bool,
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
        "Spawn and supervise independent agents. Delegate parallel, deep multi-file, or background work; avoid simple lookups, one-file edits, and tightly coupled work. Profiles: explore, plan, verify, build, or custom. Actions: spawn, list, status, wait, message, followup, interrupt, resume. The parent verifies results."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            "Use action=spawn for substantial independent work. Set run_in_background=true when the parent can continue concurrently. Use list/status/wait to observe; message/followup to steer a live child; interrupt to cancel one run; resume to start a new run from durable prior evidence. The parent remains responsible for integration and verification.",
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "list", "status", "wait", "message", "followup", "interrupt", "resume"],
                    "description": "Spawn or supervise a delegated run; defaults to spawn"
                },
                "delegated_run_id": {
                    "type": "string",
                    "description": "Run targeted by status, wait, message, followup, interrupt, or resume"
                },
                "message": {
                    "type": "string",
                    "description": "Steering sent between child turns"
                },
                "wait_timeout_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 300000,
                    "description": "Maximum status wait"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum runs returned by list"
                },
                "profile": {
                    "type": "string",
                    "description": "Optional built-in or custom agent profile"
                },
                "prompt": {
                    "type": "string",
                    "description": "Objective for the sub-agent"
                },
                "expected_output": {
                    "type": "string",
                    "description": "Expected result or proof contract"
                },
                "delegation_reason": {
                    "type": "string",
                    "description": "Why delegation improves this task"
                },
                "capabilities": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["read", "write", "execute"]
                    },
                    "description": "Requested capabilities; parent governance is the ceiling"
                },
                "context": {
                    "type": "string",
                    "enum": ["auto", "project", "brief", "full"],
                    "description": "Parent context inherited by the child"
                },
                "max_turns": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional child budget; omitted means inherited or unlimited"
                },
                "scope": {
                    "type": "string",
                    "description": "Optional exploration path"
                },
                "components": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Parallel build components"
                },
                "conventions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Shared build conventions"
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Optional parallel builder ceiling",
                    "minimum": 1
                },
                "task_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional plan task IDs corresponding to build components"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Return immediately and run in background"
                },
                "name": {
                    "type": "string",
                    "description": "Background run label"
                },
                "description": {
                    "type": "string",
                    "description": "Short status label"
                }
            },
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

        if params.action == AgentAction::Spawn {
            self.execute_spawn(params, ctx).await
        } else {
            self.execute_control(params, ctx).await
        }
    }
}

impl AgentTool {
    async fn execute_spawn(&self, mut params: Params, ctx: &ToolContext) -> ToolResult {
        let spec = match AgentSpec::resolve(
            params.profile.as_deref(),
            params.agent_type.as_deref(),
            params.name.as_deref(),
            &params.prompt,
            params.expected_output.as_deref(),
            params.delegation_reason.as_deref(),
            &params.capabilities,
            params.context.as_deref(),
            params.max_turns,
            ctx.subagent_max_turns,
        ) {
            Ok(spec) => spec,
            Err(error) => return ToolResult::error_with_code("invalid_agent_spec", error),
        };
        let execution_profile = spec.execution_profile();
        params.prompt = spec.rendered_objective();
        params.profile = Some(spec.profile.clone());
        params.agent_type = Some(execution_profile.as_str().to_string());
        params.max_turns = spec.max_turns;
        params.name.get_or_insert_with(|| spec.task_name.clone());
        if let Some(turns) = spec.parent_context_turns() {
            if let Some(parent_conversation) = ctx.parent_conversation.as_ref() {
                let brief = build_parent_context_brief(parent_conversation, turns);
                if !brief.is_empty() {
                    params.prompt = format!("{}\n\n{}", brief, params.prompt);
                }
            }
        }
        params.parent_context_applied = true;
        if execution_profile == AgentExecutionProfile::Build
            && params.components.as_ref().is_none_or(Vec::is_empty)
        {
            params.components = Some(vec![spec.objective.clone()]);
        }

        info!(
            name = %spec.task_name,
            description = ?params.description,
            profile = %spec.profile,
            execution_profile = %execution_profile.as_str(),
            capabilities = ?spec.capabilities,
            max_turns = ?spec.max_turns,
            background = params.run_in_background.unwrap_or(false),
            "Resolved delegated AgentSpec"
        );

        match execution_profile {
            AgentExecutionProfile::Explore => self.execute_explore(params, ctx).await,
            AgentExecutionProfile::Plan => self.execute_plan(params, ctx).await,
            AgentExecutionProfile::Verify => self.execute_verify(params, ctx).await,
            AgentExecutionProfile::Build => self.execute_build(params, ctx).await,
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

    /// Resolve the immutable model owned by the session client.
    ///
    /// `ToolContext::current_model` is retained as legacy UI metadata. It is a
    /// bare slug and therefore cannot safely override the exact provider/auth/
    /// transport identity frozen into `AiClient` for this run.
    fn resolve_model(&self, _ctx: &ToolContext, client: &AiClient) -> String {
        client.resolved_model().wire_model_id.clone()
    }

    /// Resolve a model for lightweight delegated work.
    ///
    /// A different fast model requires its own exact catalog resolution,
    /// credentials, transport, and `AiClient`. Until that typed boundary is
    /// supplied, inherit the parent runtime instead of changing only a slug.
    fn resolve_fast_model(&self, ctx: &ToolContext, client: &AiClient) -> String {
        self.resolve_model(ctx, client)
    }
}
