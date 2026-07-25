//! Canonical construction boundary for production agent runs.
//!
//! Product surfaces should resolve their mutable inputs into a `RunSpec`
//! before starting the orchestrator. This keeps workspace identity, provider
//! request policy, and the provider cache key consistent across callers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::ai::client::{AiClient, CallOptions};
use crate::ai::types::ModelMessage;
use crate::storage::{MakoProfileSnapshot, SessionType, WorkMode};
use crate::tools::registry::PermissionMode;

use super::loop_events::{LoopEvent, LoopInput};
use super::orchestrator::{AgenticOrchestrator, OrchestratorConfig, OrchestratorServices};
use super::state::RunBudget;
use super::DelegatedProgressEvent;

/// Product surface that resolved a canonical run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunProvenance {
    Server,
    Tui,
    Acp,
    Mako,
    Delegated,
}

/// Execution kernel owned by a run surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKernel {
    StreamingOrchestrator,
    DelegatedToolLoop,
}

impl RunKernel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StreamingOrchestrator => "streaming_orchestrator",
            Self::DelegatedToolLoop => "delegated_tool_loop",
        }
    }
}

impl RunProvenance {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Tui => "tui",
            Self::Acp => "acp",
            Self::Mako => "mako",
            Self::Delegated => "delegated",
        }
    }

    pub const fn kernel(self) -> RunKernel {
        match self {
            Self::Delegated => RunKernel::DelegatedToolLoop,
            Self::Server | Self::Tui | Self::Acp | Self::Mako => RunKernel::StreamingOrchestrator,
        }
    }
}

/// Invalid or internally inconsistent run input.
#[derive(Debug, Error)]
pub enum RunSpecError {
    #[error("agent run session_id must not be empty")]
    EmptySessionId,
    #[error("agent run session_id must not contain surrounding whitespace")]
    SessionIdWhitespace,
    #[error("agent run session_id contains a control character")]
    InvalidSessionId,
    #[error("{kind} must be an absolute path: '{path}'")]
    RelativeWorkspacePath { kind: &'static str, path: PathBuf },
    #[error("{kind} is not accessible as a directory: '{path}': {source}")]
    InaccessibleWorkspacePath {
        kind: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{kind} is not a directory: '{path}'")]
    WorkspacePathNotDirectory { kind: &'static str, path: PathBuf },
    #[error("project_dir '{project_dir}' must be within working_dir '{working_dir}'")]
    ProjectOutsideWorkingDirectory {
        working_dir: PathBuf,
        project_dir: PathBuf,
    },
    #[error("run surface '{provenance}' cannot execute a '{session_type}' session")]
    SurfaceSessionTypeMismatch {
        provenance: &'static str,
        session_type: SessionType,
    },
    #[error(
        "provider cache session_id '{cache_session_id}' does not match run session_id '{run_session_id}'"
    )]
    CacheSessionMismatch {
        run_session_id: String,
        cache_session_id: String,
    },
    #[error("max_tokens must be greater than zero")]
    ZeroMaxTokens,
    #[error("temperature must be finite")]
    NonFiniteTemperature,
    #[error("tool names must not be empty")]
    EmptyToolName,
    #[error("duplicate tool name in request: '{0}'")]
    DuplicateToolName(String),
    #[error("run provenance '{actual}' cannot be consumed by '{expected}' driver")]
    DriverProvenanceMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("run surface '{provenance}' is owned by the '{kernel}' kernel, not RunSpec")]
    UnsupportedKernel {
        provenance: &'static str,
        kernel: &'static str,
    },
}

/// Fully validated orchestration configuration and provider request options.
pub struct RunSpec {
    provenance: RunProvenance,
    config: OrchestratorConfig,
    call_options: CallOptions,
}

impl RunSpec {
    pub const fn provenance(&self) -> RunProvenance {
        self.provenance
    }

    #[cfg(test)]
    pub(crate) fn config(&self) -> &OrchestratorConfig {
        &self.config
    }

    pub fn call_options(&self) -> &CallOptions {
        &self.call_options
    }

    /// Start the canonical orchestrator from already validated inputs.
    pub fn start(
        self,
        services: OrchestratorServices,
        conversation: Vec<ModelMessage>,
    ) -> (
        mpsc::UnboundedReceiver<LoopEvent>,
        mpsc::UnboundedSender<LoopInput>,
    ) {
        let (provenance, config, call_options) = self.into_parts();
        tracing::info!(
            surface = provenance.as_str(),
            session_id = %config.session_id,
            session_type = %config.session_type,
            working_dir = %config.working_dir.display(),
            project_dir = ?config.project_dir,
            "Starting resolved agent run"
        );
        AgenticOrchestrator::new(services, config).run(conversation, call_options)
    }

