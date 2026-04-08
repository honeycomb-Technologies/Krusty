//! Unified agent tool — dispatches explore, plan, verify, and build agents.
//!
//! Replaces the separate `explore` and `build` tools with a single tool that
//! accepts an `agent_type` parameter to select the sub-agent flavor.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::agent::agent_types::{PlanConfig, VerifyConfig};
use crate::agent::context::build_subagent_project_context;
use crate::agent::subagent::{
    execute_single_agent, execute_single_explorer, AgentProgress, AgentProgressStatus,
    SubAgentPool, SubAgentResult, SubAgentTask,
};
use crate::agent::{AgentCancellation, DelegatedRunStage, SharedBuildContext};
use crate::ai::client::AiClient;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::{
    Database, DelegatedRunRecord, DelegatedRunRole, DelegatedRunScope, DelegatedRunStartInput,
    DelegatedRunStore, ProjectSettings,
};
use crate::tools::registry::{DelegationPolicy, Tool};
use crate::tools::{parse_params, ToolContext, ToolResult};

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

// ---------------------------------------------------------------------------
// Background dispatch helper
// ---------------------------------------------------------------------------

/// Build the immediate response for a background agent launch.
fn background_started_result(
    delegated_run_id: &str,
    agent_type: &str,
    name: Option<&str>,
) -> ToolResult {
    let mut result = json!({
        "status": "background_started",
        "delegated_run_id": delegated_run_id,
        "agent_type": agent_type,
        "message": format!(
            "{} agent started in background. You will see the result in your delegated context on the next turn. \
             Use delegated_run_id '{}' to track status.",
            agent_type, delegated_run_id
        ),
    });
    if let Some(name) = name {
        result["name"] = json!(name);
        result["message"] = json!(format!(
            "Named agent '{}' ({}) started in background. Track it via delegated progress and delegated_run_id '{}'.",
            name, agent_type, delegated_run_id
        ));
    }
    ToolResult::success_data(result)
}

