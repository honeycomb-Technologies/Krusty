//! Unified agent tool — spawns agnostic parent-directed child agents.
//!
//! Children are malleable workers: the parent supplies a name and instructions.
//! Capabilities select tool access (read vs write vs execute). Legacy
//! profile/agent_type labels map to capability defaults only.

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
use crate::storage::{Database, SessionType};
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};
use crate::SessionManager;

use helpers::*;

/// Unified agent tool — spawns and supervises agnostic child agents.
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

    /// Parent instructions for the child (preferred product field).
    #[serde(default)]
    instructions: Option<String>,

    /// The main objective for the sub-agent (alias of instructions).
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

    /// Exact durable run selected by lifecycle `resume`/terminal `followup`.
    /// Fresh spawns may still discover related evidence, but an explicit
    /// resume must never be rebound to a newer sibling with the same scope.
    #[serde(skip)]
    resumed_from_run_id: Option<String>,

    /// A single component is executed by the unified child rather than the
    /// legacy parallel pool. Keep the assignment after `components` is
    /// normalized away so it remains part of the durable resume contract.
    #[serde(skip)]
    assigned_component: Option<String>,

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

    /// Parent-chosen child identity for spawn calls. This remains optional in
    /// deserialization for lifecycle actions and legacy compatibility; current
    /// spawn validation requires a non-empty name.
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
        "Spawn and supervise agnostic child agents directed by the parent. Use for parallel or deep multi-file work — not simple lookups or one-file edits. Required product fields: name (from your plan) and instructions (or prompt). Optional capabilities: read, write, execute (parent policy is the ceiling). Actions: spawn, list, status, wait, message, followup, interrupt, resume. The parent integrates results."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            "Spawn a named child with clear instructions for substantial independent work. Use wait only when you must block. message steers a live child; followup/resume continue from durable evidence. Keep multi-scope digs on children so the parent transcript stays thin.",
        )
    }

    fn parameters_schema(&self) -> Value {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "list", "status", "wait", "message", "followup", "interrupt", "resume"],
                    "description": "Spawn or supervise a delegated child; defaults to spawn"
                },
                "delegated_run_id": {
                    "type": "string",
                    "description": "Run targeted by status, wait, message, followup, interrupt, or resume"
                },
                "message": {
                    "type": "string",
                    "description": "Steering for a live child, or the next objective when followup resumes a completed child"
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
                "name": {
                    "type": "string",
                    "description": "Parent-chosen task name for status and completion (from your plan)"
                },
                "instructions": {
                    "type": "string",
                    "description": "Full parent instructions that shape this child (preferred over prompt)"
                },
                "prompt": {
                    "type": "string",
                    "description": "Alias of instructions — objective for the child"
                },
                "capabilities": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "string",
                        "enum": ["read", "write", "execute"]
                    },
                    "description": "Requested capabilities; parent governance is the ceiling. write enables edits; execute enables bash when write is not set"
                },
                "profile": {
                    "type": "string",
                    "description": "Optional legacy label (explore/build/plan/verify/custom). Prefer name + instructions + capabilities"
                },
                "expected_output": {
                    "type": "string",
                    "description": "Expected result or proof contract"
                },
                "delegation_reason": {
                    "type": "string",
                    "description": "Why delegation improves this task"
                },
                "context": {
                    "type": "string",
                    "enum": ["auto", "project", "brief", "full"],
                    "description": "Parent context inherited by the child"
                },
                "max_turns": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Child turn budget; omitted inherits parent or defaults to 20"
                },
                "scope": {
                    "type": "string",
                    "description": "Optional path scope for the child"
                },
                "components": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional parallel write components (one child per component)"
                },
                "conventions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Shared conventions for parallel writers"
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Optional parallel writer ceiling",
                    "minimum": 1
                },
                "task_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional plan task IDs corresponding to components"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Hosted Chat/Code only: return immediately; the parent is notified when the child completes"
                },
                "description": {
                    "type": "string",
                    "description": "Short status label"
                }
            },
            "additionalProperties": false
        });
        if !self.runtime.has_completion_listener() {
            schema["properties"]
                .as_object_mut()
                .expect("Agent schema properties")
                .remove("run_in_background");
        }
        schema
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        info!("Agent tool execute called with params: {:?}", params);

        if has_explicit_empty_capabilities(&params) {
            return ToolResult::error_with_code(
                "invalid_agent_capabilities",
                "Agent capabilities cannot be an empty array. Omit the field for the default read capability, or request one or more of read, write, and execute.",
            );
        }

        let mut params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => {
                warn!("Agent tool parameter validation failed: {}", e.output);
                return e;
            }
        };

        params.normalize_instructions();

        if params.action == AgentAction::Spawn {
            self.execute_spawn(params, ctx).await
        } else {
            self.execute_control(params, ctx).await
        }
    }
}

