use std::path::Path;

use tracing::{info, warn};
use uuid::Uuid;

use crate::agent::context::build_subagent_project_context;
use crate::agent::subagent::{execute_single_child, AgentCapability, SubAgentTask};
use crate::agent::DelegatedRunStage;
use crate::storage::{
    DelegatedRunCreateOutcome, DelegatedRunLease, DelegatedRunRole, DelegatedRunScope,
    DelegatedRunStartInput,
};
use crate::tools::registry::DelegationPolicy;
use crate::tools::{ToolContext, ToolResult};

use super::{
    background_started_result, build_parent_context_brief, build_resume_seed,
    build_single_agent_artifact, build_single_agent_warnings, concise_target_label,
    delegated_persistence_error, delegated_scope, delegated_workspace_scope,
    emit_single_agent_completion, existing_continuation_error, notify_child_completion,
    open_delegated_run_store, persist_background_single_agent_artifact,
    persist_single_agent_artifact, resolve_explore_target, AgentTool, Params,
};

struct ResolvedChildTarget {
    working_dir: std::path::PathBuf,
    target_path: std::path::PathBuf,
    label: String,
    kind: &'static str,
}

fn normalize_persisted_target(value: &str) -> &str {
    let trimmed = value.trim().trim_end_matches('/');
    let normalized = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if normalized == "." {
        ""
    } else {
        normalized
    }
}

fn persisted_target_matches(previous: &[DelegatedRunScope], current: &[DelegatedRunScope]) -> bool {
    let previous_workspaces = previous
        .iter()
        .filter(|scope| scope.kind == "workspace")
        .collect::<Vec<_>>();
    let current_workspaces = current
        .iter()
        .filter(|scope| scope.kind == "workspace")
        .collect::<Vec<_>>();
    let ([previous_workspace], [current_workspace]) = (
        previous_workspaces.as_slice(),
        current_workspaces.as_slice(),
    ) else {
        return false;
    };
    if normalize_persisted_target(&previous_workspace.path)
        != normalize_persisted_target(&current_workspace.path)
    {
        return false;
    }

    let previous_primary = previous
        .iter()
        .filter(|scope| !matches!(scope.kind.as_str(), "workspace" | "component"))
        .collect::<Vec<_>>();
    let current_primary = current
        .iter()
        .filter(|scope| !matches!(scope.kind.as_str(), "workspace" | "component"))
        .collect::<Vec<_>>();
    let ([previous_primary], [current_primary]) =
        (previous_primary.as_slice(), current_primary.as_slice())
    else {
        return false;
    };
    let kind_matches = previous_primary.kind == current_primary.kind
        || (matches!(
            (
                previous_primary.kind.as_str(),
                current_primary.kind.as_str()
            ),
            ("project", "directory") | ("directory", "project")
        ) && normalize_persisted_target(&previous_primary.path).is_empty()
            && normalize_persisted_target(&current_primary.path).is_empty());
    if !kind_matches
        || normalize_persisted_target(&previous_primary.path)
            != normalize_persisted_target(&current_primary.path)
    {
        return false;
    }

    let mut previous_components = previous
        .iter()
        .filter(|scope| scope.kind == "component")
        .map(|scope| normalize_persisted_target(&scope.path))
        .collect::<Vec<_>>();
    let mut current_components = current
        .iter()
        .filter(|scope| scope.kind == "component")
        .map(|scope| normalize_persisted_target(&scope.path))
        .collect::<Vec<_>>();
    previous_components.sort_unstable();
    current_components.sort_unstable();
    previous_components == current_components
}

fn resolve_child_target(
    scope: Option<&str>,
    project_dir: &Path,
) -> Result<ResolvedChildTarget, String> {
    let Some(scope) = scope else {
        return Ok(ResolvedChildTarget {
            working_dir: project_dir.to_path_buf(),
            target_path: project_dir.to_path_buf(),
            label: "project".to_string(),
            kind: "directory",
        });
    };

    if let Ok(path) = resolve_explore_target(scope, project_dir, "directory") {
        return Ok(ResolvedChildTarget {
            working_dir: path.clone(),
            target_path: path,
            label: concise_target_label(scope, 0),
            kind: "directory",
        });
    }

    let path = resolve_explore_target(scope, project_dir, "file")?;
    let working_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project_dir.to_path_buf());
    Ok(ResolvedChildTarget {
        working_dir,
        target_path: path,
        label: concise_target_label(scope, 0),
        kind: "file",
    })
}

