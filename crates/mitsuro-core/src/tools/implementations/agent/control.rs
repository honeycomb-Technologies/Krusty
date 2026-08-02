use std::time::{Duration, Instant};

use serde_json::json;

use crate::agent::subagent::AgentCapability;
use crate::agent::DelegatedRunStage;
use crate::storage::{DelegatedRunRecord, DelegatedRunRole};
use crate::tools::{ToolContext, ToolResult};

use super::{open_delegated_run_store, AgentAction, AgentTool, Params};

fn session_id(ctx: &ToolContext) -> Result<&str, ToolResult> {
    ctx.session_id.as_deref().ok_or_else(|| {
        ToolResult::error_with_code(
            "agent_session_required",
            "Agent lifecycle actions require a persisted parent session.",
        )
    })
}

fn load_owned_run(
    ctx: &ToolContext,
    delegated_run_id: &str,
) -> Result<DelegatedRunRecord, ToolResult> {
    let parent_session_id = session_id(ctx)?;
    let store = open_delegated_run_store(ctx).ok_or_else(|| {
        ToolResult::error_with_code(
            "agent_store_unavailable",
            "Delegated run storage is unavailable for this session.",
        )
    })?;
    let record = store
        .get_run(delegated_run_id)
        .map_err(|error| ToolResult::error_with_code("agent_store_error", error.to_string()))?
        .ok_or_else(|| {
            ToolResult::error_with_code(
                "agent_run_not_found",
                format!("Delegated run '{delegated_run_id}' was not found."),
            )
        })?;
    if record.parent_session_id != parent_session_id {
        return Err(ToolResult::error_with_code(
            "agent_run_not_found",
            format!("Delegated run '{delegated_run_id}' was not found."),
        ));
    }
    Ok(record)
}

fn required_run_id(params: &Params) -> Result<&str, ToolResult> {
    params
        .delegated_run_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ToolResult::error_with_code(
                "delegated_run_id_required",
                "This agent lifecycle action requires delegated_run_id.",
            )
        })
}

fn is_terminal(stage: DelegatedRunStage) -> bool {
    matches!(
        stage,
        DelegatedRunStage::Complete
            | DelegatedRunStage::Degraded
            | DelegatedRunStage::Failed
            | DelegatedRunStage::Cancelled
    )
}

fn role_profile(role: &DelegatedRunRole) -> &'static str {
    match role {
        DelegatedRunRole::Explore => "explore",
        DelegatedRunRole::Build => "build",
        DelegatedRunRole::Planner => "plan",
        DelegatedRunRole::Verifier => "verify",
    }
}

fn should_resume_terminal_followup(action: AgentAction, record: &DelegatedRunRecord) -> bool {
    action == AgentAction::Followup && is_terminal(record.stage)
}

