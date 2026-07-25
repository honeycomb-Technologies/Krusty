use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::str::FromStr;
use std::time::Duration;

use crate::ai::providers::ProviderId;
use crate::ai::types::AiTool;

/// Default tool execution timeout (2 minutes)
pub(crate) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(120);
/// Delegated audit/build tools can legitimately run much longer than generic reads.
pub(crate) const DELEGATED_TOOL_TIMEOUT: Duration = Duration::from_secs(900);

/// Maximum number of function tools in the default coding-agent request.
pub const DEFAULT_CODE_TOOL_LIMIT: usize = 10;

const STANDARD_CODE_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "agent",
    "apply_patch",
    "bash",
    "enter_plan_mode",
    "glob",
    "grep",
    "read",
    "tool_search",
];

const STANDARD_EDIT_WRITE_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "agent",
    "bash",
    "edit",
    "enter_plan_mode",
    "glob",
    "grep",
    "read",
    "tool_search",
    "write",
];

const PLAN_MODE_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "agent",
    "glob",
    "grep",
    "read",
    "set_work_mode",
    "tool_search",
];

const ACTIVE_PLAN_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "add_subtask",
    "agent",
    "apply_patch",
    "bash",
    "read",
    "set_dependency",
    "set_work_mode",
    "task_complete",
    "task_start",
    "tool_search",
];

const ACTIVE_PLAN_EDIT_WRITE_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "add_subtask",
    "agent",
    "bash",
    "edit",
    "read",
    "set_dependency",
    "set_work_mode",
    "task_complete",
    "task_start",
    "tool_search",
    "write",
];

/// Active implementation plans add canonical lifecycle tools to the normal
/// coding surface. Keeping these direct is intentional: lifecycle and user
/// interaction tools are intercepted by the orchestrator and cannot be safely
/// emulated by nested registry dispatch.
const ACTIVE_PLAN_TOOL_LIMIT: usize = 11;
const ACTIVE_PLAN_EDIT_WRITE_TOOL_LIMIT: usize = 12;

/// Mutation grammar exposed directly to a model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MutationToolSurface {
    /// GPT/Codex models reliably produce the structured multi-file patch grammar.
    #[default]
    ApplyPatch,
    /// Grok, Claude, Gemini, Kimi, and generic models are more reliable with
    /// exact replacement plus whole-file creation tools.
    EditWrite,
}

impl MutationToolSurface {
    pub fn for_model(provider: ProviderId, model_id: &str) -> Self {
        if provider == ProviderId::Grok {
            return Self::EditWrite;
        }

        let model = model_id.trim().to_ascii_lowercase();
        if provider == ProviderId::OpenAI
            || model.contains("codex")
            || model.starts_with("gpt-")
            || model.contains("/gpt-")
            || model.starts_with("o1")
            || model.starts_with("o3")
            || model.starts_with("o4")
        {
            Self::ApplyPatch
        } else {
            Self::EditWrite
        }
    }
}

/// Request-time exposure policy for the primary coding surface.
///
/// The registry may contain dozens of built-in, MCP, extension, and plugin
/// tools. Keeping only the high-frequency surface on the wire improves prompt
/// caching and tool choice. `tool_search` preserves governed access to regular
/// specialist tools without serializing every schema on every request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRequestPolicy {
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub active_plan: bool,
    pub supervised_approval_available: bool,
    pub mutation_surface: MutationToolSurface,
    disabled_tools: HashSet<String>,
}

impl Default for ToolRequestPolicy {
    fn default() -> Self {
        Self::code(PermissionMode::Autonomous, false, false, true, &[])
    }
}

impl ToolRequestPolicy {
    pub fn code(
        permission_mode: PermissionMode,
        plan_mode: bool,
        active_plan: bool,
        supervised_approval_available: bool,
        disabled_tools: &[String],
    ) -> Self {
        Self {
            permission_mode,
            plan_mode,
            active_plan,
            supervised_approval_available,
            mutation_surface: MutationToolSurface::ApplyPatch,
            disabled_tools: disabled_tools.iter().cloned().collect(),
        }
    }

    pub fn with_mutation_surface(mut self, mutation_surface: MutationToolSurface) -> Self {
        self.mutation_surface = mutation_surface;
        self
    }

