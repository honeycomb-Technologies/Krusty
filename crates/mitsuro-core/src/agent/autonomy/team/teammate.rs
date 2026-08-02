use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::agent::subagent::AgentIdentity;
use crate::agent::subagent::AgentProgress;
use crate::tools::registry::{DelegationPolicy, DelegationSurface, PermissionMode};

#[derive(Debug, Clone)]
pub struct TeammateConfig {
    pub name: String,
    pub role: TeammateRole,
    pub max_turns: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeammateRole {
    Builder,
    Reviewer,
    Tester,
}

impl TeammateRole {
    pub fn delegation_policy(
        &self,
        inherited_permission_mode: PermissionMode,
        max_turns: Option<usize>,
    ) -> DelegationPolicy {
        match self {
            Self::Builder => DelegationPolicy {
                surface: DelegationSurface::SubagentBuild,
                inherited_permission_mode,
                supervised_approval_granted: false,
                max_turns,
                read_only_only: false,
                bash_allowed: false,
                execution_tool_allowlist: None,
            },
            Self::Reviewer | Self::Tester => DelegationPolicy {
                surface: DelegationSurface::SubagentVerify,
                inherited_permission_mode,
                supervised_approval_granted: false,
                max_turns,
                read_only_only: true,
                bash_allowed: true,
                execution_tool_allowlist: None,
            },
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Builder => "builder",
            Self::Reviewer => "reviewer",
            Self::Tester => "tester",
        }
    }
}

impl std::fmt::Display for TeammateRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone)]
pub enum TeammateStatus {
    Idle,
    Working { task_id: String },
    Completed { task_id: String, result: String },
    Failed { task_id: String, error: String },
    Cancelled,
}

impl TeammateStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working { .. } => "working",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for TeammateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Working { task_id } => write!(f, "working on {task_id}"),
            Self::Completed { task_id, .. } => write!(f, "completed {task_id}"),
            Self::Failed { task_id, error } => write!(f, "failed {task_id}: {error}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

pub struct Teammate {
    pub config: TeammateConfig,
    pub identity: AgentIdentity,
    pub status: Arc<RwLock<TeammateStatus>>,
    pub cancel_token: CancellationToken,
    pub progress: Arc<RwLock<AgentProgress>>,
    task_handle: Option<JoinHandle<()>>,
}

impl Teammate {
    pub fn new(config: TeammateConfig, identity: AgentIdentity) -> Self {
        Self {
            status: Arc::new(RwLock::new(TeammateStatus::Idle)),
            cancel_token: CancellationToken::new(),
            progress: Arc::new(RwLock::new(AgentProgress::default())),
            config,
            identity,
            task_handle: None,
        }
    }

    pub fn set_handle(&mut self, handle: JoinHandle<()>) {
        self.task_handle = Some(handle);
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    pub async fn shutdown(mut self) {
        self.cancel_token.cancel();
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_policy_allows_writes() {
        let policy = TeammateRole::Builder.delegation_policy(PermissionMode::Autonomous, Some(30));
        assert!(!policy.read_only_only);
        assert!(!policy.bash_allowed);
        assert_eq!(policy.surface, DelegationSurface::SubagentBuild);
        assert_eq!(policy.max_turns, Some(30));
    }

    #[test]
    fn reviewer_policy_is_read_only_with_bash() {
        let policy = TeammateRole::Reviewer.delegation_policy(PermissionMode::Autonomous, None);
        assert!(policy.read_only_only);
        assert!(policy.bash_allowed);
        assert_eq!(policy.surface, DelegationSurface::SubagentVerify);
    }

    #[test]
    fn tester_policy_matches_reviewer() {
        let tester = TeammateRole::Tester.delegation_policy(PermissionMode::Autonomous, Some(20));
        let reviewer =
            TeammateRole::Reviewer.delegation_policy(PermissionMode::Autonomous, Some(20));
        assert_eq!(tester.read_only_only, reviewer.read_only_only);
        assert_eq!(tester.bash_allowed, reviewer.bash_allowed);
        assert_eq!(tester.surface, reviewer.surface);
    }

    #[test]
    fn supervised_parent_cannot_create_autonomous_teammate() {
        for role in [
            TeammateRole::Builder,
            TeammateRole::Reviewer,
            TeammateRole::Tester,
        ] {
            let policy = role.delegation_policy(PermissionMode::Supervised, Some(12));
            assert_eq!(policy.inherited_permission_mode, PermissionMode::Supervised);
        }
    }

    #[test]
    fn status_labels_are_stable() {
        assert_eq!(TeammateStatus::Idle.label(), "idle");
        assert_eq!(
            TeammateStatus::Working {
                task_id: "t1".into()
            }
            .label(),
            "working"
        );
        assert_eq!(
            TeammateStatus::Completed {
                task_id: "t1".into(),
                result: String::new()
            }
            .label(),
            "completed"
        );
        assert_eq!(
            TeammateStatus::Failed {
                task_id: "t1".into(),
                error: "boom".into()
            }
            .label(),
            "failed"
        );
        assert_eq!(TeammateStatus::Cancelled.label(), "cancelled");
    }
}
