use std::time::Duration;

use crate::agent::AgentConfig as RuntimeAgentConfig;
use crate::tools::registry::ToolContext;

use super::super::types::SubAgentTask;

pub(super) fn delegated_turn_budget(task: &SubAgentTask) -> Option<usize> {
    task.max_turns_override
        .or(task
            .delegation_policy
            .as_ref()
            .and_then(|policy| policy.max_turns))
        .or(RuntimeAgentConfig::default().subagent_max_turns)
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

    if delegated_is_explore(task) {
        ctx = ctx.with_sandbox(task.working_dir.clone());
    }

    ctx
}

pub(super) fn delegated_is_explore(task: &SubAgentTask) -> bool {
    task.delegation_policy
        .as_ref()
        .is_some_and(|policy| policy.read_only_only)
}
