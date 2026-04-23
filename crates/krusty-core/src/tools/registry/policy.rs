use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Default tool execution timeout (2 minutes)
pub(crate) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(120);
/// Delegated audit/build tools can legitimately run much longer than generic reads.
pub(crate) const DELEGATED_TOOL_TIMEOUT: Duration = Duration::from_secs(900);

/// Tool category for permission checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    /// Read-only tools that never modify state.
    ReadOnly,
    /// Write tools that modify files, execute commands, etc.
    Write,
    /// Interactive tools that require user input.
    Interactive,
}

/// Centralized policy contract for a tool name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPolicy {
    pub category: ToolCategory,
    pub requires_supervised_approval: bool,
    pub retry_timeout_once: bool,
    pub allowed_in_plan_mode: bool,
    pub timeout_override: Option<Duration>,
}

impl ToolPolicy {
    const fn read_only() -> Self {
        Self {
            category: ToolCategory::ReadOnly,
            requires_supervised_approval: false,
            retry_timeout_once: true,
            allowed_in_plan_mode: true,
            timeout_override: None,
        }
    }

    const fn read_only_with_timeout(timeout_override: Duration) -> Self {
        Self {
            category: ToolCategory::ReadOnly,
            requires_supervised_approval: false,
            retry_timeout_once: true,
            allowed_in_plan_mode: true,
            timeout_override: Some(timeout_override),
        }
    }

    const fn interactive() -> Self {
        Self {
            category: ToolCategory::Interactive,
            requires_supervised_approval: false,
            retry_timeout_once: false,
            allowed_in_plan_mode: true,
            timeout_override: None,
        }
    }

    const fn write() -> Self {
        Self {
            category: ToolCategory::Write,
            requires_supervised_approval: true,
            retry_timeout_once: false,
            allowed_in_plan_mode: false,
            timeout_override: None,
        }
    }
}

/// Permission mode for tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    #[default]
    Supervised,
    Autonomous,
}

/// Delegated execution surface type for governance/audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationSurface {
    SubagentExplore,
    SubagentBuild,
    SubagentPlan,
    SubagentVerify,
    McpRemote,
    Skill,
    Extension,
}

/// Inherited delegated execution policy from the parent orchestrator/tool context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationPolicy {
    pub surface: DelegationSurface,
    pub inherited_permission_mode: PermissionMode,
    pub max_turns: Option<usize>,
    pub read_only_only: bool,
    pub bash_allowed: bool,
}

impl DelegationPolicy {
    pub fn for_subagent_explore(
        inherited_permission_mode: PermissionMode,
        max_turns: Option<usize>,
    ) -> Self {
        Self {
            surface: DelegationSurface::SubagentExplore,
            inherited_permission_mode,
            max_turns,
            read_only_only: true,
            bash_allowed: false,
        }
    }

    pub fn for_subagent_build(
        inherited_permission_mode: PermissionMode,
        max_turns: Option<usize>,
    ) -> Self {
        Self {
            surface: DelegationSurface::SubagentBuild,
            inherited_permission_mode,
            max_turns,
            read_only_only: false,
            bash_allowed: false,
        }
    }

    pub fn for_subagent_plan(
        inherited_permission_mode: PermissionMode,
        max_turns: Option<usize>,
    ) -> Self {
        Self {
            surface: DelegationSurface::SubagentPlan,
            inherited_permission_mode,
            max_turns,
            read_only_only: true,
            bash_allowed: false,
        }
    }

    pub fn for_subagent_verify(
        inherited_permission_mode: PermissionMode,
        max_turns: Option<usize>,
    ) -> Self {
        Self {
            surface: DelegationSurface::SubagentVerify,
            inherited_permission_mode,
            max_turns,
            read_only_only: true,
            bash_allowed: true,
        }
    }

    pub fn authorize_tool(&self, tool_name: &str, plan_mode: bool) -> Result<(), String> {
        let policy = tool_policy(tool_name);
        if self.read_only_only
            && policy.category != ToolCategory::ReadOnly
            && !(self.bash_allowed && tool_name == "bash")
        {
            return Err(format!(
                "Delegated policy blocked tool '{}': {} only permits read-only tools",
                tool_name,
                self.surface_name()
            ));
        }
        if self.inherited_permission_mode == PermissionMode::Supervised
            && policy.requires_supervised_approval
        {
            return Err(format!(
                "Delegated policy blocked tool '{}': supervised parent requires approval for write-capable tools",
                tool_name
            ));
        }
        if plan_mode && !policy.allowed_in_plan_mode {
            return Err(format!(
                "Delegated policy blocked tool '{}': tool is disallowed in plan mode",
                tool_name
            ));
        }
        Ok(())
    }

    pub fn surface_name(&self) -> &'static str {
        match self.surface {
            DelegationSurface::SubagentExplore => "subagent_explore",
            DelegationSurface::SubagentBuild => "subagent_build",
            DelegationSurface::SubagentPlan => "subagent_plan",
            DelegationSurface::SubagentVerify => "subagent_verify",
            DelegationSurface::McpRemote => "mcp_remote",
            DelegationSurface::Skill => "skill",
            DelegationSurface::Extension => "extension",
        }
    }

    pub fn audit_json(&self) -> Value {
        serde_json::json!({
            "surface": self.surface_name(),
            "permission_mode": self.inherited_permission_mode,
            "max_turns": self.max_turns,
            "read_only_only": self.read_only_only,
            "bash_allowed": self.bash_allowed,
        })
    }
}

/// Categorize a tool by name.
pub fn tool_category(name: &str) -> ToolCategory {
    tool_policy(name).category
}

/// Resolve the canonical policy for a tool.
pub fn tool_policy(name: &str) -> ToolPolicy {
    match name {
        "agent" => ToolPolicy::read_only_with_timeout(DELEGATED_TOOL_TIMEOUT),
        "read" | "glob" | "grep" | "list" | "web_search" | "web_fetch" | "skill" => {
            ToolPolicy::read_only()
        }
        "AskUserQuestion"
        | "PlanConfirm"
        | "enter_plan_mode"
        | "memory"
        | "set_work_mode"
        | "set_workspace_context"
        | "task_start"
        | "task_complete"
        | "add_subtask"
        | "set_dependency"
        | "send_user_message"
        | "sleep"
        | "autonomous_task"
        | "report" => ToolPolicy::interactive(),
        _ => ToolPolicy::write(),
    }
}
