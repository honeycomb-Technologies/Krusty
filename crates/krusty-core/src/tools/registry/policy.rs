use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;
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

/// Runtime authorization result for a concrete tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAuthorization {
    /// The tool call may execute immediately.
    Execute,
    /// The tool call may execute only after user approval.
    RequiresApproval,
    /// The tool call is blocked because it is not allowed in plan mode.
    BlockedInPlanMode,
}

impl ToolAuthorization {
    pub fn requires_approval(self) -> bool {
        self == Self::RequiresApproval
    }

    pub fn is_blocked(self) -> bool {
        self == Self::BlockedInPlanMode
    }
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

    const fn interactive_with_supervised_approval() -> Self {
        Self {
            category: ToolCategory::Interactive,
            requires_supervised_approval: true,
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

    const fn write_with_timeout(timeout_override: Duration) -> Self {
        Self {
            category: ToolCategory::Write,
            requires_supervised_approval: true,
            retry_timeout_once: false,
            allowed_in_plan_mode: false,
            timeout_override: Some(timeout_override),
        }
    }
}

/// Permission mode for tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    Supervised,
    #[default]
    Autonomous,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supervised => "supervised",
            Self::Autonomous => "autonomous",
        }
    }
}

impl FromStr for PermissionMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "supervised" => Ok(Self::Supervised),
            "autonomous" => Ok(Self::Autonomous),
            other => Err(format!("Unknown permission mode: {other}")),
        }
    }
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
        self.authorize_tool_policy(tool_name, tool_policy(tool_name), plan_mode)
    }

    pub fn authorize_tool_call(
        &self,
        tool_name: &str,
        params: &Value,
        plan_mode: bool,
    ) -> Result<(), String> {
        self.authorize_tool_policy(
            tool_name,
            tool_policy_for_call(tool_name, params),
            plan_mode,
        )
    }

    fn authorize_tool_policy(
        &self,
        tool_name: &str,
        policy: ToolPolicy,
        plan_mode: bool,
    ) -> Result<(), String> {
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

/// Authorize a concrete top-level tool call under the current runtime mode.
pub fn authorize_tool_call(
    name: &str,
    params: &Value,
    permission_mode: PermissionMode,
    plan_mode: bool,
) -> ToolAuthorization {
    let policy = tool_policy_for_call(name, params);
    if plan_mode && !policy.allowed_in_plan_mode {
        return ToolAuthorization::BlockedInPlanMode;
    }

    if permission_mode == PermissionMode::Supervised && policy.requires_supervised_approval {
        return ToolAuthorization::RequiresApproval;
    }

    ToolAuthorization::Execute
}

/// Categorize a tool by name.
pub fn tool_category(name: &str) -> ToolCategory {
    tool_policy(name).category
}

/// Resolve the canonical policy for a concrete tool call.
///
/// This extends the name-only policy with argument-aware intent detection for
/// polymorphic tools. In particular, `agent(agent_type = "build")` is
/// write-capable even though other agent subtypes are read-only delegations.
pub fn tool_policy_for_call(name: &str, params: &Value) -> ToolPolicy {
    match name {
        "agent" => agent_tool_policy(params),
        _ => tool_policy(name),
    }
}

fn agent_tool_policy(params: &Value) -> ToolPolicy {
    match params.get("agent_type").and_then(Value::as_str) {
        Some("explore" | "plan" | "verify") => {
            ToolPolicy::read_only_with_timeout(DELEGATED_TOOL_TIMEOUT)
        }
        Some("build") => ToolPolicy::write_with_timeout(DELEGATED_TOOL_TIMEOUT),
        _ => ToolPolicy::write_with_timeout(DELEGATED_TOOL_TIMEOUT),
    }
}

/// Resolve the canonical policy for a tool name.
pub fn tool_policy(name: &str) -> ToolPolicy {
    match name {
        "agent" => ToolPolicy::read_only_with_timeout(DELEGATED_TOOL_TIMEOUT),
        "read" | "glob" | "grep" | "list" | "web_search" | "web_fetch" | "skill" => {
            ToolPolicy::read_only()
        }
        "AskUserQuestion" | "PlanConfirm" | "enter_plan_mode" | "memory" | "set_work_mode"
        | "task_start" | "task_complete" | "add_subtask" | "set_dependency"
        | "send_user_message" | "sleep" | "autonomous_task" | "report" => ToolPolicy::interactive(),
        "set_workspace_context" => ToolPolicy::interactive_with_supervised_approval(),
        _ => ToolPolicy::write(),
    }
}