// ---------------------------------------------------------------------------
// Private dispatch methods
// ---------------------------------------------------------------------------

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
    /// Only downgrades for providers that have a known fast tier — otherwise
    /// inherits the parent model.
    fn resolve_fast_model(&self, ctx: &ToolContext, client: &AiClient) -> String {
        use crate::ai::providers::ProviderId;
        match client.provider_id() {
            ProviderId::Anthropic => "claude-haiku-4-5-20251001".to_string(),
            ProviderId::OpenAI => "gpt-5.4-mini".to_string(),
            // All other providers: use the same model as the parent.
            _ => self.resolve_model(ctx, client),
        }
    }

    // -----------------------------------------------------------------------
    // Explore
    // -----------------------------------------------------------------------

    async fn execute_explore(&self, params: Params, ctx: &ToolContext) -> ToolResult {
        let Some(project_dir) = ctx.project_dir.clone() else {
            return ToolResult::error(
                "No project directory is selected for this session. Stay in neutral mode for general inspection, or select/create a project directory before using explore.",
            );
        };

        let registry = match ctx.tool_registry.as_ref() {
            Some(r) => r.clone(),
            None => {
                return ToolResult::error(
                    "Tool registry not available in context. Cannot delegate exploration.",
                );
            }
        };

        let client = self.resolve_client(ctx);

        // Resolve scope
        let (working_dir, scope_label, scope_kind) = if let Some(ref scope) = params.scope {
            match resolve_explore_target(scope, &project_dir, "directory") {
                Ok(path) => (path, concise_target_label(scope, 0), "directory"),
                Err(_) => match resolve_explore_target(scope, &project_dir, "file") {
                    Ok(path) => {
                        let dir = path
                            .parent()
                            .map(Path::to_path_buf)
                            .unwrap_or_else(|| project_dir.clone());
                        (dir, concise_target_label(scope, 0), "file")
                    }
                    Err(err) => return ToolResult::error_with_code("invalid_explore_target", err),
                },
            }
        } else {
            (project_dir.clone(), "project".to_string(), "directory")
        };

        let delegated_run_id = Uuid::new_v4().to_string();
        let delegation_policy =
            DelegationPolicy::for_subagent_explore(ctx.permission_mode, ctx.subagent_max_turns);

        let target_scope = vec![delegated_scope(
            &scope_label,
            &working_dir,
            scope_kind,
            &project_dir,
        )];

        // Persist delegated run record
        let delegated_store = open_delegated_run_store(ctx);
        let resume_candidate = match (delegated_store.as_ref(), ctx.session_id.as_ref()) {
            (Some(store), Some(session_id)) => store
                .find_related_run(session_id, DelegatedRunRole::Explore, &target_scope)
                .ok()
                .flatten(),
            _ => None,
        };

        if let (Some(store), Some(session_id)) = (delegated_store.as_ref(), ctx.session_id.as_ref())
        {
            if let Err(err) = store.create_run(&DelegatedRunStartInput {
                delegated_run_id: delegated_run_id.clone(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: ctx.tool_use_id.clone(),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Created,
                provider: Some(client.provider_id().to_string()),
                model: Some(client.config().model.clone()),
                resumable: true,
                resumed_from_run_id: resume_candidate
                    .as_ref()
                    .map(|record| record.delegated_run_id.clone()),
                target_scope: target_scope.clone(),
            }) {
                warn!(delegated_run_id = %delegated_run_id, error = %err, "Failed to persist delegated explore run start");
            }
        }

        // Build task prompt with resume context if available
        let mut task_prompt = params.prompt.clone();
        if let Some(ref previous) = resume_candidate {
            if let Some(seed) = build_resume_seed(previous, &scope_label) {
                task_prompt = format!("{}\n\n{}", task_prompt, seed);
            }
        }

        let mut task = SubAgentTask::new("explorer-0", &task_prompt)
            .with_name(scope_label.clone())
            .with_working_dir(working_dir)
            .with_delegated_run_id(delegated_run_id.clone())
            .with_delegation_policy(delegation_policy.clone());
        if let Some(max_turns) = ctx.subagent_max_turns {
            task = task.with_max_turns(max_turns);
        }

        // Explore uses a fast/cheap model when available (e.g., Haiku on Anthropic).
        // Other providers inherit the parent model unchanged.
        let model = self.resolve_fast_model(ctx, &client);

        // Build project context for the subagent, with optional parent conversation brief
        let mut project_context =
            build_subagent_project_context(&ctx.working_dir, ctx.project_dir.as_deref());
        if let Some(ref parent_conversation) = ctx.parent_conversation {
            let brief = build_parent_context_brief(parent_conversation, 10);
            if !brief.is_empty() {
                project_context = format!("{}\n\n{}", brief, project_context);
            }
        }

        let cancellation_token = self.cancellation.child_token();
        let progress_tx = ctx.agent_progress_tx.clone();

        info!(
            delegated_run_id = %delegated_run_id,
            model = %model,
            scope = %scope_label,
            background = params.run_in_background.unwrap_or(false),
            "Agent tool (explore): starting single-agent exploration"
        );

        // ── Background mode ──────────────────────────────────────────
        if params.run_in_background.unwrap_or(false) {
            let bg_delegation_policy = delegation_policy.clone();
            let bg_delegated_run_id = delegated_run_id.clone();
            let bg_db_path = ctx.db_path.clone();

            tokio::spawn(async move {
                let result = execute_single_explorer(
                    client,
                    task,
                    registry,
                    bg_delegation_policy.clone(),
                    project_context,
                    model,
                    cancellation_token,
                    progress_tx.clone(),
                )
                .await;

                let artifact = build_single_agent_artifact(
                    &bg_delegated_run_id,
                    &result,
                    &bg_delegation_policy,
                );

                if let Some(ref db_path) = bg_db_path {
                    persist_single_agent_artifact_from_db_path(
                        db_path,
                        &bg_delegated_run_id,
                        &artifact,
                        true,
                        "explore",
                    );
                }

                emit_single_agent_completion(
                    &progress_tx,
                    &bg_delegated_run_id,
                    "explore",
                    &result,
                    &artifact.review_summary,
                );
            });

            return background_started_result(&delegated_run_id, "explore", params.name.as_deref());
        }

        // ── Synchronous mode (existing behavior) ─────────────────────
        let result = execute_single_explorer(
            client,
            task,
            registry,
            delegation_policy.clone(),
            project_context,
            model,
            cancellation_token,
            progress_tx,
        )
        .await;

        let artifact = build_single_agent_artifact(&delegated_run_id, &result, &delegation_policy);

        // Finalize the delegated run
        if let Some(store) = delegated_store.as_ref() {
            persist_single_agent_artifact(
                store,
                &delegated_run_id,
                &artifact,
                true,
                "Failed to persist delegated explore run final artifact",
            );
        }

        let warnings = build_single_agent_warnings(&result, "Exploration");

        ToolResult::success_data_with(artifact.payload, warnings, None, None)
    }

    // -----------------------------------------------------------------------
    // Plan
    // -----------------------------------------------------------------------

    async fn execute_plan(&self, params: Params, ctx: &ToolContext) -> ToolResult {
        if ctx.project_dir.is_none() {
            return ToolResult::error(
                "No project directory is selected for this session. Select or create a project directory before using plan.",
            );
        }

        let registry = match ctx.tool_registry.as_ref() {
            Some(r) => r.clone(),
            None => {
                return ToolResult::error(
                    "Tool registry not available in context. Cannot delegate planning.",
                );
            }
        };

        let client = self.resolve_client(ctx);

        let delegated_run_id = Uuid::new_v4().to_string();
        let delegation_policy =
            DelegationPolicy::for_subagent_plan(ctx.permission_mode, ctx.subagent_max_turns);

        let target_scope = vec![DelegatedRunScope {
            label: "project".to_string(),
            path: ".".to_string(),
            kind: "project".to_string(),
        }];

        let delegated_store = open_delegated_run_store(ctx);

        if let (Some(store), Some(session_id)) = (delegated_store.as_ref(), ctx.session_id.as_ref())
        {
            if let Err(err) = store.create_run(&DelegatedRunStartInput {
                delegated_run_id: delegated_run_id.clone(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: ctx.tool_use_id.clone(),
                role: DelegatedRunRole::Planner,
                stage: DelegatedRunStage::Created,
                provider: Some(client.provider_id().to_string()),
                model: Some(client.config().model.clone()),
                resumable: false,
                resumed_from_run_id: None,
                target_scope: target_scope.clone(),
            }) {
                warn!(delegated_run_id = %delegated_run_id, error = %err, "Failed to persist delegated plan run start");
            }
        }

        let mut task = SubAgentTask::new("planner-0", &params.prompt)
            .with_name("planner")
            .with_working_dir(ctx.working_dir.clone())
            .with_delegated_run_id(delegated_run_id.clone())
            .with_delegation_policy(delegation_policy.clone());
        if let Some(max_turns) = ctx.subagent_max_turns {
            task = task.with_max_turns(max_turns);
        }

        let model = self.resolve_model(ctx, &client);

        // Fresh project context (no parent conversation)
        let project_context =
            build_subagent_project_context(&ctx.working_dir, ctx.project_dir.as_deref());

        let cancellation_token = self.cancellation.child_token();
        let progress_tx = ctx.agent_progress_tx.clone();

        info!(
            delegated_run_id = %delegated_run_id,
            model = %model,
            background = params.run_in_background.unwrap_or(false),
            "Agent tool (plan): starting planning agent"
        );

        // ── Background mode ──────────────────────────────────────────
        if params.run_in_background.unwrap_or(false) {
            let bg_delegation_policy = delegation_policy.clone();
            let bg_delegated_run_id = delegated_run_id.clone();
            let bg_db_path = ctx.db_path.clone();

            tokio::spawn(async move {
                let config =
                    PlanConfig::new(registry, bg_delegation_policy.clone(), project_context).await;

                let result = execute_single_agent(
                    &client,
                    task,
                    config,
                    &model,
                    cancellation_token,
                    progress_tx.clone(),
                )
                .await;

                let artifact = build_single_agent_artifact(
                    &bg_delegated_run_id,
                    &result,
                    &bg_delegation_policy,
                );

                if let Some(ref db_path) = bg_db_path {
                    persist_single_agent_artifact_from_db_path(
                        db_path,
                        &bg_delegated_run_id,
                        &artifact,
                        false,
                        "plan",
                    );
                }

                emit_single_agent_completion(
                    &progress_tx,
                    &bg_delegated_run_id,
                    "plan",
                    &result,
                    &artifact.review_summary,
                );
            });

            return background_started_result(&delegated_run_id, "plan", params.name.as_deref());
        }

        // ── Synchronous mode (existing behavior) ─────────────────────
        let config = PlanConfig::new(registry, delegation_policy.clone(), project_context).await;

        let result = execute_single_agent(
            &client,
            task,
            config,
            &model,
            cancellation_token,
            progress_tx,
        )
        .await;

        let artifact = build_single_agent_artifact(&delegated_run_id, &result, &delegation_policy);

        if let Some(store) = delegated_store.as_ref() {
            persist_single_agent_artifact(
                store,
                &delegated_run_id,
                &artifact,
                false,
                "Failed to persist delegated plan run final artifact",
            );
        }

        let warnings = build_single_agent_warnings(&result, "Planning");

        ToolResult::success_data_with(artifact.payload, warnings, None, None)
    }

    // -----------------------------------------------------------------------
    // Verify
    // -----------------------------------------------------------------------

    async fn execute_verify(&self, params: Params, ctx: &ToolContext) -> ToolResult {
        if ctx.project_dir.is_none() {
            return ToolResult::error(
                "No project directory is selected for this session. Select or create a project directory before using verify.",
            );
        }

        let registry = match ctx.tool_registry.as_ref() {
            Some(r) => r.clone(),
            None => {
                return ToolResult::error(
                    "Tool registry not available in context. Cannot delegate verification.",
                );
            }
        };

        let client = self.resolve_client(ctx);

        let delegated_run_id = Uuid::new_v4().to_string();
        let delegation_policy =
            DelegationPolicy::for_subagent_verify(ctx.permission_mode, ctx.subagent_max_turns);

        let target_scope = vec![DelegatedRunScope {
            label: "project".to_string(),
            path: ".".to_string(),
            kind: "project".to_string(),
        }];

        let delegated_store = open_delegated_run_store(ctx);

        if let (Some(store), Some(session_id)) = (delegated_store.as_ref(), ctx.session_id.as_ref())
        {
            if let Err(err) = store.create_run(&DelegatedRunStartInput {
                delegated_run_id: delegated_run_id.clone(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: ctx.tool_use_id.clone(),
                role: DelegatedRunRole::Verifier,
                stage: DelegatedRunStage::Created,
                provider: Some(client.provider_id().to_string()),
                model: Some(client.config().model.clone()),
                resumable: false,
                resumed_from_run_id: None,
                target_scope: target_scope.clone(),
            }) {
                warn!(delegated_run_id = %delegated_run_id, error = %err, "Failed to persist delegated verify run start");
            }
        }

        let mut task = SubAgentTask::new("verifier-0", &params.prompt)
            .with_name("verifier")
            .with_working_dir(ctx.working_dir.clone())
            .with_delegated_run_id(delegated_run_id.clone())
            .with_delegation_policy(delegation_policy.clone());
        if let Some(max_turns) = ctx.subagent_max_turns {
            task = task.with_max_turns(max_turns);
        }

        let model = self.resolve_model(ctx, &client);

        // Fresh project context (no parent conversation)
        let project_context =
            build_subagent_project_context(&ctx.working_dir, ctx.project_dir.as_deref());

        let cancellation_token = self.cancellation.child_token();
        let progress_tx = ctx.agent_progress_tx.clone();

        info!(
            delegated_run_id = %delegated_run_id,
            model = %model,
            background = params.run_in_background.unwrap_or(false),
            "Agent tool (verify): starting verification agent"
        );

        // ── Background mode ──────────────────────────────────────────
        if params.run_in_background.unwrap_or(false) {
            let bg_delegation_policy = delegation_policy.clone();
            let bg_delegated_run_id = delegated_run_id.clone();
            let bg_db_path = ctx.db_path.clone();

            tokio::spawn(async move {
                let config =
                    VerifyConfig::new(registry, bg_delegation_policy.clone(), project_context)
                        .await;

                let result = execute_single_agent(
                    &client,
                    task,
                    config,
                    &model,
                    cancellation_token,
                    progress_tx.clone(),
                )
                .await;

                let artifact = build_single_agent_artifact(
                    &bg_delegated_run_id,
                    &result,
                    &bg_delegation_policy,
                );

                if let Some(ref db_path) = bg_db_path {
                    persist_single_agent_artifact_from_db_path(
                        db_path,
                        &bg_delegated_run_id,
                        &artifact,
                        false,
                        "verify",
                    );
                }

                emit_single_agent_completion(
                    &progress_tx,
                    &bg_delegated_run_id,
                    "verify",
                    &result,
                    &artifact.review_summary,
                );
            });

            return background_started_result(&delegated_run_id, "verify", params.name.as_deref());
        }

        // ── Synchronous mode (existing behavior) ─────────────────────
        let config = VerifyConfig::new(registry, delegation_policy.clone(), project_context).await;

        let result = execute_single_agent(
            &client,
            task,
            config,
            &model,
            cancellation_token,
            progress_tx,
        )
        .await;

        let artifact = build_single_agent_artifact(&delegated_run_id, &result, &delegation_policy);

        if let Some(store) = delegated_store.as_ref() {
            persist_single_agent_artifact(
                store,
                &delegated_run_id,
                &artifact,
                false,
                "Failed to persist delegated verify run final artifact",
            );
        }

        let warnings = build_single_agent_warnings(&result, "Verification");

        ToolResult::success_data_with(artifact.payload, warnings, None, None)
    }

    // -----------------------------------------------------------------------
    // Build
    // -----------------------------------------------------------------------

    async fn execute_build(&self, params: Params, ctx: &ToolContext) -> ToolResult {
        if ctx.project_dir.is_none() {
            return ToolResult::error(
                "No project directory is selected for this session. Create or choose a project directory, then promote the session before using build.",
            );
        }

        let client = self.resolve_client(ctx);

        // Create shared build context
        let context = Arc::new(SharedBuildContext::new());

        // Merge conventions: project settings as base, user params override/extend
        let project_settings = ctx
            .project_dir
            .as_ref()
            .map(|p| ProjectSettings::load(p))
            .unwrap_or_default();
        let mut conventions = project_settings.conventions.unwrap_or_default();
        if let Some(user_conventions) = &params.conventions {
            conventions.extend(user_conventions.iter().cloned());
        }
        if !conventions.is_empty() {
            context.set_conventions(conventions);
        }

        // Smart concurrency default: match component count, clamped to reasonable range
        let num_components = params.components.as_ref().map(|c| c.len()).unwrap_or(1);
        let concurrency = params.max_concurrency.unwrap_or_else(|| {
            // Default: match component count, capped at reasonable limit
            num_components.clamp(2, 10)
        });

        // Build tasks - all use Opus for high-quality code generation
        let mut tasks: Vec<SubAgentTask> = Vec::new();
        let delegated_run_id = Uuid::new_v4().to_string();
        let delegation_policy =
            DelegationPolicy::for_subagent_build(ctx.permission_mode, ctx.subagent_max_turns);
        let delegated_store = open_delegated_run_store(ctx);
        let mut target_scope = Vec::new();

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
                    .with_delegated_run_id(delegated_run_id.clone())
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

                target_scope.push(DelegatedRunScope {
                    label: name.to_string(),
                    path: component.clone(),
                    kind: "component".to_string(),
                });
                tasks.push(task);
            }
        } else {
            // Single builder for the whole task
            target_scope.push(DelegatedRunScope {
                label: "main".to_string(),
                path: ".".to_string(),
                kind: "project".to_string(),
            });
            let mut task = SubAgentTask::new("builder-main", params.prompt.clone())
                .with_name("main")
                .with_working_dir(ctx.working_dir.clone())
                .with_delegated_run_id(delegated_run_id.clone())
                .with_delegation_policy(delegation_policy.clone());
            if let Some(max_turns) = ctx.subagent_max_turns {
                task = task.with_max_turns(max_turns);
            }
            tasks.push(task);
        }

        if let (Some(store), Some(session_id)) = (delegated_store.as_ref(), ctx.session_id.as_ref())
        {
            if let Err(err) = store.create_run(&DelegatedRunStartInput {
                delegated_run_id: delegated_run_id.clone(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: ctx.tool_use_id.clone(),
                role: DelegatedRunRole::Build,
                stage: DelegatedRunStage::Created,
                provider: Some(client.provider_id().to_string()),
                model: Some(client.config().model.clone()),
                resumable: true,
                resumed_from_run_id: None,
                target_scope,
            }) {
                warn!(delegated_run_id = %delegated_run_id, error = %err, "Failed to persist delegated build run start");
            }
        }

        info!("Agent tool (build): Created {} builder tasks", tasks.len());
        for (i, task) in tasks.iter().enumerate() {
            debug!("Builder {}: id={}, name={}", i, task.id, task.name);
        }

        // Create pool and execute with build context
        let pool = SubAgentPool::new(client, self.cancellation.clone())
            .with_concurrency(concurrency)
            .with_override_model(ctx.current_model.clone());

        info!(
            "Agent tool (build): Starting builder pool with max_concurrency={} (components={}), background={}",
            concurrency, num_components, params.run_in_background.unwrap_or(false)
        );

        // ── Background mode ──────────────────────────────────────────
        if params.run_in_background.unwrap_or(false) {
            let bg_delegated_run_id = delegated_run_id.clone();
            let bg_db_path = ctx.db_path.clone();
            let bg_delegation_policy = delegation_policy.clone();
            let progress_tx = ctx.agent_progress_tx.clone();

            tokio::spawn(async move {
                // Keep a clone for the completion event after execute_builders consumes the tx
                let completion_tx = progress_tx.clone();

                let results = if let Some(progress_tx) = progress_tx {
                    pool.execute_builders(tasks, context.clone(), progress_tx)
                        .await
                } else {
                    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                    pool.execute_builders(tasks, context.clone(), tx).await
                };

                // Build the finalization payload (same logic as synchronous path)
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

                let failed_builders = errors.len();
                let successful_builders = results.len().saturating_sub(failed_builders);
                let outcome =
                    classify_build_outcome(results.len(), failed_builders, stats.files_modified);
                let investigation_summary = build_investigation_summary(
                    successful_builders,
                    failed_builders,
                    stats.files_modified,
                    stats.lines_added,
                    stats.lines_removed,
                );

                let payload = json!({
                    "delegated_run_id": bg_delegated_run_id,
                    "message": format!(
                        "Build completed: {} builders, {} turns, +{} -{} lines across {} files",
                        results.len(), total_turns, stats.lines_added, stats.lines_removed, stats.files_modified,
                    ),
                    "investigation_summary": investigation_summary,
                    "outcome": outcome,
                    "builder_count": results.len(),
                    "successful_agents": successful_builders,
                    "failed_agents": failed_builders,
                    "total_turns": total_turns,
                    "total_duration_ms": total_duration_ms,
                    "files_examined": unique_files,
                    "builders": builders,
                    "lines_added": stats.lines_added,
                    "lines_removed": stats.lines_removed,
                    "files_modified": stats.files_modified,
                    "errors": errors,
                    "delegation_policy": bg_delegation_policy.audit_json(),
                });

                if let Some(ref db_path) = bg_db_path {
                    match Database::new(db_path) {
                        Ok(db) => {
                            let store = DelegatedRunStore::new(db);
                            let final_stage = match outcome {
                                "success" => DelegatedRunStage::Complete,
                                "partial" => DelegatedRunStage::Degraded,
                                _ => DelegatedRunStage::Failed,
                            };
                            if let Err(err) = store.finalize_run(
                                &bg_delegated_run_id,
                                final_stage,
                                &payload,
                                Some(&investigation_summary),
                                failed_builders > 0,
                            ) {
                                warn!(
                                    delegated_run_id = %bg_delegated_run_id,
                                    error = %err,
                                    "Background build: failed to persist final artifact"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                delegated_run_id = %bg_delegated_run_id,
                                error = %e,
                                "Failed to open database for background build finalization"
                            );
                        }
                    }
                }

                // Emit completion event so the parent sees the result
                if let Some(ref tx) = completion_tx {
                    let build_success = failed_builders == 0 && stats.files_modified > 0;
                    let status = if build_success {
                        AgentProgressStatus::Complete
                    } else {
                        AgentProgressStatus::Failed
                    };
                    if tx
                        .send(AgentProgress {
                            delegated_run_id: Some(bg_delegated_run_id.clone()),
                            task_id: "build".to_string(),
                            name: "build".to_string(),
                            status,
                            tool_count: 0,
                            tokens: 0,
                            current_action: None,
                            completion_summary: Some(investigation_summary),
                            lines_added: stats.lines_added,
                            lines_removed: stats.lines_removed,
                            completed_plan_task: None,
                        })
                        .is_err()
                    {
                        debug!("Background build progress channel disconnected (parent returned)");
                    }
                }
            });

            return background_started_result(&delegated_run_id, "build", params.name.as_deref());
        }

        // ── Synchronous mode (existing behavior) ─────────────────────

        // Execute builders with progress channel if available
        let results = if let Some(ref progress_tx) = ctx.agent_progress_tx {
            pool.execute_builders(tasks, context.clone(), progress_tx.clone())
                .await
        } else {
            // Fallback: create a dummy channel and discard progress
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            pool.execute_builders(tasks, context.clone(), tx).await
        };

        info!(
            "Agent tool (build): Builder pool returned {} results",
            results.len()
        );

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
        let failed_builders = errors.len();
        let successful_builders = results.len().saturating_sub(failed_builders);
        let outcome = classify_build_outcome(results.len(), failed_builders, stats.files_modified);
        let confidence = build_confidence(failed_builders, stats.files_modified);
        let investigation_summary = build_investigation_summary(
            successful_builders,
            failed_builders,
            stats.files_modified,
            stats.lines_added,
            stats.lines_removed,
        );
        let coverage_gap_notice = build_coverage_gap_notice(&errors);

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

        let payload = json!({
            "delegated_run_id": delegated_run_id,
            "message": message,
            "investigation_summary": investigation_summary,
            "confidence": confidence,
            "outcome": outcome,
            "outcome_reason": if failed_builders == 0 { "usable_evidence" } else { "mixed" },
            "builder_count": results.len(),
            "agent_count": results.len(),
            "successful_agents": successful_builders,
            "usable_agents": successful_builders,
            "degraded_agents": 0,
            "failed_agents": failed_builders,
            "total_turns": total_turns,
            "total_duration_ms": total_duration_ms,
            "paths_examined": unique_files,
            "paths_examined_count": unique_files.len(),
            "files_examined": unique_files,
            "builders": builders,
            "lines_added": stats.lines_added,
            "lines_removed": stats.lines_removed,
            "files_modified": stats.files_modified,
            "lock_contentions": stats.lock_contentions,
            "total_lock_wait_ms": stats.total_lock_wait_ms,
            "high_contention_files": high_contention_files,
            "coverage_gap_notice": coverage_gap_notice,
            "errors": errors,
            "delegation_policy": delegation_policy.audit_json(),
        });

        if let Some(store) = delegated_store.as_ref() {
            let final_stage = match outcome {
                "success" => DelegatedRunStage::Complete,
                "partial" => DelegatedRunStage::Degraded,
                _ => DelegatedRunStage::Failed,
            };
            if let Err(err) = store.finalize_run(
                &delegated_run_id,
                final_stage,
                &payload,
                Some(&investigation_summary),
                failed_builders > 0,
            ) {
                warn!(delegated_run_id = %delegated_run_id, error = %err, "Failed to persist delegated build run final artifact");
            }
        }

        ToolResult::success_data(payload)
    }
}

// ---------------------------------------------------------------------------
// Helper functions (ported from explore.rs and build.rs)
// ---------------------------------------------------------------------------

struct SingleAgentArtifact {
    payload: Value,
    review_summary: String,
    final_stage: DelegatedRunStage,
}

fn build_single_agent_artifact(
    delegated_run_id: &str,
    result: &SubAgentResult,
    delegation_policy: &DelegationPolicy,
) -> SingleAgentArtifact {
    let review_summary = truncate_utf8(&result.output, 500).to_string();
    let payload = json!({
        "delegated_run_id": delegated_run_id,
        "findings": result.output,
        "files_examined": result.files_examined,
        "files_examined_count": result.files_examined.len(),
        "duration_ms": result.duration_ms,
        "turns_used": result.turns_used,
        "success": result.success,
        "outcome": if result.success { "success" } else { "failed" },
        "delegation_policy": delegation_policy.audit_json(),
    });

    SingleAgentArtifact {
        payload,
        review_summary,
        final_stage: single_agent_final_stage(result.success),
    }
}

fn single_agent_final_stage(success: bool) -> DelegatedRunStage {
    if success {
        DelegatedRunStage::Complete
    } else {
        DelegatedRunStage::Failed
    }
}

fn persist_single_agent_artifact(
    store: &DelegatedRunStore,
    delegated_run_id: &str,
    artifact: &SingleAgentArtifact,
    resumable: bool,
    error_message: &str,
) {
    if let Err(err) = store.finalize_run(
        delegated_run_id,
        artifact.final_stage,
        &artifact.payload,
        Some(&artifact.review_summary),
        resumable,
    ) {
        warn!(delegated_run_id = %delegated_run_id, error = %err, "{}", error_message);
    }
}

fn persist_single_agent_artifact_from_db_path(
    db_path: &Path,
    delegated_run_id: &str,
    artifact: &SingleAgentArtifact,
    resumable: bool,
    agent_type: &str,
) {
    match Database::new(db_path) {
        Ok(db) => {
            let store = DelegatedRunStore::new(db);
            let error_message = format!(
                "Background {}: failed to persist final artifact",
                agent_type
            );
            persist_single_agent_artifact(
                &store,
                delegated_run_id,
                artifact,
                resumable,
                &error_message,
            );
        }
        Err(err) => {
            tracing::error!(
                delegated_run_id = %delegated_run_id,
                error = %err,
                "Failed to open database for background {} finalization",
                agent_type
            );
        }
    }
}

fn emit_single_agent_completion(
    progress_tx: &Option<mpsc::UnboundedSender<AgentProgress>>,
    delegated_run_id: &str,
    agent_type: &str,
    result: &SubAgentResult,
    review_summary: &str,
) {
    if let Some(tx) = progress_tx {
        let status = if result.success {
            AgentProgressStatus::Complete
        } else {
            AgentProgressStatus::Failed
        };
        if tx
            .send(AgentProgress {
                delegated_run_id: Some(delegated_run_id.to_string()),
                task_id: result.task_id.clone(),
                name: agent_type.to_string(),
                status,
                tool_count: 0,
                tokens: 0,
                current_action: None,
                completion_summary: Some(review_summary.to_string()),
                lines_added: 0,
                lines_removed: 0,
                completed_plan_task: None,
            })
            .is_err()
        {
            debug!(
                "Background {} progress channel disconnected (parent returned)",
                agent_type
            );
        }
    }
}

fn build_single_agent_warnings(result: &SubAgentResult, action: &str) -> Vec<String> {
    if result.success {
        return Vec::new();
    }

    match result.error.as_deref() {
        Some(err) => vec![format!("{} failed: {}", action, err)],
        None => vec![format!("{} completed without usable results.", action)],
    }
}

fn concise_target_label(target: &str, index: usize) -> String {
    let trimmed = target.trim_matches('/');
    if trimmed.is_empty() {
        return format!("target-{}", index + 1);
    }

    let parts = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if parts.len() >= 2 {
        return format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
    }

    parts[0].to_string()
}

fn resolve_explore_target(target: &str, project_dir: &Path, kind: &str) -> Result<PathBuf, String> {
    let candidate = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        project_dir.join(target)
    };

    let canonical = candidate.canonicalize().map_err(|_| {
        format!(
            "Missing explore target: {} '{}' does not exist under project root",
            kind, target
        )
    })?;

    let project_root = project_dir
        .canonicalize()
        .map_err(|err| format!("Invalid project root '{}': {}", project_dir.display(), err))?;

    if !canonical.starts_with(&project_root) {
        return Err(format!(
            "Invalid explore target: {} '{}' is outside project root",
            kind, target
        ));
    }

    match kind {
        "directory" if !canonical.is_dir() => Err(format!(
            "Invalid explore target: directory '{}' resolved to a non-directory path",
            target
        )),
        "file" if !canonical.is_file() => Err(format!(
            "Invalid explore target: file '{}' resolved to a non-file path",
            target
        )),
        _ => Ok(canonical),
    }
}

fn relative_label(path: &Path, project_dir: &Path) -> String {
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn delegated_scope(label: &str, path: &Path, kind: &str, project_dir: &Path) -> DelegatedRunScope {
    DelegatedRunScope {
        label: label.to_string(),
        path: relative_label(path, project_dir),
        kind: kind.to_string(),
    }
}

fn open_delegated_run_store(ctx: &ToolContext) -> Option<DelegatedRunStore> {
    let db_path = ctx.db_path.as_ref()?;
    match Database::new(db_path) {
        Ok(db) => Some(DelegatedRunStore::new(db)),
        Err(error) => {
            warn!(
                session_id = ?ctx.session_id,
                tool_use_id = ?ctx.tool_use_id,
                db_path = %db_path.display(),
                error = %error,
                "Failed to open delegated run store"
            );
            None
        }
    }
}

fn build_resume_seed(previous: &DelegatedRunRecord, target_label: &str) -> Option<String> {
    let artifact = previous.artifact.as_ref()?;
    let agent_artifact = artifact
        .get("agents")
        .and_then(|value| value.as_array())
        .and_then(|agents| {
            agents.iter().find(|entry| {
                entry
                    .get("agent")
                    .and_then(|value| value.as_str())
                    .is_some_and(|agent| agent == target_label)
            })
        });

    let summary = agent_artifact
        .and_then(|entry| entry.get("summary").and_then(|value| value.as_str()))
        .or(previous.human_review.as_deref())
        .or_else(|| {
            artifact
                .get("investigation_summary")
                .and_then(|value| value.as_str())
        })?;

    let paths = agent_artifact
        .and_then(|entry| {
            entry
                .get("paths_examined")
                .and_then(|value| value.as_array())
        })
        .map(|paths| {
            paths
                .iter()
                .filter_map(|value| value.as_str())
                .take(8)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.trim().is_empty());

    let gaps = agent_artifact
        .and_then(|entry| entry.get("gaps").and_then(|value| value.as_array()))
        .map(|gaps| {
            gaps.iter()
                .filter_map(|value| value.as_str())
                .take(4)
                .collect::<Vec<_>>()
                .join("; ")
        })
        .filter(|value| !value.trim().is_empty());

    let mut lines = vec![format!(
        "Previous delegated run {} already covered this target.",
        previous.delegated_run_id
    )];
    lines.push(format!(
        "Reuse and extend this prior evidence: {}",
        summary.trim()
    ));
    if let Some(paths) = paths {
        lines.push(format!("Previously examined paths: {}", paths));
    }
    if let Some(gaps) = gaps {
        lines.push(format!("Previously identified gaps: {}", gaps));
    }
    lines.push(
        "Do not remap the same directory structure from scratch unless it is required to close a concrete gap."
            .to_string(),
    );
    Some(lines.join("\n"))
}

/// Build a brief summary of the parent conversation for context injection.
///
/// Extracts the last `max_turns` user/assistant messages and formats them as a
/// `[PARENT CONTEXT]` block that the sub-agent can use for upstream awareness.
fn build_parent_context_brief(conversation: &[ModelMessage], max_turns: usize) -> String {
    let relevant: Vec<_> = conversation
        .iter()
        .filter(|msg| msg.role == Role::User || msg.role == Role::Assistant)
        .collect();

    let start = relevant.len().saturating_sub(max_turns);
    let window = &relevant[start..];

    if window.is_empty() {
        return String::new();
    }

    let mut lines = vec!["[PARENT CONTEXT]".to_string()];
    for msg in window {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            _ => continue,
        };

        let text = msg
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");

        if text.is_empty() {
            continue;
        }

        let truncated = if text.len() > 200 {
            let mut end = 200;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &text[..end])
        } else {
            text
        };

        lines.push(format!("{}: {}", role_label, truncated));
    }
    lines.push("[/PARENT CONTEXT]".to_string());

    lines.join("\n")
}

// Build-specific helper functions

fn classify_build_outcome(
    results_len: usize,
    failed_agents: usize,
    modified_files: usize,
) -> &'static str {
    if results_len == 0 || modified_files == 0 {
        return "failed";
    }
    if failed_agents == 0 {
        return "success";
    }
    "partial"
}

