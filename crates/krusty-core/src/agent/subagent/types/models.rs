use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::agent::subagent::AgentIdentity;
use crate::agent::subagent::AgentMailbox;
use crate::agent::ProviderCallTraceContext;
use crate::process::ProcessRegistry;
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
    /// Stable runtime identity. `name` remains the semantic task label while
    /// the identity carries the creature-themed display name.
    pub identity: Option<AgentIdentity>,
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
    /// Stable runtime identity kept separate from semantic task fields.
    pub identity: Option<AgentIdentity>,
    pub prompt: String,
    pub working_dir: PathBuf,
    /// Filesystem sandbox root inherited from the parent tool context.
    pub sandbox_root: Option<PathBuf>,
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
    /// Shared process registry inherited from the parent runtime. Delegated
    /// background commands must remain visible and controllable from the
    /// originating session instead of falling back to detached shell handles.
    pub process_registry: Option<Arc<ProcessRegistry>>,
    /// Parent owner key used to preserve multi-tenant process isolation.
    pub process_owner_id: Option<String>,
    /// Parent session used for tool-output scoping and delegated provenance.
    pub parent_session_id: Option<String>,
    /// Parent run's canonical provider-call accounting sink.
    pub provider_call_trace: Option<ProviderCallTraceContext>,
    /// Parent-to-child steering delivered between model turns.
    pub mailbox: Option<AgentMailbox>,
}

impl SubAgentTask {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        let id = id.into();
        let name = id.clone();
        Self {
            id,
            name,
            identity: None,
            prompt: prompt.into(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            sandbox_root: None,
            delegated_run_id: None,
            plan_task_id: None,
            thinking_enabled: false,
            delegation_policy: None,
            max_turns_override: None,
            process_registry: None,
            process_owner_id: None,
            parent_session_id: None,
            provider_call_trace: None,
            mailbox: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_identity(mut self, identity: AgentIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    pub fn ensure_identity(&mut self, parent_path: &str, role: &str, ordinal: usize) {
        if self.identity.is_none() {
            self.identity = Some(AgentIdentity::child(
                self.id.clone(),
                parent_path,
                self.name.clone(),
                role,
                ordinal,
            ));
        }
    }

    pub fn display_name(&self) -> String {
        self.identity
            .as_ref()
            .map_or_else(|| self.name.clone(), AgentIdentity::display_name)
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = dir;
        self
    }

    pub fn with_sandbox_root(mut self, sandbox_root: PathBuf) -> Self {
        self.sandbox_root = Some(sandbox_root);
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

    pub fn with_mailbox(mut self, mailbox: AgentMailbox) -> Self {
        self.mailbox = Some(mailbox);
        self
    }

    pub fn with_provider_call_trace(
        mut self,
        provider_call_trace: Option<ProviderCallTraceContext>,
    ) -> Self {
        self.provider_call_trace = provider_call_trace;
        self
    }

    pub fn with_process_context(
        mut self,
        process_registry: Option<Arc<ProcessRegistry>>,
        process_owner_id: Option<String>,
        parent_session_id: Option<String>,
    ) -> Self {
        self.process_registry = process_registry;
        self.process_owner_id = process_owner_id;
        self.parent_session_id = parent_session_id;
        self
    }
}
