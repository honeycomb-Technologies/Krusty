use std::time::Duration;

use crate::process::CommandEnvironmentPolicy;
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
    .with_subagent_max_turns(delegated_turn_budget(task))
    .with_delegated_reasoning_effort(task.reasoning_effort);

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
    ctx.command_environment = task.command_environment.clone();
    configure_sanitized_command_environment(&mut ctx);
    ctx.user_id = task.process_owner_id.clone();
    ctx.process_owner_id = Some(task.delegated_process_owner_id());
    ctx.session_id = task.parent_session_id.clone();

    ctx
}

fn configure_sanitized_command_environment(ctx: &mut ToolContext) {
    let root = ctx
        .filesystem_access_root()
        .unwrap_or_else(|| ctx.working_dir.clone());
    let home = root.join(".mitsuro/home");
    let temp = root.join(".mitsuro/tmp");
    let cache = root.join(".mitsuro/cache");
    let cargo = root.join(".cargo-home");
    let npm = root.join(".mitsuro/npm");

    for directory in [&home, &temp, &cache, &cargo, &npm] {
        if let Err(error) = std::fs::create_dir_all(directory) {
            tracing::warn!(
                path = %directory.display(),
                %error,
                "Failed to prepare delegated command runtime directory"
            );
        }
    }

    let environment = &mut ctx.command_environment;
    for (key, path) in [
        ("HOME", &home),
        ("USERPROFILE", &home),
        ("TMPDIR", &temp),
        ("TMP", &temp),
        ("TEMP", &temp),
        ("XDG_CACHE_HOME", &cache),
        ("CARGO_HOME", &cargo),
        ("npm_config_cache", &npm),
        ("NPM_CONFIG_CACHE", &npm),
    ] {
        environment
            .entry(key.to_string())
            .or_insert_with(|| path.display().to_string());
    }

    if !environment.contains_key("RUSTUP_HOME") {
        let rustup_home = std::env::var("RUSTUP_HOME").ok().or_else(|| {
            std::env::var("HOME").ok().map(|home| {
                std::path::PathBuf::from(home)
                    .join(".rustup")
                    .display()
                    .to_string()
            })
        });
        if let Some(rustup_home) = rustup_home {
            environment.insert("RUSTUP_HOME".to_string(), rustup_home);
        }
    }

    ctx.command_environment_policy = CommandEnvironmentPolicy::Sanitized;
}

pub(super) fn delegated_is_explore(task: &SubAgentTask) -> bool {
    task.delegation_policy
        .as_ref()
        .is_some_and(|policy| policy.read_only_only)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn delegated_context_preserves_runtime_owned_environment_and_scope() {
        let directory = TempDir::new().expect("temporary root");
        let root = directory.path().to_path_buf();
        let environment = BTreeMap::from([
            (
                "CARGO_HOME".to_string(),
                root.join(".cargo-home").display().to_string(),
            ),
            (
                "TMPDIR".to_string(),
                root.join(".mitsuro/tmp").display().to_string(),
            ),
        ]);
        let task = SubAgentTask::new("builder", "build")
            .with_working_dir(root.clone())
            .with_sandbox_root(root.clone())
            .with_command_environment(environment.clone());

        let context = build_subagent_tool_context(&task, 30);

        for (key, value) in environment {
            assert_eq!(context.command_environment.get(&key), Some(&value));
        }
        assert_eq!(
            context.command_environment.get("HOME"),
            Some(&root.join(".mitsuro/home").display().to_string())
        );
        assert_eq!(
            context.command_environment_policy,
            CommandEnvironmentPolicy::Sanitized
        );
        assert_eq!(
            context.filesystem_access_root().as_deref(),
            Some(root.as_path())
        );
    }
}
