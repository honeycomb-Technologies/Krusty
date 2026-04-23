use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::tools::registry::DelegationPolicy;

/// Real-time progress update from a sub-agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentProgress {
    /// First-class delegated runtime unit identifier.
    pub delegated_run_id: Option<String>,
    /// Agent task ID.
    pub task_id: String,
    /// Display name (derived from task context).
    pub name: String,
    /// Current status.
    pub status: AgentProgressStatus,
    /// Number of tool calls made.
    pub tool_count: usize,
    /// Approximate token usage.
    pub tokens: usize,
    /// Current action description (e.g. "reading app.rs").
    pub current_action: Option<String>,
    /// Short completion summary when the sub-agent finishes a delegated task.
    pub completion_summary: Option<String>,
    /// Lines added (for build agents).
    pub lines_added: usize,
    /// Lines removed (for build agents).
    pub lines_removed: usize,
    /// Plan task ID completed (for auto-marking tasks).
    pub completed_plan_task: Option<String>,
}

/// Status of a sub-agent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AgentProgressStatus {
    /// Agent is running.
    #[default]
    Running,
    /// Agent completed successfully.
    Complete,
    /// Agent failed.
    Failed,
}

/// Configuration for a sub-agent task.
///
/// The model to use is determined by `SubAgentPool.override_model`, not by the task.
/// This provides a provider-agnostic experience where all sub-agents use the user's
/// current model.
#[derive(Debug, Clone)]
pub struct SubAgentTask {
    pub id: String,
    /// Display name for the agent (e.g. "tui", "agent", "main").
    pub name: String,
    pub prompt: String,
    pub working_dir: PathBuf,
    /// Stable delegated runtime unit identifier shared by all tasks in one parent invocation.
    pub delegated_run_id: Option<String>,
    /// Plan task ID this agent completes (for auto-marking).
    pub plan_task_id: Option<String>,
    /// Whether thinking/reasoning is enabled for this agent.
    pub thinking_enabled: bool,
    /// Inherited delegated execution policy from parent tool context.
    pub delegation_policy: Option<DelegationPolicy>,
    /// Optional per-task turn budget inherited from parent.
    pub max_turns_override: Option<usize>,
}

impl SubAgentTask {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        let id = id.into();
        let name = id.clone();
        Self {
            id,
            name,
            prompt: prompt.into(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            delegated_run_id: None,
            plan_task_id: None,
            thinking_enabled: false,
            delegation_policy: None,
            max_turns_override: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = dir;
        self
    }

    pub fn with_delegated_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.delegated_run_id = Some(run_id.into());
        self
    }

    pub fn with_plan_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.plan_task_id = Some(task_id.into());
        self
    }

    pub fn with_thinking(mut self, enabled: bool) -> Self {
        self.thinking_enabled = enabled;
        self
    }

    pub fn with_delegation_policy(mut self, policy: DelegationPolicy) -> Self {
        self.delegation_policy = Some(policy);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns_override = Some(max_turns);
        self
    }
}