fn build_confidence(failed_agents: usize, modified_files: usize) -> &'static str {
    if modified_files == 0 {
        return "low";
    }
    if failed_agents == 0 {
        return "high";
    }
    "medium"
}

fn build_investigation_summary(
    successful_builders: usize,
    failed_builders: usize,
    modified_files: usize,
    lines_added: usize,
    lines_removed: usize,
) -> String {
    let mut parts = vec![format!(
        "Delegated build completed with {} successful builders across {} modified files.",
        successful_builders, modified_files
    )];
    parts.push(format!(
        "Net code movement: +{} / -{} lines.",
        lines_added, lines_removed
    ));
    if failed_builders > 0 {
        parts.push(format!(
            "{} builders reported errors and should be reviewed before accepting the build output.",
            failed_builders
        ));
    }
    parts.join(" ")
}

fn build_coverage_gap_notice(errors: &[String]) -> Option<String> {
    if errors.is_empty() {
        None
    } else {
        Some(format!(
            "These builders failed or reported issues: {}",
            errors.join(" | ")
        ))
    }
}

/// Truncate a string to at most `max_bytes` bytes without splitting a UTF-8
/// character. Returns the longest prefix that fits within the byte budget.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::{
        build_parent_context_brief, build_single_agent_artifact, build_single_agent_warnings,
        concise_target_label, emit_single_agent_completion, open_delegated_run_store,
        resolve_explore_target, truncate_utf8,
    };
    use crate::agent::subagent::{AgentProgressStatus, SubAgentResult};
    use crate::agent::DelegatedRunStage;
    use crate::ai::types::{Content, ModelMessage, Role};
    use crate::storage::WorkspaceMode;
    use crate::tools::registry::{DelegationPolicy, PermissionMode};
    use crate::tools::ToolContext;
    use crate::Database;
    use std::fs;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn sample_result(success: bool, error: Option<&str>) -> SubAgentResult {
        SubAgentResult {
            task_id: "agent-1".to_string(),
            agent_name: "planner".to_string(),
            delegated_run_id: Some("run-123".to_string()),
            success,
            output: "Found the relevant code paths".to_string(),
            files_examined: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
            duration_ms: 1200,
            turns_used: 3,
            error: error.map(ToString::to_string),
            policy_violations: vec![],
        }
    }

    #[test]
    fn concise_target_label_uses_stable_readable_segments() {
        assert_eq!(
            concise_target_label("crates/krusty-core/src", 0),
            "krusty-core/src"
        );
        assert_eq!(concise_target_label("README.md", 1), "README.md");
        assert_eq!(concise_target_label("", 2), "target-3");
    }

    #[test]
    fn resolve_explore_target_rejects_missing_or_outside_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("src")).expect("project dirs");

        let inside = resolve_explore_target("src", &project, "directory").expect("inside target");
        assert!(inside.ends_with("src"));

        let missing = resolve_explore_target("missing", &project, "directory")
            .expect_err("missing target should fail");
        assert!(missing.contains("Missing explore target"));

        let outside = resolve_explore_target("../outside", &project, "directory")
            .expect_err("outside target should fail");
        assert!(
            outside.contains("outside project root") || outside.contains("Missing explore target")
        );
    }

    #[test]
    fn parent_context_brief_extracts_last_turns() {
        let conversation = vec![
            ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: "First question".to_string(),
                }],
            },
            ModelMessage {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "First answer".to_string(),
                }],
            },
            ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: "Second question".to_string(),
                }],
            },
        ];

        let brief = build_parent_context_brief(&conversation, 2);
        assert!(brief.contains("[PARENT CONTEXT]"));
        assert!(brief.contains("[/PARENT CONTEXT]"));
        assert!(brief.contains("First answer"));
        assert!(brief.contains("Second question"));
    }

    #[test]
    fn parent_context_brief_truncates_long_messages() {
        let long_text = "x".repeat(300);
        let conversation = vec![ModelMessage {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: long_text.clone(),
            }],
        }];

        let brief = build_parent_context_brief(&conversation, 10);
        assert!(brief.contains("..."));
        assert!(brief.len() < long_text.len());
    }

    #[test]
    fn parent_context_brief_empty_on_no_messages() {
        let brief = build_parent_context_brief(&[], 10);
        assert!(brief.is_empty());
    }

    #[test]
    fn open_delegated_run_store_returns_none_without_db_path() {
        let ctx = ToolContext::default();

        assert!(open_delegated_run_store(&ctx).is_none());
    }

    #[test]
    fn open_delegated_run_store_opens_valid_database() {
        let temp_dir = TempDir::new().expect("tempdir");
        let db_path = temp_dir.path().join("delegated.db");
        Database::new(&db_path).expect("database");

        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            workspace_mode: WorkspaceMode::Neutral,
            db_path: Some(db_path),
            ..Default::default()
        };

        assert!(open_delegated_run_store(&ctx).is_some());
    }

    #[test]
    fn open_delegated_run_store_returns_none_for_unopenable_database() {
        let temp_dir = TempDir::new().expect("tempdir");
        let invalid_db_path = temp_dir.path().join("not-a-db-file");
        std::fs::create_dir_all(&invalid_db_path).expect("directory path should exist");
        let ctx = ToolContext {
            working_dir: temp_dir.path().to_path_buf(),
            workspace_mode: WorkspaceMode::Neutral,
            db_path: Some(invalid_db_path),
            session_id: Some("session-1".to_string()),
            tool_use_id: Some("tool-1".to_string()),
            ..Default::default()
        };

        assert!(open_delegated_run_store(&ctx).is_none());
    }

    #[test]
    fn single_agent_artifact_keeps_payload_shape() {
        let result = sample_result(true, None);
        let policy = DelegationPolicy::for_subagent_plan(PermissionMode::Autonomous, Some(8));

        let artifact = build_single_agent_artifact("run-123", &result, &policy);

        assert_eq!(artifact.review_summary, "Found the relevant code paths");
        assert_eq!(artifact.final_stage, DelegatedRunStage::Complete);
        assert_eq!(artifact.payload["delegated_run_id"], "run-123");
        assert_eq!(
            artifact.payload["findings"],
            "Found the relevant code paths"
        );
        assert_eq!(artifact.payload["files_examined_count"], 2);
        assert_eq!(artifact.payload["success"], true);
        assert_eq!(artifact.payload["outcome"], "success");
        assert_eq!(
            artifact.payload["delegation_policy"]["surface"],
            "subagent_plan"
        );
    }

    #[test]
    fn single_agent_warnings_preserve_failure_wording() {
        let errored = sample_result(false, Some("tool timeout"));
        assert_eq!(
            build_single_agent_warnings(&errored, "Verification"),
            vec!["Verification failed: tool timeout".to_string()]
        );

        let empty = sample_result(false, None);
        assert_eq!(
            build_single_agent_warnings(&empty, "Planning"),
            vec!["Planning completed without usable results.".to_string()]
        );

        let ok = sample_result(true, Some("ignored"));
        assert!(build_single_agent_warnings(&ok, "Exploration").is_empty());
    }

    #[test]
    fn emit_single_agent_completion_reports_summary_and_status() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let result = sample_result(false, Some("lint failed"));

        emit_single_agent_completion(&Some(tx), "run-999", "verify", &result, "short summary");

        let progress = rx.try_recv().expect("completion event");
        assert_eq!(progress.delegated_run_id.as_deref(), Some("run-999"));
        assert_eq!(progress.task_id, "agent-1");
        assert_eq!(progress.name, "verify");
        assert_eq!(progress.status, AgentProgressStatus::Failed);
        assert_eq!(
            progress.completion_summary.as_deref(),
            Some("short summary")
        );
    }

    #[test]
    fn truncate_utf8_respects_char_boundaries() {
        // ASCII: truncation at exact byte offset is fine
        assert_eq!(truncate_utf8("hello world", 5), "hello");
        // Multi-byte: U+00E9 (e-acute) is 2 bytes in UTF-8
        let s = "caf\u{00e9}!"; // "cafe!" with accented e — 6 bytes total
        assert_eq!(truncate_utf8(s, 10), s); // within budget
        assert_eq!(truncate_utf8(s, 4), "caf"); // byte 4 is mid-char, backs up to 3
        assert_eq!(truncate_utf8(s, 5), "caf\u{00e9}"); // byte 5 is right after the char
                                                        // Empty and zero budget
        assert_eq!(truncate_utf8("", 0), "");
        assert_eq!(truncate_utf8("abc", 0), "");
    }
}