fn apply_persisted_child_contract(
    params: &mut Params,
    record: &DelegatedRunRecord,
    delegated_run_id: &str,
) -> Result<(), ToolResult> {
    let has_explicit_contract = !record.capabilities.is_empty();
    params.profile = Some(if has_explicit_contract {
        "child".to_string()
    } else {
        role_profile(&record.role).to_string()
    });
    params.agent_type = None;
    params.capabilities = record
        .effective_capabilities()
        .into_iter()
        .map(|capability| match capability {
            AgentCapability::Read => "read".to_string(),
            AgentCapability::Write => "write".to_string(),
            AgentCapability::Execute => "execute".to_string(),
        })
        .collect();

    if params.name.is_none() {
        params.name = record.child_name.clone().or_else(|| {
            Some(format!(
                "resume-{}",
                &delegated_run_id[..delegated_run_id.len().min(8)]
            ))
        });
    }

    match record.target_scope.as_slice() {
        [] => {
            return Err(ToolResult::error_with_code(
                "agent_resume_target_invalid",
                format!(
                    "Delegated run '{delegated_run_id}' has no persisted target scope and cannot be resumed safely. Spawn a new child instead."
                ),
            ));
        }
        [scope] if scope.kind == "component" => {
            return Err(ToolResult::error_with_code(
                "agent_resume_target_invalid",
                format!(
                    "Delegated run '{delegated_run_id}' has a legacy single-component scope that cannot be reconstructed without widening it. Spawn a new child instead."
                ),
            ));
        }
        [scope] if matches!(scope.kind.as_str(), "directory" | "file") => {
            reject_conflicting_components(params, delegated_run_id)?;
            if let Some(requested_scope) = params.scope.as_deref() {
                if normalize_target(requested_scope) != normalize_target(&scope.path) {
                    return Err(resume_target_conflict(delegated_run_id));
                }
            }
            params.scope = if normalize_target(&scope.path).is_empty() {
                None
            } else {
                Some(scope.path.clone())
            };
        }
        [scope] if scope.kind == "project" => {
            reject_conflicting_components(params, delegated_run_id)?;
            if params
                .scope
                .as_deref()
                .is_some_and(|value| !normalize_target(value).is_empty())
            {
                return Err(resume_target_conflict(delegated_run_id));
            }
            params.scope = None;
        }
        scopes if scopes.iter().all(|scope| scope.kind == "component") => {
            reject_conflicting_scope(params, delegated_run_id)?;
            let components = scopes
                .iter()
                .map(|scope| scope.path.clone())
                .collect::<Vec<_>>();
            restore_exact_components(params, delegated_run_id, &components)?;
        }
        _ => {
            return Err(ToolResult::error_with_code(
                "agent_resume_target_invalid",
                format!(
                    "Delegated run '{delegated_run_id}' has a legacy mixed target scope that cannot be resumed safely. Spawn a new child to retarget the work."
                ),
            ));
        }
    }

    Ok(())
}

fn normalize_target(value: &str) -> &str {
    let trimmed = value.trim().trim_end_matches('/');
    let normalized = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if normalized == "." {
        ""
    } else {
        normalized
    }
}

fn resume_target_conflict(delegated_run_id: &str) -> ToolResult {
    ToolResult::error_with_code(
        "agent_resume_target_conflict",
        format!(
            "Resume must keep delegated run '{delegated_run_id}' on its persisted target. Spawn a new child to use a different scope or component set."
        ),
    )
}

fn reject_conflicting_scope(params: &Params, delegated_run_id: &str) -> Result<(), ToolResult> {
    if params
        .scope
        .as_deref()
        .is_some_and(|scope| !scope.trim().is_empty())
    {
        return Err(resume_target_conflict(delegated_run_id));
    }
    Ok(())
}

fn reject_conflicting_components(
    params: &Params,
    delegated_run_id: &str,
) -> Result<(), ToolResult> {
    if params
        .components
        .as_ref()
        .is_some_and(|components| !components.is_empty())
    {
        return Err(resume_target_conflict(delegated_run_id));
    }
    Ok(())
}

fn restore_exact_components(
    params: &mut Params,
    delegated_run_id: &str,
    components: &[String],
) -> Result<(), ToolResult> {
    if let Some(requested) = params.components.as_ref() {
        let requested = requested
            .iter()
            .map(|component| component.trim())
            .collect::<Vec<_>>();
        let persisted = components
            .iter()
            .map(|component| component.trim())
            .collect::<Vec<_>>();
        if requested != persisted {
            return Err(resume_target_conflict(delegated_run_id));
        }
    }
    params.components = Some(components.to_vec());
    Ok(())
}