    pub fn filter(&self, tools: Vec<AiTool>) -> Vec<AiTool> {
        let (selected, limit): (&[&str], usize) = if self.plan_mode {
            (PLAN_MODE_TOOLS, DEFAULT_CODE_TOOL_LIMIT)
        } else if self.active_plan {
            match self.mutation_surface {
                MutationToolSurface::ApplyPatch => (ACTIVE_PLAN_TOOLS, ACTIVE_PLAN_TOOL_LIMIT),
                MutationToolSurface::EditWrite => (
                    ACTIVE_PLAN_EDIT_WRITE_TOOLS,
                    ACTIVE_PLAN_EDIT_WRITE_TOOL_LIMIT,
                ),
            }
        } else {
            match self.mutation_surface {
                MutationToolSurface::ApplyPatch => (STANDARD_CODE_TOOLS, DEFAULT_CODE_TOOL_LIMIT),
                MutationToolSurface::EditWrite => {
                    (STANDARD_EDIT_WRITE_TOOLS, DEFAULT_CODE_TOOL_LIMIT)
                }
            }
        };

        let mut filtered = tools
            .into_iter()
            .filter(|tool| selected.contains(&tool.name.as_str()))
            .filter(|tool| !self.disabled_tools.contains(&tool.name))
            .filter(|tool| {
                let policy = tool_policy(&tool.name);
                !self.plan_mode || policy.allowed_in_plan_mode
            })
            .filter(|tool| {
                self.permission_mode != PermissionMode::Supervised
                    || self.supervised_approval_available
                    || (!tool_policy(&tool.name).requires_supervised_approval
                        && !matches!(tool.name.as_str(), "agent" | "tool_search"))
            })
            .collect::<Vec<_>>();

        filtered.sort_by(|left, right| left.name.cmp(&right.name));
        filtered.truncate(limit);
        filtered
    }

    pub fn is_disabled(&self, tool_name: &str) -> bool {
        self.disabled_tools.contains(tool_name)
    }
}

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
    /// Immutable tool-capability ceiling inherited from an explicit parent
    /// execution scope. `None` preserves the ordinary governed delegation
    /// surface; `Some(empty)` deliberately creates a tool-free child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_tool_allowlist: Option<BTreeSet<String>>,
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
            execution_tool_allowlist: None,
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
            execution_tool_allowlist: None,
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
            execution_tool_allowlist: None,
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
            execution_tool_allowlist: None,
        }
    }

    /// Intersect delegated execution with the parent's exact per-turn tool
    /// scope. The parent scope is already the narrowest run-level capability,
    /// so a child may retain only names present in it; it must never reconstruct
    /// the default build/explore surface.
    pub fn with_execution_tool_allowlist(
        mut self,
        execution_tool_allowlist: Option<&HashSet<String>>,
    ) -> Self {
        self.execution_tool_allowlist = execution_tool_allowlist
            .map(|allowlist| allowlist.iter().cloned().collect::<BTreeSet<String>>());
        self
    }

    pub fn authorize_tool(&self, tool_name: &str, plan_mode: bool) -> Result<(), String> {
        self.authorize_non_recursive_tool(tool_name)?;
        self.authorize_execution_scope(tool_name, tool_name)?;
        self.authorize_tool_policy(tool_name, tool_policy(tool_name), plan_mode)
    }

    pub fn authorize_tool_call(
        &self,
        tool_name: &str,
        params: &Value,
        plan_mode: bool,
    ) -> Result<(), String> {
        let (effective_name, effective_params) = effective_tool_call(tool_name, params);
        self.authorize_non_recursive_tool(tool_name)?;
        self.authorize_non_recursive_tool(effective_name)?;
        self.authorize_execution_scope(tool_name, effective_name)?;
        self.authorize_tool_policy(
            effective_name,
            tool_policy_for_call(effective_name, effective_params),
            plan_mode,
        )
    }

    fn authorize_execution_scope(
        &self,
        wrapper_name: &str,
        effective_name: &str,
    ) -> Result<(), String> {
        let Some(allowlist) = self.execution_tool_allowlist.as_ref() else {
            return Ok(());
        };

        if allowlist.contains(wrapper_name) && allowlist.contains(effective_name) {
            return Ok(());
        }

        Err(format!(
            "Delegated policy blocked tool '{}': it exceeds the parent run's explicit tool capability",
            effective_name
        ))
    }

    fn authorize_non_recursive_tool(&self, tool_name: &str) -> Result<(), String> {
        let is_subagent = matches!(
            self.surface,
            DelegationSurface::SubagentExplore
                | DelegationSurface::SubagentBuild
                | DelegationSurface::SubagentPlan
                | DelegationSurface::SubagentVerify
        );
        if is_subagent
            && matches!(
                tool_name,
                "agent" | "skill" | "enter_plan_mode" | "set_work_mode" | "set_workspace_context"
            )
        {
            return Err(format!(
                "Delegated policy blocked tool '{}': delegated agents cannot recursively delegate or change parent runtime mode",
                tool_name
            ));
        }
        Ok(())
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
            "execution_tool_allowlist": self.execution_tool_allowlist,
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
        "tool_search" => tool_search_policy(params),
        _ => tool_policy(name),
    }
}

