use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::agent::loop_events::LoopEvent;
use crate::agent::subagent::AgentProgress;
use crate::agent::ProviderCallTraceContext;
use crate::ai::client::AiClient;
use crate::ai::models::ModelKey;
use crate::ai::providers::ReasoningEffort;
use crate::ai::types::ModelMessage;
use crate::mcp::McpManager;
use crate::process::{CommandEnvironmentPolicy, ProcessRegistry};
use crate::skills::SkillsManager;
use crate::storage::WorkspaceMode;
use crate::tools::git_identity::GitIdentity;

use super::{DelegationPolicy, PermissionMode, ToolRegistry};

/// Filesystem access policy for local tool execution.
///
/// Workspace/project context is orientation for the model. This policy is the
/// explicit runtime filesystem boundary: local sessions default to unrestricted
/// access, while server/remote/multi-tenant paths can opt into a scoped root.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FilesystemAccess {
    /// Resolve paths relative to the working directory without imposing a root.
    #[default]
    Unrestricted,
    /// Require filesystem paths to stay under the configured root.
    Scoped { root: PathBuf },
}

impl FilesystemAccess {
    pub fn scoped(root: impl Into<PathBuf>) -> Self {
        Self::Scoped { root: root.into() }
    }

    pub fn scoped_root(&self) -> Option<&PathBuf> {
        match self {
            Self::Unrestricted => None,
            Self::Scoped { root } => Some(root),
        }
    }
}

/// Shared record of files successfully observed or authored by file tools.
#[derive(Debug, Default)]
pub struct FileObservationTracker {
    observed_files: StdRwLock<HashSet<PathBuf>>,
}

