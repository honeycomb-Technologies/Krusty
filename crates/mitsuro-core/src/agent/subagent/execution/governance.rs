use std::time::Duration;

use crate::tools::registry::ToolContext;

use super::super::types::SubAgentTask;

pub(super) fn delegated_turn_budget(task: &SubAgentTask) -> Option<usize> {
    task.max_turns_override.or(task
        .delegation_policy
        .as_ref()
        .and_then(|policy| policy.max_turns))
}

pub(super) fn build_subagent_tool_context(task: &SubAgentTask, timeout_secs: u64) -> ToolContext {
    let mut ctx = ToolContext {
        working_dir: task.working_dir.clone(),
        timeout: Some(Duration::from_secs(timeout_secs)),
        ..Default::default()
    }
    .with_subagent_max_turns(delegated_turn_budget(task));

    if let Some(policy) = task.delegation_policy.clone() {
        ctx = ctx
            .with_permission_mode(policy.inherited_permission_mode)
            .with_delegation_policy(policy);
    }

    // Delegated agents inherit the parent's filesystem access boundary when one
    // is present. Unrestricted local parents remain unrestricted; read-only
    // delegation is enforced by the delegation policy, not by inventing a path root.
    if let Some(sandbox_root) = task.sandbox_root.clone() {
        ctx = ctx.with_sandbox(sandbox_root);
    }

    ctx.process_registry = task.process_registry.clone();
    ctx.user_id = task.process_owner_id.clone();
    ctx.session_id = task.parent_session_id.clone();

    ctx
}

pub(super) fn delegated_is_explore(task: &SubAgentTask) -> bool {
    task.delegation_policy
        .as_ref()
        .is_some_and(|policy| policy.read_only_only)
}
