use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};

use crate::agent::loop_events::LoopEvent;
use crate::agent::subagent::AgentProgress;
use crate::ai::client::AiClient;
use crate::ai::types::ModelMessage;
use crate::mcp::McpManager;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::storage::WorkspaceMode;
use crate::tools::git_identity::GitIdentity;

use super::{DelegationPolicy, PermissionMode, ToolRegistry};

/// Output chunk from a streaming tool (like bash)
#[derive(Debug, Clone)]
pub struct ToolOutputChunk {
    pub tool_use_id: String,
    pub chunk: String,
    pub is_complete: bool,
    pub exit_code: Option<i32>,
}

/// Context for tool execution
pub struct ToolContext {
    pub working_dir: std::path::PathBuf,
    pub project_dir: Option<std::path::PathBuf>,
    pub workspace_mode: WorkspaceMode,
    pub session_id: Option<String>,
    pub db_path: Option<std::path::PathBuf>,
    /// Sandbox root for multi-tenant path isolation (e.g., /workspaces/{user_id})
    /// If set, all file operations must be within this directory.
    pub sandbox_root: Option<std::path::PathBuf>,
    /// User ID for multi-tenant operation scoping (processes, etc.)
    pub user_id: Option<String>,
    pub process_registry: Option<Arc<ProcessRegistry>>,
    pub skills_manager: Option<Arc<RwLock<SkillsManager>>>,
    pub mcp_manager: Option<Arc<McpManager>>,
    /// Optional per-call timeout override
    pub timeout: Option<Duration>,
    /// Channel for streaming output (used by bash tool)
    pub output_tx: Option<mpsc::UnboundedSender<ToolOutputChunk>>,
    /// Tool use ID for streaming output
    pub tool_use_id: Option<String>,
    /// Whether plan mode is active (restricts write tools)
    pub plan_mode: bool,
    /// Channel for delegated agent progress updates (explore, build, plan, verify)
    pub agent_progress_tx: Option<mpsc::UnboundedSender<AgentProgress>>,
    /// Current user-selected model (for non-Anthropic providers, subagents use this)
    pub current_model: Option<String>,
    /// Session-scoped AI client (used by tools that spawn sub-agents)
    pub ai_client: Option<Arc<AiClient>>,
    /// Git identity for commit attribution
    pub git_identity: Option<GitIdentity>,
    /// Parent execution permission mode inherited into delegated surfaces.
    pub permission_mode: PermissionMode,
    /// Optional delegated sub-agent turn budget inherited from parent config.
    pub subagent_max_turns: Option<usize>,
    /// Optional delegated execution policy contract for downstream calls.
    pub delegation_policy: Option<DelegationPolicy>,
    /// Tool registry for subagent delegation (explore/build use this to give subagents real tools).
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Parent conversation context for delegated agents that need upstream history.
    pub parent_conversation: Option<Arc<Vec<ModelMessage>>>,
    /// Canonical loop-event sink for hooks/tools that need to surface runtime events.
    pub loop_event_tx: Option<mpsc::UnboundedSender<LoopEvent>>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            project_dir: None,
            workspace_mode: WorkspaceMode::Neutral,
            session_id: None,
            db_path: None,
            sandbox_root: None,
            user_id: None,
            process_registry: None,
            skills_manager: None,
            mcp_manager: None,
            timeout: None,
            output_tx: None,
            tool_use_id: None,
            plan_mode: false,
            agent_progress_tx: None,
            current_model: None,
            ai_client: None,
            git_identity: None,
            permission_mode: PermissionMode::Supervised,
            subagent_max_turns: None,
            delegation_policy: None,
            tool_registry: None,
            parent_conversation: None,
            loop_event_tx: None,
        }
    }
}

impl ToolContext {
    /// Create a new tool context with process registry
    pub fn with_process_registry(
        working_dir: std::path::PathBuf,
        process_registry: Arc<ProcessRegistry>,
    ) -> Self {
        Self {
            working_dir,
            process_registry: Some(process_registry),
            ..Default::default()
        }
    }

    pub fn with_workspace(
        mut self,
        project_dir: Option<std::path::PathBuf>,
        workspace_mode: WorkspaceMode,
    ) -> Self {
        self.project_dir = project_dir;
        self.workspace_mode = workspace_mode;
        self
    }

    pub fn with_session_metadata(
        mut self,
        session_id: String,
        db_path: std::path::PathBuf,
    ) -> Self {
        self.session_id = Some(session_id);
        self.db_path = Some(db_path);
        self
    }

    /// Set sandbox root for multi-tenant path isolation.
    pub fn with_sandbox(mut self, sandbox_root: std::path::PathBuf) -> Self {
        self.sandbox_root = Some(sandbox_root);
        self
    }

    /// Set user ID for multi-tenant operation scoping.
    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Add MCP manager to context
    pub fn with_mcp_manager(mut self, mcp_manager: Arc<McpManager>) -> Self {
        self.mcp_manager = Some(mcp_manager);
        self
    }