impl FileObservationTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, path: impl Into<PathBuf>) {
        let path = path.into();
        let mut observed = self.observed_files.write().unwrap_or_else(|poisoned| {
            tracing::warn!("File observation tracker write lock was poisoned; recovering");
            poisoned.into_inner()
        });
        observed.insert(path);
    }

    pub fn contains(&self, path: &Path) -> bool {
        let observed = self.observed_files.read().unwrap_or_else(|poisoned| {
            tracing::warn!("File observation tracker read lock was poisoned; recovering");
            poisoned.into_inner()
        });
        observed.contains(path)
    }

    pub fn remove(&self, path: &Path) {
        let mut observed = self.observed_files.write().unwrap_or_else(|poisoned| {
            tracing::warn!("File observation tracker write lock was poisoned; recovering");
            poisoned.into_inner()
        });
        observed.remove(path);
    }

    pub fn snapshot(&self) -> Vec<PathBuf> {
        let observed = self.observed_files.read().unwrap_or_else(|poisoned| {
            tracing::warn!("File observation tracker read lock was poisoned; recovering");
            poisoned.into_inner()
        });
        let mut paths = observed.iter().cloned().collect::<Vec<_>>();
        paths.sort();
        paths
    }
}

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
    /// Deprecated compatibility mirror for scoped filesystem access. New code
    /// should use `filesystem_access`; resolver helpers still consult this field
    /// while the legacy sandbox terminology is migrated.
    pub sandbox_root: Option<PathBuf>,
    /// Explicit runtime filesystem access policy.
    pub filesystem_access: FilesystemAccess,
    /// User ID for multi-tenant operation scoping (processes, etc.)
    pub user_id: Option<String>,
    /// Optional execution-scoped process owner. Delegated tasks use a distinct
    /// owner so temporary servers cannot consume the parent session's process
    /// budget. Tenant-scoped tools must continue to use `user_id`.
    pub process_owner_id: Option<String>,
    pub process_registry: Option<Arc<ProcessRegistry>>,
    pub skills_manager: Option<Arc<RwLock<SkillsManager>>>,
    pub mcp_manager: Option<Arc<McpManager>>,
    /// Optional per-call timeout override
    pub timeout: Option<Duration>,
    /// Runtime-owned command environment. Tool inputs cannot modify this map.
    pub command_environment: BTreeMap<String, String>,
    /// Whether shell commands inherit the host environment or execute inside
    /// the delegated sanitized environment boundary.
    pub command_environment_policy: CommandEnvironmentPolicy,
    /// Cancellation scoped to this exact dispatched tool call. Delegated
    /// runtimes use this instead of the process-global compatibility token so
    /// cancelling one parent session cannot affect another session's child.
    pub execution_cancellation: Option<CancellationToken>,
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
    /// Exact provider/auth/transport identity for the current run.
    pub current_model_key: Option<ModelKey>,
    /// Session-scoped AI client (used by tools that spawn sub-agents)
    pub ai_client: Option<Arc<AiClient>>,
    /// Git identity for commit attribution
    pub git_identity: Option<GitIdentity>,
    /// Parent execution permission mode inherited into delegated surfaces.
    pub permission_mode: PermissionMode,
    /// Whether the supervised parent explicitly approved this exact outer tool
    /// dispatch. Delegated tools use it to authorize only the capabilities
    /// declared by that approved call.
    pub supervised_approval_granted: bool,
    /// Optional delegated sub-agent turn budget inherited from parent config.
    pub subagent_max_turns: Option<usize>,
    /// Exact effective parent reasoning level inherited by delegated runs.
    /// `None` preserves legacy/unspecified behavior; `Some(None)` is explicit
    /// Off for models which permit it.
    pub delegated_reasoning_effort: Option<ReasoningEffort>,
    /// Optional delegated execution policy contract for downstream calls.
    pub delegation_policy: Option<DelegationPolicy>,
    /// Exact run-scoped tool capability inherited by delegated execution.
    /// `None` uses normal governed defaults; `Some(empty)` is intentionally
    /// tool-free and must remain distinguishable from `None`.
    pub execution_tool_allowlist: Option<HashSet<String>>,
    /// Tool registry for subagent delegation (explore/build use this to give subagents real tools).
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Parent conversation context for delegated agents that need upstream history.
    pub parent_conversation: Option<Arc<Vec<ModelMessage>>>,
    /// Canonical loop-event sink for hooks/tools that need to surface runtime events.
    pub loop_event_tx: Option<mpsc::UnboundedSender<LoopEvent>>,
    /// Provider-call accounting context inherited from the active agent turn.
    pub provider_call_trace: Option<ProviderCallTraceContext>,
    /// Shared file-observation tracker used to enforce observe-before-edit policy.
    pub file_observations: Arc<FileObservationTracker>,
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
            filesystem_access: FilesystemAccess::Unrestricted,
            user_id: None,
            process_owner_id: None,
            process_registry: None,
            skills_manager: None,
            mcp_manager: None,
            timeout: None,
            command_environment: BTreeMap::new(),
            command_environment_policy: CommandEnvironmentPolicy::default(),
            execution_cancellation: None,
            output_tx: None,
            tool_use_id: None,
            plan_mode: false,
            agent_progress_tx: None,
            current_model: None,
            current_model_key: None,
            ai_client: None,
            git_identity: None,
            permission_mode: PermissionMode::default(),
            supervised_approval_granted: false,
            subagent_max_turns: None,
            delegated_reasoning_effort: None,
            delegation_policy: None,
            execution_tool_allowlist: None,
            tool_registry: None,
            parent_conversation: None,
            loop_event_tx: None,
            provider_call_trace: None,
            file_observations: Arc::new(FileObservationTracker::default()),
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

    /// Set an explicit filesystem access policy.
    pub fn with_filesystem_access(mut self, access: FilesystemAccess) -> Self {
        self.sandbox_root = access.scoped_root().cloned();
        self.filesystem_access = access;
        self
    }

    /// Clear any scoped filesystem boundary.
    pub fn with_unrestricted_filesystem_access(mut self) -> Self {
        self.sandbox_root = None;
        self.filesystem_access = FilesystemAccess::Unrestricted;
        self
    }

    /// Set a scoped filesystem access root for host-level isolation.
    pub fn with_sandbox(mut self, sandbox_root: PathBuf) -> Self {
        self.sandbox_root = Some(sandbox_root.clone());
        self.filesystem_access = FilesystemAccess::scoped(sandbox_root);
        self
    }

    /// Effective filesystem access policy, including legacy `sandbox_root`
    /// struct-literal compatibility while call sites migrate.
    pub fn filesystem_access(&self) -> FilesystemAccess {
        if let Some(root) = self.sandbox_root.clone() {
            FilesystemAccess::Scoped { root }
        } else if let Some(root) = self.filesystem_access.scoped_root() {
            FilesystemAccess::Scoped { root: root.clone() }
        } else {
            FilesystemAccess::Unrestricted
        }
    }

    pub fn filesystem_access_root(&self) -> Option<PathBuf> {
        match self.filesystem_access() {
            FilesystemAccess::Unrestricted => None,
            FilesystemAccess::Scoped { root } => Some(root),
        }
    }

    /// Set user ID for multi-tenant operation scoping.
    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Set a process-only owner without changing tenant ownership.
    pub fn with_process_owner_id(mut self, process_owner_id: String) -> Self {
        self.process_owner_id = Some(process_owner_id);
        self
    }

    /// Owner used exclusively by the shared process registry. Normal parent
    /// sessions retain the historical user-scoped behavior.
    pub fn effective_process_owner_id(&self) -> Option<&str> {
        self.process_owner_id.as_deref().or(self.user_id.as_deref())
    }

    /// Add MCP manager to context
    pub fn with_mcp_manager(mut self, mcp_manager: Arc<McpManager>) -> Self {
        self.mcp_manager = Some(mcp_manager);
        self
    }

    /// Attach cancellation scoped to this exact tool dispatch.
    pub fn with_execution_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.execution_cancellation = Some(cancellation);
        self
    }

    /// Record approval for this exact dispatched tool call.
    pub fn with_supervised_approval(mut self, granted: bool) -> Self {
        self.supervised_approval_granted = granted;
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
        self.current_model = Some(client.resolved_model().wire_model_id.clone());
        self.current_model_key = Some(client.resolved_model().key.clone());
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

    pub fn with_delegated_reasoning_effort(
        mut self,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Self {
        self.delegated_reasoning_effort = reasoning_effort;
        self
    }

    /// Attach delegated execution policy metadata.
    pub fn with_delegation_policy(mut self, policy: DelegationPolicy) -> Self {
        self.delegation_policy = Some(policy);
        self
    }

    /// Attach the exact per-run execution capability. Delegated tools copy
    /// this ceiling into their child policy rather than rebuilding defaults.
    pub fn with_execution_tool_allowlist(
        mut self,
        execution_tool_allowlist: Option<&HashSet<String>>,
    ) -> Self {
        self.execution_tool_allowlist = execution_tool_allowlist.cloned();
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

    pub fn with_provider_call_trace(mut self, trace: ProviderCallTraceContext) -> Self {
        self.provider_call_trace = Some(trace);
        self
    }

    /// Attach a shared file-observation tracker.
    pub fn with_file_observation_tracker(mut self, tracker: Arc<FileObservationTracker>) -> Self {
        self.file_observations = tracker;
        self
    }

    /// Record that a canonical file path has been successfully observed or authored.
    pub fn record_file_observation(&self, path: impl Into<PathBuf>) {
        self.file_observations.record(path);
    }

    /// Record a successful file mutation using the canonical post-mutation path.
    pub fn record_file_mutation(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical = path.canonicalize().map_err(|e| {
            format!(
                "Failed to resolve mutated file path '{}': {}",
                path.display(),
                e
            )
        })?;
        self.record_file_observation(canonical.clone());
        Ok(canonical)
    }

    /// Invalidate a canonical file observation after deletion.
    pub fn forget_file_observation(&self, path: &Path) {
        self.file_observations.remove(path);
    }

    /// Check whether a canonical file path has been successfully observed.
    pub fn has_file_observation(&self, path: &Path) -> bool {
        self.file_observations.contains(path)
    }

    /// Require a resolved existing file path to have been observed first.
    ///
    /// Returns the canonical path used for observation matching.
    pub fn require_file_observation(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("Failed to resolve file path '{}': {}", path.display(), e))?;

        if self.has_file_observation(&canonical) {
            return Ok(canonical);
        }

        Err(format!(
            "Read required before modifying '{}'. Use the read tool on this file before edit, multiedit, or overwrite operations.",
            canonical.display()
        ))
    }

    /// Return a deterministic snapshot of observed file paths.
    pub fn observed_files_snapshot(&self) -> Vec<PathBuf> {
        self.file_observations.snapshot()
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

    /// Resolve a path under the configured filesystem access policy.
    ///
    /// Unrestricted local contexts return the path relative to `working_dir`.
    /// Scoped contexts canonicalize the target and require it to remain under
    /// the configured access root.
    pub fn sandboxed_resolve(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let resolved = self.resolve_path(path);

        let Some(access_root) = self.filesystem_access_root() else {
            return Ok(resolved);
        };

        let canonical_root = access_root.canonicalize().map_err(|e| {
            format!(
                "Invalid filesystem access root '{}': {}",
                access_root.display(),
                e
            )
        })?;
        let canonical = resolved
            .canonicalize()
            .map_err(|e| format!("Invalid path '{}': {}", path, e))?;

        if !canonical.starts_with(&canonical_root) {
            return Err(format!(
                "Access denied: path '{}' is outside configured filesystem access root",
                path
            ));
        }

        Ok(canonical)
    }

    /// Check if a path is allowed by the configured filesystem access policy.
    pub fn is_path_allowed(&self, path: &std::path::Path) -> bool {
        let Some(access_root) = self.filesystem_access_root() else {
            return true;
        };

        let Ok(canonical_root) = access_root.canonicalize() else {
            return false;
        };

        path.canonicalize()
            .map(|p| p.starts_with(canonical_root))
            .unwrap_or(false)
    }

    /// Resolve a path that may not exist yet (for write operations) under the
    /// configured filesystem access policy.
    ///
    /// Unlike `sandboxed_resolve`, this handles paths where parent directories don't exist yet.
    /// It finds the nearest existing ancestor, canonicalizes it, validates it
    /// against the configured access root, then appends the remaining path
    /// components (which are verified to not contain traversal).
    pub fn sandboxed_resolve_new_path(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let resolved = self.resolve_path(path);

        let Some(access_root) = self.filesystem_access_root() else {
            return Ok(resolved);
        };

        for component in resolved.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err("Path traversal (..) not allowed".into());
            }
        }

        let canonical_root = access_root.canonicalize().map_err(|e| {
            format!(
                "Invalid filesystem access root '{}': {}",
                access_root.display(),
                e
            )
        })?;

        if resolved.exists() {
            let canonical = resolved
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(
                    "Access denied: path is outside configured filesystem access root".into(),
                );
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
            canonical_root.clone()
        } else {
            check
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e))?
        };

        if !canonical_base.starts_with(&canonical_root) {
            return Err("Access denied: path is outside configured filesystem access root".into());
        }

        let mut final_path = canonical_base;
        for component in suffix.into_iter().rev() {
            final_path.push(component);
        }

        Ok(final_path)
    }
}