fn has_explicit_empty_capabilities(params: &Value) -> bool {
    params
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
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

        if params.run_in_background.unwrap_or(false) {
            if let Err(error) = validate_background_wake_host(&self.runtime, ctx) {
                return error;
            }
        }
        params.prompt = spec.rendered_objective();
        params.profile = Some(spec.profile.clone());
        // agent_type retained for internal compatibility as capability class only.
        params.agent_type = Some(execution_profile.as_str().to_string());
        params.max_turns = spec.max_turns;
        params.name = Some(spec.task_name.clone());
        if let Some(turns) = spec.parent_context_turns() {
            if let Some(parent_conversation) = ctx.parent_conversation.as_ref() {
                let brief = build_parent_context_brief(parent_conversation, turns);
                if !brief.is_empty() {
                    params.prompt = format!("{}\n\n{}", brief, params.prompt);
                }
            }
        }
        params.parent_context_applied = true;
        if let Some(components) = params.components.as_mut() {
            components.retain(|component| !component.trim().is_empty());
        }
        let parallel_components =
            should_use_parallel_component_pool(execution_profile, params.components.as_deref());
        if !parallel_components {
            if let Some(component) = params
                .components
                .as_ref()
                .and_then(|components| components.first())
            {
                params.assigned_component = Some(component.trim().to_string());
                params.prompt = format!(
                    "{}\n\nAssigned component: {}",
                    params.prompt,
                    component.trim()
                );
            }
            params.components = None;
        }

        // Stash capability hints for child policy selection in execute paths.
        params.capabilities = spec
            .capabilities
            .iter()
            .map(|cap| match cap {
                crate::agent::subagent::AgentCapability::Read => "read".to_string(),
                crate::agent::subagent::AgentCapability::Write => "write".to_string(),
                crate::agent::subagent::AgentCapability::Execute => "execute".to_string(),
            })
            .collect();

        info!(
            name = %spec.task_name,
            description = ?params.description,
            profile = %spec.profile,
            execution_profile = %execution_profile.as_str(),
            capabilities = ?spec.capabilities,
            max_turns = ?spec.max_turns,
            background = params.run_in_background.unwrap_or(false),
            "Resolved delegated AgentSpec (agnostic child)"
        );

        if parallel_components {
            self.execute_build(params, ctx).await
        } else {
            self.execute_child(params, ctx).await
        }
    }
}

fn validate_background_wake_host(
    runtime: &AgentRuntimeManager,
    ctx: &ToolContext,
) -> Result<(), ToolResult> {
    if !runtime.has_completion_listener() {
        return Err(ToolResult::error_with_code(
            "background_wake_unsupported",
            "Background Agent execution requires a live parent-wake host. Use a foreground Agent on this surface.",
        ));
    }
    let db_path = ctx.db_path.as_deref().ok_or_else(|| {
        ToolResult::error_with_code(
            "background_session_required",
            "Background Agent execution requires durable session storage.",
        )
    })?;
    let session_id = ctx.session_id.as_deref().ok_or_else(|| {
        ToolResult::error_with_code(
            "background_session_required",
            "Background Agent execution requires a persisted parent session.",
        )
    })?;
    let manager =
        SessionManager::new(Database::new(db_path).map_err(|error| {
            ToolResult::error_with_code("agent_store_error", error.to_string())
        })?);
    let session = manager
        .get_session(session_id)
        .map_err(|error| ToolResult::error_with_code("agent_store_error", error.to_string()))?
        .ok_or_else(|| {
            ToolResult::error_with_code(
                "background_session_required",
                format!("Parent session '{session_id}' was not found."),
            )
        })?;
    if !matches!(session.session_type, SessionType::Chat | SessionType::Code) {
        return Err(ToolResult::error_with_code(
            "background_wake_unsupported",
            "Background child completion cannot autonomously wake a Hive-owned session. Run this Agent in the foreground.",
        ));
    }
    Ok(())
}

fn should_use_parallel_component_pool(
    execution_profile: AgentExecutionProfile,
    components: Option<&[String]>,
) -> bool {
    execution_profile == AgentExecutionProfile::Build
        && components.is_some_and(|components| components.len() > 1)
}

impl Params {
    /// Normalize the preferred product field before either spawn or durable
    /// resume control paths inspect the objective.
    fn normalize_instructions(&mut self) {
        if self.prompt.trim().is_empty() {
            if let Some(instructions) = self.instructions.as_deref() {
                self.prompt = instructions.trim().to_string();
            }
        } else if let Some(instructions) = self.instructions.as_deref() {
            if !instructions.trim().is_empty() && instructions.trim() != self.prompt.trim() {
                self.prompt = format!("{}\n\n{}", instructions.trim(), self.prompt.trim());
            }
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
}