    /// Decompose a validated run for a higher-order driver such as Mako's
    /// tick engine. The returned settings remain canonical and aligned.
    pub(crate) fn into_parts(self) -> (RunProvenance, OrchestratorConfig, CallOptions) {
        (self.provenance, self.config, self.call_options)
    }

    /// Decompose only when the higher-order driver owns this run surface.
    pub(crate) fn into_parts_for(
        self,
        expected: RunProvenance,
    ) -> Result<(OrchestratorConfig, CallOptions), RunSpecError> {
        if self.provenance != expected {
            return Err(RunSpecError::DriverProvenanceMismatch {
                expected: expected.as_str(),
                actual: self.provenance.as_str(),
            });
        }
        Ok((self.config, self.call_options))
    }
}

/// Builder for the one canonical production run contract.
pub struct RunSpecBuilder {
    provenance: RunProvenance,
    config: OrchestratorConfig,
    call_options: CallOptions,
}

impl RunSpecBuilder {
    pub fn new(
        provenance: RunProvenance,
        session_id: impl Into<String>,
        working_dir: impl Into<PathBuf>,
        session_type: SessionType,
    ) -> Self {
        Self {
            provenance,
            config: OrchestratorConfig {
                session_id: session_id.into(),
                working_dir: working_dir.into(),
                session_type,
                ..Default::default()
            },
            call_options: CallOptions::default(),
        }
    }

    pub fn project_dir(mut self, project_dir: Option<PathBuf>) -> Self {
        self.config.project_dir = project_dir;
        self
    }

    pub fn mako_crew_slug(mut self, mako_crew_slug: Option<String>) -> Self {
        self.config.mako_crew_slug = mako_crew_slug;
        self
    }

    pub fn mako_profile(mut self, mako_profile: Option<Arc<MakoProfileSnapshot>>) -> Self {
        self.config.mako_profile = mako_profile;
        self
    }

    pub fn permission_mode(mut self, permission_mode: PermissionMode) -> Self {
        self.config.permission_mode = permission_mode;
        self
    }

    /// Constrain execution to an explicit per-turn capability set. This is
    /// intentionally separate from provider-advertised tools because an
    /// unrestricted `tool_search` may normally dispatch hidden specialists.
    pub fn execution_tool_allowlist(
        mut self,
        execution_tool_allowlist: Option<HashSet<String>>,
    ) -> Self {
        self.config.execution_tool_allowlist = execution_tool_allowlist;
        self
    }

    pub fn run_budget(mut self, run_budget: Option<RunBudget>) -> Self {
        self.config.run_budget = run_budget;
        self
    }

    pub fn stream_idle_timeout(mut self, stream_idle_timeout: Duration) -> Self {
        self.config.stream_idle_timeout = stream_idle_timeout;
        self
    }

    pub fn user_id(mut self, user_id: Option<String>) -> Self {
        self.config.user_id = user_id;
        self
    }

    pub fn initial_work_mode(mut self, initial_work_mode: WorkMode) -> Self {
        self.config.initial_work_mode = initial_work_mode;
        self
    }

    /// Declare that the caller's Code schemas came from the canonical mode
    /// policy and may be rebuilt from the registry as work mode changes.
    ///
    /// This is deliberately opt-in: an arbitrary caller-provided subset stays
    /// an immutable capability ceiling. An exact `execution_tool_allowlist`
    /// remains an upper bound even for policy-derived callers that opt in.
    pub fn mode_aware_code_tools(mut self, enabled: bool) -> Self {
        self.config.refresh_code_tools_on_mode_change =
            enabled && self.config.session_type == SessionType::Code;
        self
    }

    pub fn generate_title(mut self, generate_title: bool) -> Self {
        self.config.generate_title = generate_title;
        self
    }

    pub fn delegated_progress_tx(
        mut self,
        delegated_progress_tx: Option<mpsc::UnboundedSender<DelegatedProgressEvent>>,
    ) -> Self {
        self.config.delegated_progress_tx = delegated_progress_tx;
        self
    }

    pub fn call_options(mut self, call_options: CallOptions) -> Self {
        self.call_options = call_options;
        self
    }

