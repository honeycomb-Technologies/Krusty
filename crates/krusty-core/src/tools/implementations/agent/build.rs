use std::sync::Arc;

use serde_json::json;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::agent::subagent::{
    build_context::SharedBuildContext, AgentProgress, AgentProgressStatus, SubAgentPool,
    SubAgentTask,
};
use crate::agent::DelegatedRunStage;
use crate::storage::{
    Database, DelegatedRunRole, DelegatedRunScope, DelegatedRunStartInput, DelegatedRunStore,
    ProjectSettings,
};
use crate::tools::registry::DelegationPolicy;
use crate::tools::{ToolContext, ToolResult};

use super::{
    background_started_result, build_confidence, build_coverage_gap_notice,
    build_investigation_summary, classify_build_outcome, open_delegated_run_store, AgentTool,
    Params,
};

impl AgentTool {
    // Build
    // -----------------------------------------------------------------------

    pub(super) async fn execute_build(&self, params: Params, ctx: &ToolContext) -> ToolResult {
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