impl AgentTool {
    async fn execute_resume_from_record(
        &self,
        mut params: Params,
        ctx: &ToolContext,
        delegated_run_id: String,
        record: DelegatedRunRecord,
    ) -> ToolResult {
        if !record.resumable {
            return ToolResult::error_with_code(
                "agent_run_not_resumable",
                format!("Delegated run '{delegated_run_id}' is not resumable."),
            );
        }

        let prior_evidence = record
            .artifact
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "No prior artifact was persisted.".to_string());
        let requested_objective = params.prompt.trim();
        params.prompt = if requested_objective.is_empty() {
            format!(
                "Resume delegated run {delegated_run_id}. Continue from durable evidence and close remaining gaps.\n\n[PRIOR EVIDENCE]\n{prior_evidence}\n[/PRIOR EVIDENCE]"
            )
        } else {
            format!(
                "{requested_objective}\n\nResume delegated run {delegated_run_id}.\n[PRIOR EVIDENCE]\n{prior_evidence}\n[/PRIOR EVIDENCE]"
            )
        };
        params.action = AgentAction::Spawn;
        params.delegated_run_id = None;
        params.run_in_background = Some(params.run_in_background.unwrap_or(true));
        if let Err(error) = apply_persisted_child_contract(&mut params, &record, &delegated_run_id)
        {
            return error;
        }
        params.resumed_from_run_id = Some(delegated_run_id.clone());
        self.execute_spawn(params, ctx).await
    }

    pub(super) async fn execute_control(
        &self,
        mut params: Params,
        ctx: &ToolContext,
    ) -> ToolResult {
        match params.action {
            AgentAction::Spawn => self.execute_spawn(params, ctx).await,
            AgentAction::List => {
                let parent_session_id = match session_id(ctx) {
                    Ok(value) => value,
                    Err(error) => return error,
                };
                let store = match open_delegated_run_store(ctx) {
                    Some(store) => store,
                    None => {
                        return ToolResult::error_with_code(
                            "agent_store_unavailable",
                            "Delegated run storage is unavailable for this session.",
                        )
                    }
                };
                let limit = params.limit.unwrap_or(20).clamp(1, 100);
                match store.list_runs_for_session(parent_session_id, limit) {
                    Ok(runs) => ToolResult::success_data(json!({
                        "runs": runs,
                        "live": self.runtime.snapshots(),
                    })),
                    Err(error) => {
                        ToolResult::error_with_code("agent_store_error", error.to_string())
                    }
                }
            }
            AgentAction::Status => {
                let delegated_run_id = match required_run_id(&params) {
                    Ok(value) => value,
                    Err(error) => return error,
                };
                match load_owned_run(ctx, delegated_run_id) {
                    Ok(run) => {
                        let live = self
                            .runtime
                            .snapshots()
                            .into_iter()
                            .find(|snapshot| snapshot.delegated_run_id == delegated_run_id);
                        ToolResult::success_data(json!({"run": run, "live": live}))
                    }
                    Err(error) => error,
                }
            }
            AgentAction::Wait => {
                let delegated_run_id = match required_run_id(&params) {
                    Ok(value) => value.to_string(),
                    Err(error) => return error,
                };
                let timeout =
                    Duration::from_millis(params.wait_timeout_ms.unwrap_or(30_000).min(300_000));
                let started = Instant::now();
                loop {
                    let run = match load_owned_run(ctx, &delegated_run_id) {
                        Ok(run) => run,
                        Err(error) => return error,
                    };
                    if is_terminal(run.stage) || started.elapsed() >= timeout {
                        return ToolResult::success_data(json!({
                            "terminal": is_terminal(run.stage),
                            "timed_out": !is_terminal(run.stage),
                            "run": run,
                        }));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            AgentAction::Message | AgentAction::Followup => {
                let delegated_run_id = match required_run_id(&params) {
                    Ok(value) => value.to_string(),
                    Err(error) => return error,
                };
                let record = match load_owned_run(ctx, &delegated_run_id) {
                    Ok(record) => record,
                    Err(error) => return error,
                };
                let message = match params
                    .message
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(value) => value.to_string(),
                    None => {
                        return ToolResult::error_with_code(
                            "agent_message_required",
                            "message or followup requires non-empty message.",
                        )
                    }
                };
                match self
                    .runtime
                    .send_message(&delegated_run_id, message.clone())
                {
                    Ok(()) => ToolResult::success_data(json!({
                        "status": "delivered",
                        "delegated_run_id": delegated_run_id,
                    })),
                    Err(_) if should_resume_terminal_followup(params.action, &record) => {
                        params.prompt = message;
                        self.execute_resume_from_record(params, ctx, delegated_run_id, record)
                            .await
                    }
                    Err(error) => ToolResult::error_with_code("agent_not_live", error),
                }
            }
            AgentAction::Interrupt => {
                let delegated_run_id = match required_run_id(&params) {
                    Ok(value) => value.to_string(),
                    Err(error) => return error,
                };
                let record = match load_owned_run(ctx, &delegated_run_id) {
                    Ok(record) => record,
                    Err(error) => return error,
                };
                if is_terminal(record.stage) {
                    return ToolResult::success_data(json!({
                        "status": "already_terminal",
                        "run": record,
                    }));
                }
                if let Err(error) = self.runtime.cancel(&delegated_run_id) {
                    return ToolResult::error_with_code("agent_not_live", error);
                }
                if let Some(store) = open_delegated_run_store(ctx) {
                    let artifact = json!({
                        "delegated_run_id": delegated_run_id,
                        "outcome": "cancelled",
                        "outcome_reason": "parent_interrupt",
                    });
                    if let Err(error) = store.finalize_run(
                        &delegated_run_id,
                        DelegatedRunStage::Cancelled,
                        &artifact,
                        Some("Cancelled by parent agent."),
                        true,
                    ) {
                        return ToolResult::error_with_code("agent_store_error", error.to_string());
                    }
                }
                ToolResult::success_data(json!({
                    "status": "cancelled",
                    "delegated_run_id": delegated_run_id,
                }))
            }
            AgentAction::Resume => {
                let delegated_run_id = match required_run_id(&params) {
                    Ok(value) => value.to_string(),
                    Err(error) => return error,
                };
                let record = match load_owned_run(ctx, &delegated_run_id) {
                    Ok(record) => record,
                    Err(error) => return error,
                };
                self.execute_resume_from_record(params, ctx, delegated_run_id, record)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rusqlite::params;
    use tempfile::TempDir;

    use super::*;
    use crate::storage::{Database, DelegatedRunScope, DelegatedRunStartInput, DelegatedRunStore};

    fn ownership_fixture() -> (ToolContext, ToolContext, TempDir) {
        let temp = TempDir::new().expect("tempdir");
        let db_path = temp.path().join("agent-control.db");
        let db = Database::new(&db_path).expect("database");
        let now = Utc::now().to_rfc3339();
        for session in ["parent-a", "parent-b"] {
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                    params![session, session, now, now],
                )
                .expect("seed session");
        }
        DelegatedRunStore::new(db)
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: "run-owned".to_string(),
                parent_session_id: "parent-a".to_string(),
                parent_tool_call_id: Some("tool-1".to_string()),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Running,
                provider: None,
                model: None,
                resumable: true,
                resumed_from_run_id: None,
                target_scope: vec![DelegatedRunScope {
                    label: "project".to_string(),
                    path: ".".to_string(),
                    kind: "project".to_string(),
                }],
            })
            .expect("seed delegated run");

        let owner = ToolContext {
            session_id: Some("parent-a".to_string()),
            db_path: Some(db_path.clone()),
            ..Default::default()
        };
        let foreign = ToolContext {
            session_id: Some("parent-b".to_string()),
            db_path: Some(db_path),
            ..Default::default()
        };
        (owner, foreign, temp)
    }

    #[test]
    fn lifecycle_lookup_hides_foreign_parent_runs() {
        let (owner, foreign, _temp) = ownership_fixture();
        assert_eq!(
            load_owned_run(&owner, "run-owned")
                .expect("owner can read")
                .delegated_run_id,
            "run-owned"
        );

        let error = load_owned_run(&foreign, "run-owned").expect_err("foreign run hidden");
        let envelope: serde_json::Value =
            serde_json::from_str(&error.output).expect("structured error");
        assert_eq!(envelope["error"]["code"], "agent_run_not_found");
    }

    #[test]
    fn only_terminal_followup_selects_durable_resume_path() {
        let (owner, _foreign, _temp) = ownership_fixture();
        let running = load_owned_run(&owner, "run-owned").expect("running run");
        assert!(!should_resume_terminal_followup(
            AgentAction::Followup,
            &running
        ));

        let store = open_delegated_run_store(&owner).expect("delegated store");
        store
            .finalize_run(
                "run-owned",
                DelegatedRunStage::Complete,
                &json!({"success": true, "findings": "durable result"}),
                Some("durable result"),
                true,
            )
            .expect("finalize delegated run");
        let completed = load_owned_run(&owner, "run-owned").expect("completed run");

        assert!(should_resume_terminal_followup(
            AgentAction::Followup,
            &completed
        ));
        assert!(!should_resume_terminal_followup(
            AgentAction::Message,
            &completed
        ));
    }

    #[test]
    fn resume_restores_execute_only_contract_name_and_not_fake_components() {
        let (owner, _foreign, _temp) = ownership_fixture();
        let db_path = owner.db_path.as_ref().expect("db path");
        let store = DelegatedRunStore::new(Database::new(db_path).expect("db"));
        let capabilities = [AgentCapability::Execute].into_iter().collect();
        store
            .create_run_with_child_contract(
                &DelegatedRunStartInput {
                    delegated_run_id: "run-exec".to_string(),
                    parent_session_id: "parent-a".to_string(),
                    parent_tool_call_id: Some("tool-exec".to_string()),
                    role: DelegatedRunRole::Explore,
                    stage: DelegatedRunStage::Complete,
                    provider: None,
                    model: None,
                    resumable: true,
                    resumed_from_run_id: None,
                    target_scope: vec![DelegatedRunScope {
                        label: "project".to_string(),
                        path: ".".to_string(),
                        kind: "project".to_string(),
                    }],
                },
                Some("focused validator"),
                &capabilities,
            )
            .expect("seed contracted run");
        let record = store
            .get_run("run-exec")
            .expect("read run")
            .expect("run exists");
        let mut params = Params::default();

        apply_persisted_child_contract(&mut params, &record, "run-exec")
            .expect("persisted contract should restore");

        assert_eq!(params.profile.as_deref(), Some("child"));
        assert_eq!(params.name.as_deref(), Some("focused validator"));
        assert_eq!(params.capabilities, vec!["execute"]);
        assert!(params.components.is_none());
        assert!(params.scope.is_none());
    }

    #[test]
    fn resume_restores_single_scope_and_rejects_retargeting() {
        let (owner, _foreign, _temp) = ownership_fixture();
        let mut record = load_owned_run(&owner, "run-owned").expect("seed run");
        record.target_scope = vec![DelegatedRunScope {
            label: "auth/mod.rs".to_string(),
            path: "src/auth/mod.rs".to_string(),
            kind: "file".to_string(),
        }];

        let mut params = Params::default();
        apply_persisted_child_contract(&mut params, &record, "run-owned")
            .expect("file scope should restore");
        assert_eq!(params.scope.as_deref(), Some("src/auth/mod.rs"));

        let mut conflicting = Params {
            scope: Some("src/billing/mod.rs".to_string()),
            ..Params::default()
        };
        let error = apply_persisted_child_contract(&mut conflicting, &record, "run-owned")
            .expect_err("resume must reject a different target");
        assert!(error.output.contains("agent_resume_target_conflict"));
    }

    #[test]
    fn resume_restores_exact_parallel_components() {
        let (owner, _foreign, _temp) = ownership_fixture();
        let mut record = load_owned_run(&owner, "run-owned").expect("seed run");
        record.role = DelegatedRunRole::Build;
        record.target_scope = vec![
            DelegatedRunScope {
                label: "api".to_string(),
                path: "API component".to_string(),
                kind: "component".to_string(),
            },
            DelegatedRunScope {
                label: "ui".to_string(),
                path: "UI component".to_string(),
                kind: "component".to_string(),
            },
        ];

        let mut params = Params::default();
        apply_persisted_child_contract(&mut params, &record, "run-owned")
            .expect("components should restore");
        assert_eq!(
            params.components,
            Some(vec![
                "API component".to_string(),
                "UI component".to_string()
            ])
        );
    }
}