    /// Validate surface/workspace identity and freeze request policy against
    /// the immutable model runtime held by this client.
    pub fn build(mut self, ai_client: &AiClient) -> Result<RunSpec, RunSpecError> {
        validate_session_id(&self.config.session_id)?;
        validate_surface(self.provenance, self.config.session_type)?;

        self.config.working_dir = canonical_directory("working_dir", &self.config.working_dir)?;
        if let Some(project_dir) = self.config.project_dir.take() {
            let project_dir = canonical_directory("project_dir", &project_dir)?;
            if !project_dir.starts_with(&self.config.working_dir) {
                return Err(RunSpecError::ProjectOutsideWorkingDirectory {
                    working_dir: self.config.working_dir,
                    project_dir,
                });
            }
            self.config.project_dir = Some(project_dir);
        }

        match self.call_options.session_id.as_deref() {
            Some(cache_session_id) if cache_session_id != self.config.session_id => {
                return Err(RunSpecError::CacheSessionMismatch {
                    run_session_id: self.config.session_id,
                    cache_session_id: cache_session_id.to_string(),
                });
            }
            Some(_) => {}
            None => self.call_options.session_id = Some(self.config.session_id.clone()),
        }
        // Opt-in never overrides a deliberately tool-free request.
        self.config.refresh_code_tools_on_mode_change &= self.call_options.tools.is_some();
        apply_execution_tool_allowlist(
            &mut self.call_options,
            self.config.execution_tool_allowlist.as_ref(),
        );
        validate_call_options(&mut self.call_options)?;
        self.call_options =
            ai_client.canonical_call_options(&ai_client.config().model, &self.call_options);
        validate_call_options(&mut self.call_options)?;

        Ok(RunSpec {
            provenance: self.provenance,
            config: self.config,
            call_options: self.call_options,
        })
    }
}

pub(crate) fn apply_execution_tool_allowlist(
    options: &mut CallOptions,
    execution_tool_allowlist: Option<&HashSet<String>>,
) {
    let Some(allowlist) = execution_tool_allowlist else {
        return;
    };

    if let Some(tools) = options.tools.as_mut() {
        tools.retain(|tool| allowlist.contains(&tool.name));
        if tools.is_empty() {
            options.tools = None;
        }
    }
    if !allowlist.contains("web_search") {
        options.web_search = None;
    }
    if !allowlist.contains("web_fetch") {
        options.web_fetch = None;
    }
    if options.tools.as_ref().is_none_or(|tools| tools.len() <= 1) {
        options.codex_parallel_tool_calls = false;
    }
}

fn validate_session_id(session_id: &str) -> Result<(), RunSpecError> {
    if session_id.is_empty() {
        return Err(RunSpecError::EmptySessionId);
    }
    if session_id.trim() != session_id {
        return Err(RunSpecError::SessionIdWhitespace);
    }
    if session_id.chars().any(char::is_control) {
        return Err(RunSpecError::InvalidSessionId);
    }
    Ok(())
}

fn validate_surface(
    provenance: RunProvenance,
    session_type: SessionType,
) -> Result<(), RunSpecError> {
    if provenance.kernel() != RunKernel::StreamingOrchestrator {
        return Err(RunSpecError::UnsupportedKernel {
            provenance: provenance.as_str(),
            kernel: provenance.kernel().as_str(),
        });
    }
    let valid = match provenance {
        RunProvenance::Server => session_type != SessionType::Mako,
        RunProvenance::Acp => session_type == SessionType::Code,
        RunProvenance::Mako => session_type == SessionType::Mako,
        RunProvenance::Tui => session_type == SessionType::Code,
        RunProvenance::Delegated => unreachable!("delegated kernel rejected above"),
    };
    if valid {
        Ok(())
    } else {
        Err(RunSpecError::SurfaceSessionTypeMismatch {
            provenance: provenance.as_str(),
            session_type,
        })
    }
}

fn canonical_directory(kind: &'static str, path: &Path) -> Result<PathBuf, RunSpecError> {
    if !path.is_absolute() {
        return Err(RunSpecError::RelativeWorkspacePath {
            kind,
            path: path.to_path_buf(),
        });
    }
    let canonical =
        path.canonicalize()
            .map_err(|source| RunSpecError::InaccessibleWorkspacePath {
                kind,
                path: path.to_path_buf(),
                source,
            })?;
    if !canonical.is_dir() {
        return Err(RunSpecError::WorkspacePathNotDirectory {
            kind,
            path: canonical,
        });
    }
    Ok(canonical)
}

