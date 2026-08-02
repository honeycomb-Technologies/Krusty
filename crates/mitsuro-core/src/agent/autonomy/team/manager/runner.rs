use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::subagent::{
    execute_single_agent, AgentIdentity, AgentProgress, AgentProgressStatus, SingleExplorerConfig,
    SubAgentTask,
};
use crate::ai::client::AiClient;
use crate::process::ProcessRegistry;
use crate::tools::registry::{DelegationPolicy, PermissionMode, ToolRegistry};

use super::super::teammate::{TeammateConfig, TeammateStatus};
use super::task_store::{poll_next_task, record_task_complete, record_task_failed};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Background loop for a single teammate.
///
/// Polls the autonomous_tasks table for unclaimed tasks, executes them via the
/// subagent execution loop, and records results. Exits after `IDLE_TIMEOUT` of
/// continuous idleness.
pub(super) async fn run_teammate_loop(
    config: TeammateConfig,
    cancel_token: CancellationToken,
    status: Arc<RwLock<TeammateStatus>>,
    progress: Arc<RwLock<AgentProgress>>,
    ai_client: Arc<AiClient>,
    tool_registry: Arc<ToolRegistry>,
    working_dir: PathBuf,
    session_id: String,
    db_path: PathBuf,
    process_registry: Option<Arc<ProcessRegistry>>,
    process_owner_id: Option<String>,
    permission_mode: PermissionMode,
    identity: AgentIdentity,
) {
    let policy = config
        .role
        .delegation_policy(permission_mode, config.max_turns);
    let model = ai_client.config().model.clone();

    info!(
        teammate = %config.name,
        role = %config.role,
        model = %model,
        "teammate loop started"
    );

    let mut consecutive_idle_polls: u64 = 0;
    let max_idle_polls = IDLE_TIMEOUT.as_secs() / POLL_INTERVAL.as_secs();

    loop {
        if cancel_token.is_cancelled() {
            *status.write().await = TeammateStatus::Cancelled;
            info!(teammate = %config.name, "teammate cancelled");
            return;
        }

        let task = match poll_next_task(&db_path, &session_id, &config.name).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                consecutive_idle_polls += 1;
                if consecutive_idle_polls >= max_idle_polls {
                    info!(
                        teammate = %config.name,
                        idle_secs = consecutive_idle_polls * POLL_INTERVAL.as_secs(),
                        "teammate exiting after idle timeout"
                    );
                    *status.write().await = TeammateStatus::Idle;
                    return;
                }

                *status.write().await = TeammateStatus::Idle;
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        *status.write().await = TeammateStatus::Cancelled;
                        info!(teammate = %config.name, "teammate cancelled while idle");
                        return;
                    }
                    _ = sleep(POLL_INTERVAL) => continue,
                }
            }
            Err(error) => {
                warn!(
                    teammate = %config.name,
                    error = %error,
                    "failed to poll for tasks"
                );
                sleep(POLL_INTERVAL).await;
                continue;
            }
        };

        consecutive_idle_polls = 0;
        let task_id = task.task_id.clone();
        let task_description = task.description.clone();

        info!(
            teammate = %config.name,
            task_id = %task_id,
            "claimed task"
        );

        *status.write().await = TeammateStatus::Working {
            task_id: task_id.clone(),
        };

        {
            let mut current = progress.write().await;
            *current = AgentProgress {
                task_id: task_id.clone(),
                name: identity.display_name(),
                identity: Some(identity.clone()),
                status: AgentProgressStatus::Running,
                ..Default::default()
            };
        }

        let subagent_task = build_teammate_subagent_task(
            &task_id,
            &task_description,
            &config.name,
            working_dir.clone(),
            policy.clone(),
            process_registry.clone(),
            process_owner_id.clone(),
            &session_id,
            identity.clone(),
        );

        let agent_config =
            SingleExplorerConfig::new(tool_registry.clone(), policy.clone(), String::new()).await;

        let result = execute_single_agent(
            &ai_client,
            subagent_task,
            agent_config,
            &model,
            cancel_token.clone(),
            None,
        )
        .await;

        if cancel_token.is_cancelled() {
            *status.write().await = TeammateStatus::Cancelled;
            return;
        }

        if result.success {
            info!(
                teammate = %config.name,
                task_id = %task_id,
                turns = result.turns_used,
                duration_ms = result.duration_ms,
                "task completed"
            );
            let summary = truncate(&result.output, 2000);
            record_task_complete(&db_path, &task_id, &summary).await;

            {
                let mut current = progress.write().await;
                current.status = AgentProgressStatus::Complete;
                current.current_action = None;
                current.completion_summary = Some(summary.clone());
            }

            *status.write().await = TeammateStatus::Completed {
                task_id: task_id.clone(),
                result: summary,
            };
        } else {
            let error_msg = result
                .error
                .clone()
                .unwrap_or_else(|| "unknown error".to_string());

            warn!(
                teammate = %config.name,
                task_id = %task_id,
                error = %error_msg,
                "task failed"
            );
            record_task_failed(&db_path, &task_id, &error_msg).await;

            {
                let mut current = progress.write().await;
                current.status = AgentProgressStatus::Failed;
            }

            *status.write().await = TeammateStatus::Failed {
                task_id: task_id.clone(),
                error: error_msg,
            };
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_teammate_subagent_task(
    task_id: &str,
    task_description: &str,
    teammate_name: &str,
    working_dir: PathBuf,
    policy: DelegationPolicy,
    process_registry: Option<Arc<ProcessRegistry>>,
    process_owner_id: Option<String>,
    session_id: &str,
    identity: AgentIdentity,
) -> SubAgentTask {
    let parent_session_id = (!session_id.trim().is_empty()).then(|| session_id.to_string());
    SubAgentTask::new(task_id, task_description)
        .with_name(teammate_name)
        .with_identity(identity)
        .with_working_dir(working_dir)
        .with_delegation_policy(policy)
        .with_process_context(process_registry, process_owner_id, parent_session_id)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut boundary = max_chars.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text[..boundary].to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{build_teammate_subagent_task, truncate};
    use crate::agent::autonomy::team::TeammateRole;
    use crate::agent::subagent::AgentIdentity;
    use crate::process::ProcessRegistry;
    use crate::tools::registry::PermissionMode;

    #[test]
    fn truncate_respects_char_boundaries() {
        let ascii = "hello world";
        assert_eq!(truncate(ascii, 5), "hello");

        let multibyte = "cafe\u{0301}";
        let result = truncate(multibyte, 5);
        assert!(result.len() <= 5);
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn truncate_returns_full_string_when_short() {
        assert_eq!(truncate("hi", 100), "hi");
    }

    #[test]
    fn teammate_task_inherits_available_process_context() {
        let registry = Arc::new(ProcessRegistry::new());
        let task = build_teammate_subagent_task(
            "task-1",
            "verify the preview",
            "tester-1",
            std::path::PathBuf::from("/workspace"),
            TeammateRole::Tester.delegation_policy(PermissionMode::Autonomous, Some(10)),
            Some(Arc::clone(&registry)),
            Some("owner-1".to_string()),
            "session-1",
            AgentIdentity::child("tester-1", "/root/team", "tester-1", "tester", 0),
        );

        assert!(Arc::ptr_eq(
            task.process_registry.as_ref().expect("shared registry"),
            &registry
        ));
        assert_eq!(task.process_owner_id.as_deref(), Some("owner-1"));
        assert_eq!(task.parent_session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn teammate_task_does_not_invent_process_or_session_context() {
        let task = build_teammate_subagent_task(
            "task-1",
            "review files",
            "reviewer-1",
            std::path::PathBuf::from("/workspace"),
            TeammateRole::Reviewer.delegation_policy(PermissionMode::Autonomous, None),
            None,
            None,
            "",
            AgentIdentity::child("reviewer-1", "/root/team", "reviewer-1", "reviewer", 0),
        );

        assert!(task.process_registry.is_none());
        assert!(task.process_owner_id.is_none());
        assert!(task.parent_session_id.is_none());
    }
}