fn build_child_target_scope(
    project_dir: &Path,
    label: &str,
    target_path: &Path,
    target_kind: &str,
    assigned_component: Option<&str>,
) -> Result<Vec<DelegatedRunScope>, String> {
    let mut scopes = vec![
        delegated_workspace_scope(project_dir)?,
        delegated_scope(label, target_path, target_kind, project_dir),
    ];
    if let Some(component) = assigned_component
        .map(str::trim)
        .filter(|component| !component.is_empty())
    {
        scopes.push(DelegatedRunScope {
            label: "assigned component".to_string(),
            path: component.to_string(),
            kind: "component".to_string(),
        });
    }
    Ok(scopes)
}

fn background_persistence_precondition(
    ctx: &ToolContext,
    store_available: bool,
    agent_type: &str,
) -> Option<ToolResult> {
    let reason = if ctx.db_path.is_none() {
        Some("this session has no durable database")
    } else if !store_available {
        Some("the delegated-run database could not be opened")
    } else if ctx.session_id.is_none() {
        Some("there is no durable parent session")
    } else {
        None
    }?;

    Some(ToolResult::error_with_code(
        "agent_persistence_error",
        format!("Background {agent_type} was not started because {reason}."),
    ))
}

impl AgentTool {
    // -----------------------------------------------------------------------
    // Unified parent-directed child
    // -----------------------------------------------------------------------