    /// Add skills manager to context
    pub fn with_skills_manager(mut self, skills_manager: Arc<RwLock<SkillsManager>>) -> Self {
        self.skills_manager = Some(skills_manager);
        self
    }

    /// Add streaming output channel to context
    pub fn with_output_stream(
        mut self,
        tx: mpsc::UnboundedSender<ToolOutputChunk>,
        tool_use_id: String,
    ) -> Self {
        self.output_tx = Some(tx);
        self.tool_use_id = Some(tool_use_id);
        self
    }

    /// Add delegated agent progress channel to context
    pub fn with_agent_progress(mut self, tx: mpsc::UnboundedSender<AgentProgress>) -> Self {
        self.agent_progress_tx = Some(tx);
        self
    }

    /// Set the current user-selected model (for non-Anthropic provider subagents)
    pub fn with_current_model(mut self, model: String) -> Self {
        self.current_model = Some(model);
        self
    }

    /// Add session-scoped AI client to context
    pub fn with_ai_client(mut self, client: Arc<AiClient>) -> Self {
        self.ai_client = Some(client);
        self
    }

    /// Set git identity for commit attribution
    pub fn with_git_identity(mut self, identity: GitIdentity) -> Self {
        self.git_identity = Some(identity);
        self
    }

    /// Set inherited permission mode.
    pub fn with_permission_mode(mut self, permission_mode: PermissionMode) -> Self {
        self.permission_mode = permission_mode;
        self
    }

    /// Set delegated sub-agent turn budget inherited from parent config.
    pub fn with_subagent_max_turns(mut self, max_turns: Option<usize>) -> Self {
        self.subagent_max_turns = max_turns;
        self
    }

    /// Attach delegated execution policy metadata.
    pub fn with_delegation_policy(mut self, policy: DelegationPolicy) -> Self {
        self.delegation_policy = Some(policy);
        self
    }

    /// Set tool registry for subagent delegation.
    pub fn with_tool_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    /// Set parent conversation context for delegated agents.
    pub fn with_parent_conversation(mut self, conv: Arc<Vec<ModelMessage>>) -> Self {
        self.parent_conversation = Some(conv);
        self
    }

    /// Attach the canonical loop-event sink for runtime observability.
    pub fn with_loop_event_tx(mut self, tx: mpsc::UnboundedSender<LoopEvent>) -> Self {
        self.loop_event_tx = Some(tx);
        self
    }

    /// Resolve a path relative to working directory (absolute paths pass through)
    pub fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let p = std::path::PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            self.working_dir.join(p)
        }
    }

    /// Resolve a path with sandbox enforcement for multi-tenant isolation.
    ///
    /// If sandbox_root is set, ensures the resolved path is within the sandbox.
    /// Returns an error if the path escapes the sandbox via symlinks or `..`.
    pub fn sandboxed_resolve(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let resolved = self.resolve_path(path);

        let Some(ref sandbox) = self.sandbox_root else {
            return Ok(resolved);
        };

        let canonical = resolved
            .canonicalize()
            .map_err(|e| format!("Invalid path '{}': {}", path, e))?;

        if !canonical.starts_with(sandbox) {
            return Err(format!(
                "Access denied: path '{}' is outside workspace",
                path
            ));
        }

        Ok(canonical)
    }

    /// Check if a path is within the sandbox (for validation without resolving).
    pub fn is_path_allowed(&self, path: &std::path::Path) -> bool {
        let Some(ref sandbox) = self.sandbox_root else {
            return true;
        };

        path.canonicalize()
            .map(|p| p.starts_with(sandbox))
            .unwrap_or(false)
    }

    /// Resolve a path that may not exist yet (for write operations) with sandbox enforcement.
    ///
    /// Unlike `sandboxed_resolve`, this handles paths where parent directories don't exist yet.
    /// It finds the nearest existing ancestor, canonicalizes it, validates it's within sandbox,
    /// then appends the remaining path components (which are verified to not contain traversal).
    pub fn sandboxed_resolve_new_path(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let resolved = self.resolve_path(path);

        let Some(ref sandbox) = self.sandbox_root else {
            return Ok(resolved);
        };

        for component in resolved.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err("Path traversal (..) not allowed".into());
            }
        }

        if resolved.exists() {
            let canonical = resolved
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e))?;
            if !canonical.starts_with(sandbox) {
                return Err("Access denied: path is outside workspace".into());
            }
            return Ok(canonical);
        }

        let mut check = resolved;
        let mut suffix: Vec<std::ffi::OsString> = Vec::new();

        while !check.exists() {
            if let Some(name) = check.file_name() {
                suffix.push(name.to_owned());
            }
            if !check.pop() {
                break;
            }
        }

        let canonical_base = if check.as_os_str().is_empty() || !check.exists() {
            sandbox.clone()
        } else {
            check
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e))?
        };

        if !canonical_base.starts_with(sandbox) {
            return Err("Access denied: path is outside workspace".into());
        }

        let mut final_path = canonical_base;
        for component in suffix.into_iter().rev() {
            final_path.push(component);
        }

        Ok(final_path)
    }
}
