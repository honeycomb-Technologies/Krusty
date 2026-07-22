use std::path::Path;

use tracing::{info, warn};
use uuid::Uuid;

use crate::agent::agent_types::{PlanConfig, VerifyConfig};
use crate::agent::context::build_subagent_project_context;
use crate::agent::subagent::{execute_single_agent, execute_single_explorer, SubAgentTask};
use crate::agent::DelegatedRunStage;
use crate::storage::{DelegatedRunRole, DelegatedRunScope, DelegatedRunStartInput};
use crate::tools::registry::DelegationPolicy;
use crate::tools::{ToolContext, ToolResult};

use super::{
    background_started_result, build_parent_context_brief, build_resume_seed,
    build_single_agent_artifact, build_single_agent_warnings, concise_target_label,
    delegated_scope, emit_single_agent_completion, open_delegated_run_store,
    persist_single_agent_artifact, persist_single_agent_artifact_from_db_path,
    resolve_explore_target, AgentTool, Params,
};

impl AgentTool {
    // -----------------------------------------------------------------------
    // Explore
    // -----------------------------------------------------------------------

    pub(super) async fn execute_explore(&self, params: Params, ctx: &ToolContext) -> ToolResult {
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
            DelegationPolicy::for_subagent_explore(ctx.permission_mode, params.max_turns)
                .with_execution_tool_allowlist(ctx.execution_tool_allowlist.as_ref());

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

        let inherited_sandbox = ctx
            .sandbox_root
            .clone()
            .unwrap_or_else(|| working_dir.clone());
        let mut task = SubAgentTask::new("explorer-0", &task_prompt)
            .with_name(scope_label.clone())
            .with_working_dir(working_dir)
            .with_sandbox_root(inherited_sandbox)
            .with_delegated_run_id(delegated_run_id.clone())
            .with_delegation_policy(delegation_policy.clone())
            .with_process_context(
                ctx.process_registry.clone(),
                ctx.user_id.clone(),
                ctx.session_id.clone(),
            )
            .with_provider_call_trace(ctx.provider_call_trace.clone());
        if let Some(max_turns) = params.max_turns {
            task = task.with_max_turns(max_turns);
        }

        // Explore uses a fast/cheap model when available (e.g., Haiku on Anthropic).
        // Other providers inherit the parent model unchanged.
        let model = self.resolve_fast_model(ctx, &client);

        // Build project context for the subagent, with optional parent conversation brief
        let mut project_context =
            build_subagent_project_context(&ctx.working_dir, ctx.project_dir.as_deref());
        if !params.parent_context_applied {
            if let Some(ref parent_conversation) = ctx.parent_conversation {
                let brief = build_parent_context_brief(parent_conversation, 10);
                if !brief.is_empty() {
                    project_context = format!("{}\n\n{}", brief, project_context);
                }
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
            let bg_runtime = self.runtime.clone();
            let mailbox = bg_runtime.register(
                bg_delegated_run_id.clone(),
                params.name.as_deref().unwrap_or("explore"),
                cancellation_token.clone(),
            );
            task = task.with_mailbox(mailbox);

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
                bg_runtime.finish(&bg_delegated_run_id, result.success);
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

    pub(super) async fn execute_plan(&self, params: Params, ctx: &ToolContext) -> ToolResult {
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
            DelegationPolicy::for_subagent_plan(ctx.permission_mode, params.max_turns)
                .with_execution_tool_allowlist(ctx.execution_tool_allowlist.as_ref());

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
            .with_sandbox_root(
                ctx.sandbox_root
                    .clone()
                    .unwrap_or_else(|| ctx.working_dir.clone()),
            )
            .with_delegated_run_id(delegated_run_id.clone())
            .with_delegation_policy(delegation_policy.clone())
            .with_process_context(
                ctx.process_registry.clone(),
                ctx.user_id.clone(),
                ctx.session_id.clone(),
            )
            .with_provider_call_trace(ctx.provider_call_trace.clone());
        if let Some(max_turns) = params.max_turns {
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
            let bg_runtime = self.runtime.clone();
            let mailbox = bg_runtime.register(
                bg_delegated_run_id.clone(),
                params.name.as_deref().unwrap_or("plan"),
                cancellation_token.clone(),
            );
            task = task.with_mailbox(mailbox);

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
                bg_runtime.finish(&bg_delegated_run_id, result.success);
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

    pub(super) async fn execute_verify(&self, params: Params, ctx: &ToolContext) -> ToolResult {
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
            DelegationPolicy::for_subagent_verify(ctx.permission_mode, params.max_turns)
                .with_execution_tool_allowlist(ctx.execution_tool_allowlist.as_ref());

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
            .with_sandbox_root(
                ctx.sandbox_root
                    .clone()
                    .unwrap_or_else(|| ctx.working_dir.clone()),
            )
            .with_delegated_run_id(delegated_run_id.clone())
            .with_delegation_policy(delegation_policy.clone())
            .with_process_context(
                ctx.process_registry.clone(),
                ctx.user_id.clone(),
                ctx.session_id.clone(),
            )
            .with_provider_call_trace(ctx.provider_call_trace.clone());
        if let Some(max_turns) = params.max_turns {
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
            let bg_runtime = self.runtime.clone();
            let mailbox = bg_runtime.register(
                bg_delegated_run_id.clone(),
                params.name.as_deref().unwrap_or("verify"),
                cancellation_token.clone(),
            );
            task = task.with_mailbox(mailbox);

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
                bg_runtime.finish(&bg_delegated_run_id, result.success);
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
}