/// Resolve the user-visible operation represented by a wrapper call.
///
/// Approval UIs retain the wrapper call ID for protocol continuity, but must
/// show the effective target and target arguments so consent is informed.
pub fn effective_tool_call<'a>(name: &'a str, params: &'a Value) -> (&'a str, &'a Value) {
    if name == "tool_search" && params.get("action").and_then(Value::as_str) == Some("execute") {
        if let Some(target) = params.get("tool").and_then(Value::as_str) {
            if target != "tool_search" {
                return (target, params.get("arguments").unwrap_or(&Value::Null));
            }
        }
    }

    (name, params)
}

fn tool_search_policy(params: &Value) -> ToolPolicy {
    if params.get("action").and_then(Value::as_str) != Some("execute") {
        return ToolPolicy::read_only();
    }

    let Some(target) = params.get("tool").and_then(Value::as_str) else {
        return ToolPolicy::read_only();
    };
    if target == "tool_search" {
        return ToolPolicy::read_only();
    }

    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    tool_policy_for_call(target, arguments)
}

fn agent_tool_policy(params: &Value) -> ToolPolicy {
    match agent_call_action(params) {
        "message" | "followup" | "interrupt" => ToolPolicy::interactive(),
        "list" | "status" | "wait" => ToolPolicy::read_only_with_timeout(DELEGATED_TOOL_TIMEOUT),
        "resume" => ToolPolicy::write_with_timeout(DELEGATED_TOOL_TIMEOUT),
        _ if agent_call_requests_write(params) => {
            ToolPolicy::write_with_timeout(DELEGATED_TOOL_TIMEOUT)
        }
        _ => ToolPolicy::read_only_with_timeout(DELEGATED_TOOL_TIMEOUT),
    }
}

pub fn agent_call_action(params: &Value) -> &str {
    params
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("spawn")
}

pub fn agent_call_starts_run(params: &Value) -> bool {
    matches!(agent_call_action(params), "spawn" | "resume")
}

pub fn agent_call_requests_write(params: &Value) -> bool {
    agent_call_action(params) == "resume"
        || ["profile", "agent_type"]
            .iter()
            .any(|field| params.get(field).and_then(Value::as_str) == Some("build"))
        || agent_call_has_capability(params, "write")
}

pub fn agent_call_execution_profile(params: &Value) -> &'static str {
    let profile = params
        .get("profile")
        .and_then(Value::as_str)
        .or_else(|| params.get("agent_type").and_then(Value::as_str));
    match profile {
        Some("plan") => "plan",
        Some("verify") => "verify",
        Some("build") => "build",
        Some("explore") => "explore",
        _ if agent_call_requests_write(params) => "build",
        _ if agent_call_has_capability(params, "execute") => "verify",
        _ => "explore",
    }
}

fn agent_call_has_capability(params: &Value, expected: &str) -> bool {
    params
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .filter_map(Value::as_str)
                .any(|capability| capability == expected)
        })
}

/// Resolve the canonical policy for a tool name.
pub fn tool_policy(name: &str) -> ToolPolicy {
    match name {
        "agent" => ToolPolicy::read_only_with_timeout(DELEGATED_TOOL_TIMEOUT),
        "read"
        | "glob"
        | "grep"
        | "list"
        | "web_search"
        | "web_fetch"
        | "skill"
        | "tool_search"
        | "mcp__list_resources"
        | "mcp__list_resource_templates"
        | "mcp__read_resource"
        | "mcp__list_prompts"
        | "mcp__get_prompt"
        | "mcp__list_tools" => ToolPolicy::read_only(),
        "AskUserQuestion" | "PlanConfirm" | "enter_plan_mode" | "memory" | "set_work_mode"
        | "task_start" | "task_complete" | "add_subtask" | "set_dependency"
        | "send_user_message" | "sleep" | "autonomous_task" | "report" => ToolPolicy::interactive(),
        "set_workspace_context" => ToolPolicy::interactive_with_supervised_approval(),
        _ => ToolPolicy::write(),
    }
}