    pub(super) async fn execute_child(&self, params: Params, ctx: &ToolContext) -> ToolResult {
        let Some(project_dir) = ctx.project_dir.clone() else {
            return ToolResult::error(
                "No project directory is selected for this session. Select or create a project directory before spawning a child Agent.",
            );
        };

        let registry = match ctx.tool_registry.as_ref() {
            Some(r) => r.clone(),
            None => {
                return ToolResult::error(
                    "Tool registry not available in context. Cannot delegate child work.",
                );
            }
        };

        let client = self.resolve_client(ctx);

        // Resolve scope
        let resolved_target = match resolve_child_target(params.scope.as_deref(), &project_dir) {
            Ok(target) => target,
            Err(error) => {
                return ToolResult::error_with_code("invalid_explore_target", error);
            }
        };
        let working_dir = resolved_target.working_dir;
        let target_path = resolved_target.target_path;
        let scope_label = resolved_target.label;
        let scope_kind = resolved_target.kind;

        let delegated_run_id = Uuid::new_v4().to_string();
        let capabilities = params
            .capabilities
            .iter()
            .map(|capability| AgentCapability::parse(capability))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .expect("AgentSpec validated child capabilities");
        let wants_read = capabilities.contains(&AgentCapability::Read);
        let wants_write = params.capabilities.iter().any(|c| c == "write");
        let wants_execute = params.capabilities.iter().any(|c| c == "execute");
        let delegation_policy = DelegationPolicy::for_subagent_child(
            ctx.permission_mode,
            params.max_turns,
            wants_read,
            wants_write,
            wants_execute,
        )
        .with_supervised_approval(ctx.supervised_approval_granted)
        .with_execution_tool_allowlist(ctx.execution_tool_allowlist.as_ref());

        let target_scope = match build_child_target_scope(
            &project_dir,
            &scope_label,
            &target_path,
            scope_kind,
            params.assigned_component.as_deref(),
        ) {
            Ok(scopes) => scopes,
            Err(error) => {
                return ToolResult::error_with_code("invalid_project_workspace", error);
            }
        };

        let role = if wants_write {
            DelegatedRunRole::Build
        } else {
            DelegatedRunRole::Explore
        };
        let child_name = params.name.clone().unwrap_or_else(|| scope_label.clone());

        // Persist the name and exact capability contract before execution so
        // a crash-safe resume cannot widen execute-only into read access.
        let mut delegated_lease = open_delegated_run_store(ctx).map(DelegatedRunLease::new);
        let background = params.run_in_background.unwrap_or(false);
        if background {
            if let Some(error) =
                background_persistence_precondition(ctx, delegated_lease.is_some(), "child")
            {
                return error;
            }
        }
        let explicit_resume = params.resumed_from_run_id.is_some();
        let resume_candidate = match (
            delegated_lease.as_ref(),
            ctx.session_id.as_ref(),
            params.resumed_from_run_id.as_deref(),
        ) {
            (Some(store), Some(session_id), Some(resumed_from_run_id)) => {
                match store.get_run(resumed_from_run_id) {
                    Ok(Some(record))
                        if record.parent_session_id == *session_id
                            && record.effective_capabilities() == capabilities
                            && persisted_target_matches(&record.target_scope, &target_scope) =>
                    {
                        Some(record)
                    }
                    Ok(Some(_)) => {
                        return ToolResult::error_with_code(
                            "agent_resume_contract_mismatch",
                            "The selected delegated run no longer matches this session, capability contract, or persisted target.",
                        );
                    }
                    Ok(None) => {
                        return ToolResult::error_with_code(
                            "agent_run_not_found",
                            format!("Delegated run '{resumed_from_run_id}' was not found."),
                        );
                    }
                    Err(error) => {
                        return ToolResult::error_with_code("agent_store_error", error.to_string());
                    }
                }
            }
            (Some(store), Some(session_id), None) => store
                .find_related_run(session_id, role.clone(), &target_scope)
                .ok()
                .flatten()
                .filter(|record| record.effective_capabilities() == capabilities),
            _ => None,
        };

        let durable_run_started = if let (Some(lease), Some(session_id)) =
            (delegated_lease.as_mut(), ctx.session_id.as_ref())
        {
            let start = DelegatedRunStartInput {
                delegated_run_id: delegated_run_id.clone(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: ctx.tool_use_id.clone(),
                role,
                stage: DelegatedRunStage::Created,
                provider: Some(client.provider_id().to_string()),
                model: Some(client.config().model.clone()),
                resumable: true,
                // Related-run discovery may seed a fresh child with useful
                // evidence, but only an explicit lifecycle resume consumes
                // the origin's unique durable continuation claim.
                resumed_from_run_id: params.resumed_from_run_id.clone(),
                target_scope: target_scope.clone(),
            };
            let create = if background {
                lease.create_background_run_with_child_contract(
                    &start,
                    Some(&child_name),
                    &capabilities,
                )
            } else {
                lease.create_run_with_child_contract(&start, Some(&child_name), &capabilities)
            };
            match create {
                Ok(DelegatedRunCreateOutcome::Created) => {}
                Ok(DelegatedRunCreateOutcome::ExistingContinuation {
                    delegated_run_id,
                    resumed_from_run_id,
                }) => {
                    return existing_continuation_error(&resumed_from_run_id, &delegated_run_id);
                }
                Err(error) => {
                    return ToolResult::error_with_code(
                        "agent_persistence_error",
                        format!(
                            "Delegated child was not started because its durable run record could not be created: {error}"
                        ),
                    );
                }
            }
            true
        } else {
            false
        };
        if background && !durable_run_started {
            return ToolResult::error_with_code(
                "agent_persistence_error",
                "Background child was not started because durable run creation was unavailable.",
            );
        }

        // Build task prompt with resume context if available
        let mut task_prompt = params.prompt.clone();
        if !explicit_resume {
            if let Some(ref previous) = resume_candidate {
                if let Some(seed) = build_resume_seed(previous, &scope_label) {
                    task_prompt = format!("{}\n\n{}", task_prompt, seed);
                }
            }
        }

        let inherited_sandbox = ctx
            .sandbox_root
            .clone()
            .unwrap_or_else(|| working_dir.clone());
        let mut task = SubAgentTask::new("child-0", &task_prompt)
            .with_name(child_name.clone())
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

        // Every capability class follows this same execution path and inherits
        // the parent's resolved model unless the run explicitly overrides it.
        let model = self.resolve_model(ctx, &client);

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

        let cancellation_token = if background {
            self.cancellation.child_token()
        } else {
            ctx.execution_cancellation
                .clone()
                .unwrap_or_else(|| self.cancellation.child_token())
        };
        let progress_tx = ctx.agent_progress_tx.clone();

        info!(
            delegated_run_id = %delegated_run_id,
            name = %child_name,
            model = %model,
            scope = %scope_label,
            background,
            "Agent tool: starting agnostic child"
        );

        // ── Background mode ──────────────────────────────────────────
        if background {
            let bg_delegation_policy = delegation_policy.clone();
            let bg_delegated_run_id = delegated_run_id.clone();
            let bg_db_path = ctx.db_path.clone();
            let bg_session_id = ctx.session_id.clone();
            let bg_user_id = ctx.user_id.clone();
            let bg_workspace_root = ctx.filesystem_access_root();
            let bg_child_name = child_name.clone();
            let bg_runtime = self.runtime.clone();
            let mut bg_run_lease = delegated_lease
                .take()
                .expect("background child start has an armed durable lease");
            let bg_host_heartbeat = match bg_run_lease
                .start_background_host_heartbeat(&bg_delegated_run_id, cancellation_token.clone())
            {
                Ok(heartbeat) => heartbeat,
                Err(error) => {
                    return ToolResult::error_with_code(
                        "agent_persistence_error",
                        format!(
                            "Background child was not started because its durable host lease could not start: {error}"
                        ),
                    );
                }
            };
            let (mailbox, mut bg_runtime_registration) = bg_runtime.register_guarded(
                bg_delegated_run_id.clone(),
                bg_child_name.clone(),
                bg_session_id.clone(),
                cancellation_token.clone(),
            );
            task = task.with_mailbox(mailbox);

            tokio::spawn(async move {
                let _bg_host_heartbeat = bg_host_heartbeat;
                let result = execute_single_child(
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

                let finalization = persist_background_single_agent_artifact(
                    &bg_run_lease,
                    &bg_delegated_run_id,
                    &artifact,
                    true,
                    &bg_child_name,
                );

                match finalization {
                    Ok(authoritative) => {
                        bg_run_lease.disarm(&bg_delegated_run_id);
                        let authoritative_success =
                            authoritative.stage == DelegatedRunStage::Complete;
                        let authoritative_summary = authoritative
                            .human_review
                            .as_deref()
                            .unwrap_or(&artifact.review_summary);
                        emit_single_agent_completion(
                            &progress_tx,
                            &bg_delegated_run_id,
                            &bg_child_name,
                            &result,
                            authoritative.stage,
                            authoritative_summary,
                        );
                        if authoritative.stage != DelegatedRunStage::Cancelled {
                            if let Err(error) = notify_child_completion(
                                &bg_runtime,
                                bg_db_path.as_deref(),
                                bg_session_id.as_deref(),
                                bg_user_id.as_deref(),
                                bg_workspace_root.as_deref(),
                                &bg_delegated_run_id,
                                &bg_child_name,
                                authoritative_success,
                                authoritative_summary,
                            ) {
                                warn!(
                                    delegated_run_id = %bg_delegated_run_id,
                                    %error,
                                    "Failed to queue background child completion"
                                );
                                let _ = bg_runtime
                                    .request_completion_reconciliation(bg_delegated_run_id.clone());
                            }
                        }
                        bg_runtime_registration.finish(authoritative_success);
                    }
                    Err(error) => {
                        tracing::error!(
                            delegated_run_id = %bg_delegated_run_id,
                            %error,
                            "Suppressing background child completion because terminal finalization was not authoritative"
                        );
                        // Leave the guard armed. Its Drop path asks the server
                        // to reconcile the lease's abnormal durable terminal.
                    }
                }
            });

            return background_started_result(
                &delegated_run_id,
                &child_name,
                Some(child_name.as_str()),
            );
        }

        // ── Synchronous mode (existing behavior) ─────────────────────
        let completion_tx = progress_tx.clone();
        let result = execute_single_child(
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
        if durable_run_started {
            let lease = delegated_lease
                .as_mut()
                .expect("durable child start has an open store");
            let authoritative = match persist_single_agent_artifact(
                lease,
                &delegated_run_id,
                &artifact,
                true,
                "Failed to persist delegated child run final artifact",
            ) {
                Ok(authoritative) => authoritative,
                Err(error) => {
                    return delegated_persistence_error(
                        &delegated_run_id,
                        artifact.payload,
                        &error,
                    );
                }
            };
            lease.disarm(&delegated_run_id);
            // The child's own terminal frame precedes artifact persistence and
            // is intentionally kept Running by the server until this durable
            // aggregate boundary. Re-emit now so foreground cards settle
            // without waiting for a reconnect.
            emit_single_agent_completion(
                &completion_tx,
                &delegated_run_id,
                &child_name,
                &result,
                authoritative.stage,
                authoritative
                    .human_review
                    .as_deref()
                    .unwrap_or(&artifact.review_summary),
            );
        }

        let mut warnings = build_single_agent_warnings(&result, "Child Agent");
        if !durable_run_started {
            warnings.push(
                "This synchronous child ran without a durable delegated-run record.".to_string(),
            );
        }

        ToolResult::success_data_with(artifact.payload, warnings, None, None)
    }
}

#[cfg(test)]
mod child_scope_tests {
    use std::fs;

    use super::*;

    #[test]
    fn file_target_persists_the_file_not_its_working_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let source = project.join("src/auth");
        fs::create_dir_all(&source).expect("source directory");
        let file = source.join("mod.rs");
        fs::write(&file, "pub fn authenticate() {}\n").expect("source file");

        let resolved = resolve_child_target(Some("src/auth/mod.rs"), &project)
            .expect("file target should resolve");
        let scopes = build_child_target_scope(
            &project,
            &resolved.label,
            &resolved.target_path,
            resolved.kind,
            Some("authentication component"),
        )
        .expect("target lineage should build");

        assert_eq!(
            resolved.working_dir,
            source.canonicalize().expect("canonical source")
        );
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[0].kind, "workspace");
        assert_eq!(
            scopes[0].path,
            project
                .canonicalize()
                .expect("canonical project")
                .display()
                .to_string()
        );
        assert_eq!(scopes[1].path, "src/auth/mod.rs");
        assert_eq!(scopes[1].kind, "file");
        assert_eq!(scopes[2].kind, "component");
        assert_eq!(scopes[2].path, "authentication component");
        assert!(persisted_target_matches(&scopes, &scopes));

        let mut widened = scopes.clone();
        widened[1].path = "src/auth".to_string();
        widened[1].kind = "directory".to_string();
        assert!(!persisted_target_matches(&scopes, &widened));

        let mut foreign_workspace = scopes.clone();
        foreign_workspace[0].path = temp.path().display().to_string();
        assert!(!persisted_target_matches(&scopes, &foreign_workspace));
    }

    #[test]
    fn background_start_requires_database_store_and_parent_session() {
        let missing_database =
            background_persistence_precondition(&ToolContext::default(), false, "child")
                .expect("missing database should reject background start");
        let envelope: serde_json::Value =
            serde_json::from_str(&missing_database.output).expect("structured start error");
        assert_eq!(envelope["error"]["code"], "agent_persistence_error");
        assert!(envelope["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("no durable database")));

        let temp = tempfile::tempdir().expect("tempdir");
        let missing_session = ToolContext {
            db_path: Some(temp.path().join("krusty.db")),
            ..ToolContext::default()
        };
        let error = background_persistence_precondition(&missing_session, true, "build")
            .expect("missing parent session should reject background start");
        let envelope: serde_json::Value =
            serde_json::from_str(&error.output).expect("structured start error");
        assert!(envelope["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("no durable parent session")));
    }
}