fn validate_call_options(options: &mut CallOptions) -> Result<(), RunSpecError> {
    if options.max_tokens == Some(0) {
        return Err(RunSpecError::ZeroMaxTokens);
    }
    if options
        .temperature
        .is_some_and(|temperature| !temperature.is_finite())
    {
        return Err(RunSpecError::NonFiniteTemperature);
    }

    let Some(tools) = options.tools.as_mut() else {
        return Ok(());
    };
    if tools.is_empty() {
        options.tools = None;
        return Ok(());
    }

    let mut names = HashSet::with_capacity(tools.len());
    for tool in tools {
        if tool.name.trim().is_empty() {
            return Err(RunSpecError::EmptyToolName);
        }
        if !names.insert(tool.name.as_str()) {
            return Err(RunSpecError::DuplicateToolName(tool.name.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use super::{validate_call_options, validate_session_id, validate_surface};
    use super::{RunKernel, RunProvenance, RunSpecBuilder, RunSpecError};
    use crate::ai::client::{AiClient, AiClientConfig, CallOptions};
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::ProviderId;
    use crate::ai::types::{AiTool, WebSearchConfig};
    use crate::storage::SessionType;

    #[test]
    fn session_identity_rejects_whitespace_and_control_characters() {
        assert!(matches!(
            validate_session_id(" session-1"),
            Err(RunSpecError::SessionIdWhitespace)
        ));
        assert!(matches!(
            validate_session_id("session\n1"),
            Err(RunSpecError::InvalidSessionId)
        ));
    }

    #[test]
    fn surface_session_contract_prevents_mako_and_acp_drift() {
        assert!(validate_surface(RunProvenance::Acp, SessionType::Code).is_ok());
        assert!(matches!(
            validate_surface(RunProvenance::Acp, SessionType::Chat),
            Err(RunSpecError::SurfaceSessionTypeMismatch { .. })
        ));
        assert!(matches!(
            validate_surface(RunProvenance::Server, SessionType::Mako),
            Err(RunSpecError::SurfaceSessionTypeMismatch { .. })
        ));
        assert!(validate_surface(RunProvenance::Mako, SessionType::Mako).is_ok());
        assert!(validate_surface(RunProvenance::Tui, SessionType::Code).is_ok());
        assert!(matches!(
            validate_surface(RunProvenance::Tui, SessionType::Chat),
            Err(RunSpecError::SurfaceSessionTypeMismatch { .. })
        ));
        assert_eq!(
            RunProvenance::Delegated.kernel(),
            RunKernel::DelegatedToolLoop
        );
        assert!(matches!(
            validate_surface(RunProvenance::Delegated, SessionType::Code),
            Err(RunSpecError::UnsupportedKernel {
                provenance: "delegated",
                kernel: "delegated_tool_loop"
            })
        ));
    }

    #[test]
    fn call_options_normalize_empty_tools_and_reject_duplicates() {
        let mut empty = CallOptions {
            tools: Some(Vec::new()),
            ..Default::default()
        };
        validate_call_options(&mut empty).unwrap();
        assert!(empty.tools.is_none());

        let tool = AiTool {
            name: "read".into(),
            description: "Read".into(),
            input_schema: serde_json::json!({"type": "object"}),
            prompt: None,
        };
        let mut duplicate = CallOptions {
            tools: Some(vec![tool.clone(), tool]),
            ..Default::default()
        };
        assert!(matches!(
            validate_call_options(&mut duplicate),
            Err(RunSpecError::DuplicateToolName(name)) if name == "read"
        ));
    }

    #[test]
    fn canonical_directory_rejects_relative_paths() {
        assert!(matches!(
            super::canonical_directory("working_dir", Path::new("relative")),
            Err(RunSpecError::RelativeWorkspacePath { .. })
        ));
    }

    #[test]
    fn builder_aligns_provider_cache_identity_and_freezes_model_limits() {
        let workspace = tempfile::tempdir().unwrap();
        let project = workspace.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let client = AiClient::new(
            AiClientConfig {
                model: "gpt-5.5".to_string(),
                provider_id: ProviderId::OpenAI,
                api_format: ApiFormat::OpenAIResponses,
                ..Default::default()
            },
            String::new(),
        );

        let spec = RunSpecBuilder::new(
            RunProvenance::Server,
            "session-1",
            workspace.path(),
            SessionType::Code,
        )
        .project_dir(Some(project.clone()))
        .execution_tool_allowlist(Some(HashSet::from(["tool_search".to_string()])))
        .call_options(CallOptions {
            max_tokens: Some(usize::MAX),
            tools: Some(vec![
                AiTool {
                    name: "tool_search".into(),
                    description: "Deferred tool search".into(),
                    input_schema: serde_json::json!({"type": "object"}),
                    prompt: None,
                },
                AiTool {
                    name: "read".into(),
                    description: "Read".into(),
                    input_schema: serde_json::json!({"type": "object"}),
                    prompt: None,
                },
            ]),
            web_search: Some(WebSearchConfig::default()),
            codex_parallel_tool_calls: true,
            ..Default::default()
        })
        .build(&client)
        .unwrap();

        assert_eq!(
            spec.call_options().session_id.as_deref(),
            Some(spec.config().session_id.as_str())
        );
        assert_eq!(
            spec.config().working_dir,
            workspace.path().canonicalize().unwrap()
        );
        assert_eq!(
            spec.config().project_dir.as_deref(),
            Some(project.canonicalize().unwrap().as_path())
        );
        assert_eq!(
            spec.config().execution_tool_allowlist,
            Some(HashSet::from(["tool_search".to_string()]))
        );
        assert_eq!(
            spec.call_options()
                .tools
                .as_deref()
                .expect("exact scope should retain the wrapper")
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["tool_search"]
        );
        assert!(spec.call_options().web_search.is_none());
        assert!(!spec.call_options().codex_parallel_tool_calls);
        assert!(
            spec.call_options().max_tokens.unwrap()
                <= client.resolved_model().capabilities.max_output
        );
    }

    #[test]
    fn builder_rejects_cache_and_workspace_identity_drift() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let client = AiClient::new(Default::default(), String::new());

        let cache_error = RunSpecBuilder::new(
            RunProvenance::Server,
            "session-1",
            workspace.path(),
            SessionType::Code,
        )
        .call_options(CallOptions {
            session_id: Some("session-2".into()),
            ..Default::default()
        })
        .build(&client)
        .err()
        .unwrap();
        assert!(matches!(
            cache_error,
            RunSpecError::CacheSessionMismatch { .. }
        ));

        let workspace_error = RunSpecBuilder::new(
            RunProvenance::Server,
            "session-1",
            workspace.path(),
            SessionType::Code,
        )
        .project_dir(Some(outside.path().to_path_buf()))
        .build(&client)
        .err()
        .unwrap();
        assert!(matches!(
            workspace_error,
            RunSpecError::ProjectOutsideWorkingDirectory { .. }
        ));
    }

    #[test]
    fn higher_order_driver_cannot_consume_another_surfaces_spec() {
        let workspace = tempfile::tempdir().unwrap();
        let client = AiClient::new(Default::default(), String::new());
        let spec = RunSpecBuilder::new(
            RunProvenance::Server,
            "session-1",
            workspace.path(),
            SessionType::Code,
        )
        .build(&client)
        .unwrap();

        assert!(matches!(
            spec.into_parts_for(RunProvenance::Mako),
            Err(RunSpecError::DriverProvenanceMismatch {
                expected: "mako",
                actual: "server"
            })
        ));
    }

    #[test]
    fn builder_keeps_mode_refresh_explicit_and_never_infers_it_from_a_subset() {
        let workspace = tempfile::tempdir().unwrap();
        let client = AiClient::new(Default::default(), String::new());
        let tool = AiTool {
            name: "read".into(),
            description: "Read".into(),
            input_schema: serde_json::json!({"type": "object"}),
            prompt: None,
        };

        let tool_bearing = RunSpecBuilder::new(
            RunProvenance::Server,
            "tool-bearing",
            workspace.path(),
            SessionType::Code,
        )
        .call_options(CallOptions {
            tools: Some(vec![tool]),
            ..Default::default()
        })
        .build(&client)
        .unwrap();
        assert!(!tool_bearing.config().refresh_code_tools_on_mode_change);

        let mode_aware = RunSpecBuilder::new(
            RunProvenance::Server,
            "mode-aware",
            workspace.path(),
            SessionType::Code,
        )
        .mode_aware_code_tools(true)
        .call_options(CallOptions {
            tools: Some(vec![AiTool {
                name: "read".into(),
                description: "Read".into(),
                input_schema: serde_json::json!({"type": "object"}),
                prompt: None,
            }]),
            ..Default::default()
        })
        .build(&client)
        .unwrap();
        assert!(mode_aware.config().refresh_code_tools_on_mode_change);

        let tool_free = RunSpecBuilder::new(
            RunProvenance::Server,
            "tool-free",
            workspace.path(),
            SessionType::Code,
        )
        .mode_aware_code_tools(true)
        .call_options(CallOptions::default())
        .build(&client)
        .unwrap();
        assert!(!tool_free.config().refresh_code_tools_on_mode_change);
    }
}
