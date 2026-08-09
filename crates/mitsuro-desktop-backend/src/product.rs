//! Product-domain contracts shared by the native desktop UI and transport adapters.
//!
//! These types intentionally avoid Codex app-server method names. Transport-specific
//! protocol objects stay inside the adapter implementations.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    AgentError, BackendKind, BackendSessionId, DesktopBackend, FsReadDirectoryParams,
    FsReadFileParams, FuzzyFileSearchParams, ListMcpServerStatusParams, LiveApprovalBridge,
    LiveTurnOutcome, ModelListParams, PluginListParams, Result, SessionDelegationProjection,
    SkillsListParams, ThreadDeleteParams, ThreadListParams, ThreadReadParams, ThreadSetNameParams,
    ThreadStartParams, TranscriptRole, TurnInterruptParams, TurnStartParams, TurnStreamEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: BackendSessionId,
    pub title: Option<String>,
    pub preview: Option<String>,
    pub working_dir: Option<String>,
    pub updated_at: Option<i64>,
    pub model_provider: Option<String>,
    pub ephemeral: bool,
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    Activity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub body: String,
    pub item_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConversation {
    pub session: SessionSummary,
    pub messages: Vec<ConversationMessage>,
    /// Canonical durable delegation state loaded alongside the transcript.
    /// Empty for backends that do not expose the Mitsuro coordinator contract.
    pub delegation: SessionDelegationProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductModel {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub is_default: bool,
    pub default_reasoning_effort: String,
    pub supported_reasoning_efforts: Vec<ProductReasoningEffort>,
    pub upgrade: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductReasoningEffort {
    pub effort: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateSession {
    pub working_dir: Option<String>,
    pub model: Option<String>,
    pub ephemeral: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductTurn {
    pub session_id: BackendSessionId,
    pub text: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductDirectoryEntry {
    pub name: String,
    pub is_directory: bool,
    pub is_file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductFile {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductFileMatch {
    pub root: String,
    pub path: String,
    pub file_name: String,
    pub is_directory: bool,
    pub score: u32,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSkill {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub path: String,
    pub scope: String,
    pub short_description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductMcpServer {
    pub name: String,
    pub title: Option<String>,
    pub status: String,
    pub tool_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductExtension {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub installed: bool,
    pub enabled: bool,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductProcess {
    pub id: String,
    pub command: String,
    pub description: Option<String>,
    pub pid: Option<u32>,
    pub status: String,
    pub elapsed_secs: u64,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
    pub working_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductHiveStatus {
    pub home_status: String,
    pub total_count: usize,
    pub running_count: usize,
    pub sleeping_count: usize,
    pub scheduled_count: usize,
    pub paused_count: usize,
    pub failed_count: usize,
    pub idle_count: usize,
    pub pending_approvals_count: usize,
    pub next_wake_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductHiveRun {
    pub session_id: String,
    pub title: String,
    pub updated_at: String,
    pub project_dir: Option<String>,
    pub target_branch: Option<String>,
    pub agent_state: String,
    pub pending_tasks: usize,
    pub in_progress_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub blocked_tasks: usize,
    pub diagnostic_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductHiveSnapshot {
    pub status: ProductHiveStatus,
    pub runs: Vec<ProductHiveRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSchedule {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub summary: String,
    pub objective: String,
    pub next_fire_at: Option<String>,
    pub status: String,
    pub timezone: String,
    pub project_dir: Option<String>,
    pub model: Option<String>,
    pub revision: u64,
}

#[async_trait]
pub trait ProductBackend: Send + Sync {
    fn backend_kind(&self) -> BackendKind;

    async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>>;

    async fn create_session(&self, request: CreateSession) -> Result<SessionSummary>;

    async fn read_session(&self, id: &BackendSessionId) -> Result<SessionConversation>;

    async fn rename_session(&self, id: &BackendSessionId, title: String) -> Result<()>;

    async fn delete_session(&self, id: &BackendSessionId) -> Result<()>;

    async fn list_product_models(&self, limit: usize) -> Result<Vec<ProductModel>>;

    async fn interrupt_session(&self, id: &BackendSessionId, turn_id: String) -> Result<()>;

    async fn browse_directory(&self, path: String) -> Result<Vec<ProductDirectoryEntry>>;

    async fn read_text_file(&self, path: String) -> Result<ProductFile>;

    async fn search_files(
        &self,
        query: String,
        roots: Vec<String>,
    ) -> Result<Vec<ProductFileMatch>>;

    async fn list_product_skills(&self) -> Result<Vec<ProductSkill>>;

    async fn list_product_mcp_servers(&self) -> Result<Vec<ProductMcpServer>>;

    async fn list_product_extensions(&self) -> Result<Vec<ProductExtension>>;

    async fn list_background_processes(&self) -> Result<Vec<ProductProcess>>;

    async fn hive_snapshot(&self) -> Result<ProductHiveSnapshot>;

    async fn list_schedules(&self) -> Result<Vec<ProductSchedule>>;
}

impl DesktopBackend {
    fn ensure_session_origin(&self, id: &BackendSessionId) -> Result<()> {
        if id.backend == self.kind() {
            return Ok(());
        }
        Err(AgentError::Other(format!(
            "session {} belongs to {}, but the active backend is {}",
            id.qualified(),
            id.backend.id(),
            self.kind().id()
        )))
    }

    pub fn run_product_turn_with_bridge_blocking(
        &self,
        request: ProductTurn,
        event_tx: std::sync::mpsc::Sender<TurnStreamEvent>,
        bridge: Arc<LiveApprovalBridge>,
        timeout: Duration,
    ) -> Result<LiveTurnOutcome> {
        self.ensure_session_origin(&request.session_id)?;
        self.run_turn_with_bridge_blocking(
            TurnStartParams::text_with_model(request.session_id.raw, request.text, request.model),
            event_tx,
            bridge,
            timeout,
        )
    }
}

#[async_trait]
impl ProductBackend for DesktopBackend {
    fn backend_kind(&self) -> BackendKind {
        self.kind()
    }

    async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>> {
        let response = self
            .thread_list(ThreadListParams {
                limit: Some(limit.min(u32::MAX as usize) as u32),
                use_state_db_only: Some(true),
                ..Default::default()
            })
            .await?;
        Ok(response
            .threads()
            .into_iter()
            .map(|thread| SessionSummary {
                id: BackendSessionId::new(self.kind(), thread.id),
                title: thread.name,
                preview: thread.preview,
                working_dir: thread.cwd,
                updated_at: thread.updated_at,
                model_provider: thread.model_provider,
                ephemeral: thread.ephemeral.unwrap_or(false),
                archived: thread.archived.unwrap_or(false),
            })
            .collect())
    }

    async fn create_session(&self, request: CreateSession) -> Result<SessionSummary> {
        let response = self
            .thread_start(ThreadStartParams {
                cwd: request.working_dir,
                model: request.model,
                ephemeral: Some(request.ephemeral),
                ..Default::default()
            })
            .await?;
        let thread = response.summary();
        Ok(SessionSummary {
            id: BackendSessionId::new(self.kind(), thread.id),
            title: thread.name,
            preview: thread.preview,
            working_dir: thread.cwd,
            updated_at: thread.updated_at,
            model_provider: thread.model_provider,
            ephemeral: thread.ephemeral.unwrap_or(false),
            archived: thread.archived.unwrap_or(false),
        })
    }

    async fn read_session(&self, id: &BackendSessionId) -> Result<SessionConversation> {
        self.ensure_session_origin(id)?;
        let response = self
            .thread_read(ThreadReadParams {
                thread_id: id.raw.clone(),
                include_turns: Some(true),
            })
            .await?;
        let thread = response.summary();
        let session = SessionSummary {
            id: id.clone(),
            title: thread.name,
            preview: thread.preview,
            working_dir: thread.cwd,
            updated_at: thread.updated_at,
            model_provider: thread.model_provider,
            ephemeral: thread.ephemeral.unwrap_or(false),
            archived: thread.archived.unwrap_or(false),
        };
        let messages = response
            .transcript_messages()
            .into_iter()
            .map(|message| ConversationMessage {
                role: match message.role {
                    TranscriptRole::User => MessageRole::User,
                    TranscriptRole::Assistant => MessageRole::Assistant,
                    _ => MessageRole::Activity,
                },
                body: message.body,
                item_id: message.item_id,
            })
            .collect();
        let delegation = match self {
            DesktopBackend::Mitsuro(backend) => {
                backend.session_delegation_projection(&id.raw).await?
            }
            _ => SessionDelegationProjection::default(),
        };
        Ok(SessionConversation {
            session,
            messages,
            delegation,
        })
    }

    async fn rename_session(&self, id: &BackendSessionId, title: String) -> Result<()> {
        self.ensure_session_origin(id)?;
        self.thread_name_set(ThreadSetNameParams::new(id.raw.clone(), title))
            .await?;
        Ok(())
    }

    async fn delete_session(&self, id: &BackendSessionId) -> Result<()> {
        self.ensure_session_origin(id)?;
        self.thread_delete(ThreadDeleteParams::new(id.raw.clone()))
            .await?;
        Ok(())
    }

    async fn list_product_models(&self, limit: usize) -> Result<Vec<ProductModel>> {
        let response = self
            .model_list(ModelListParams {
                limit: Some(limit.min(u32::MAX as usize) as u32),
                include_hidden: Some(false),
                ..Default::default()
            })
            .await?;
        Ok(response
            .data
            .into_iter()
            .map(|model| ProductModel {
                id: model.id,
                model: model.model,
                display_name: model.display_name,
                description: model.description,
                hidden: model.hidden,
                is_default: model.is_default,
                default_reasoning_effort: model.default_reasoning_effort,
                supported_reasoning_efforts: model
                    .supported_reasoning_efforts
                    .into_iter()
                    .map(|effort| ProductReasoningEffort {
                        effort: effort.reasoning_effort,
                        description: effort.description,
                    })
                    .collect(),
                upgrade: model.upgrade,
            })
            .collect())
    }

    async fn interrupt_session(&self, id: &BackendSessionId, turn_id: String) -> Result<()> {
        self.ensure_session_origin(id)?;
        self.turn_interrupt(TurnInterruptParams::new(id.raw.clone(), turn_id))
            .await?;
        Ok(())
    }

    async fn browse_directory(&self, path: String) -> Result<Vec<ProductDirectoryEntry>> {
        let response = self
            .fs_read_directory(FsReadDirectoryParams::new(path))
            .await?;
        Ok(response
            .entries
            .into_iter()
            .map(|entry| ProductDirectoryEntry {
                name: entry.file_name,
                is_directory: entry.is_directory,
                is_file: entry.is_file,
            })
            .collect())
    }

    async fn read_text_file(&self, path: String) -> Result<ProductFile> {
        let response = self
            .fs_read_file(FsReadFileParams::new(path.clone()))
            .await?;
        Ok(ProductFile {
            path,
            text: response.text_lossy(),
        })
    }

    async fn search_files(
        &self,
        query: String,
        roots: Vec<String>,
    ) -> Result<Vec<ProductFileMatch>> {
        let response = self
            .fuzzy_file_search(FuzzyFileSearchParams::new(query, roots))
            .await?;
        Ok(response
            .files
            .into_iter()
            .map(|entry| ProductFileMatch {
                root: entry.root,
                path: entry.path,
                file_name: entry.file_name,
                is_directory: matches!(
                    entry.match_type,
                    crate::FuzzyFileSearchMatchType::Directory
                ),
                score: entry.score,
                indices: entry.indices.unwrap_or_default(),
            })
            .collect())
    }

    async fn list_product_skills(&self) -> Result<Vec<ProductSkill>> {
        let response = self.skills_list(SkillsListParams::default()).await?;
        Ok(response
            .data
            .into_iter()
            .flat_map(|entry| entry.skills)
            .map(|skill| ProductSkill {
                name: skill.name,
                description: skill.description,
                enabled: skill.enabled,
                path: skill.path,
                scope: skill.scope,
                short_description: skill.short_description,
            })
            .collect())
    }

    async fn list_product_mcp_servers(&self) -> Result<Vec<ProductMcpServer>> {
        let response = self
            .mcp_server_status_list(ListMcpServerStatusParams::default())
            .await?;
        Ok(response
            .data
            .into_iter()
            .map(|server| ProductMcpServer {
                name: server.name.clone(),
                title: server
                    .server_info
                    .as_ref()
                    .and_then(|info| info.title.clone()),
                status: server.status_label(),
                tool_names: server.tools.into_keys().collect(),
            })
            .collect())
    }

    async fn list_product_extensions(&self) -> Result<Vec<ProductExtension>> {
        let response = self.plugin_list(PluginListParams::default()).await?;
        Ok(response
            .marketplaces
            .into_iter()
            .flat_map(|marketplace| marketplace.plugins)
            .map(|plugin| {
                let interface = plugin.interface.as_ref();
                let display_name = plugin.display_name().to_owned();
                ProductExtension {
                    id: plugin.id,
                    name: plugin.name.clone(),
                    display_name,
                    description: interface.and_then(|item| item.short_description.clone()),
                    category: interface.and_then(|item| item.category.clone()),
                    installed: plugin.installed,
                    enabled: plugin.enabled,
                    version: plugin.version.or(plugin.local_version),
                    capabilities: interface
                        .map(|item| item.capabilities.clone())
                        .unwrap_or_default(),
                    source: plugin.source.label(),
                }
            })
            .collect())
    }

    async fn list_background_processes(&self) -> Result<Vec<ProductProcess>> {
        let DesktopBackend::Mitsuro(backend) = self else {
            return Err(AgentError::NotImplemented(
                "Codex does not expose a background-process catalog".to_owned(),
            ));
        };
        let processes = backend
            .client()
            .list_processes()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(processes
            .into_iter()
            .map(|process| ProductProcess {
                id: process.id,
                command: process.command,
                description: process.description,
                pid: process.pid,
                status: process.status_code,
                elapsed_secs: process.elapsed_secs,
                error: process.error,
                exit_code: process.exit_code,
                working_dir: process.working_dir,
            })
            .collect())
    }

    async fn hive_snapshot(&self) -> Result<ProductHiveSnapshot> {
        let DesktopBackend::Mitsuro(backend) = self else {
            return Err(AgentError::NotImplemented(
                "Codex does not expose the Mitsuro Hive control plane".to_owned(),
            ));
        };
        let current = backend
            .client()
            .hive_current()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(ProductHiveSnapshot {
            status: ProductHiveStatus {
                home_status: current.status.home_status,
                total_count: current.status.total_count,
                running_count: current.status.running_count,
                sleeping_count: current.status.sleeping_count,
                scheduled_count: current.status.scheduled_count,
                paused_count: current.status.paused_count,
                failed_count: current.status.failed_count,
                idle_count: current.status.idle_count,
                pending_approvals_count: current.status.pending_approvals_count,
                next_wake_at: current.status.next_wake_at,
            },
            runs: current
                .runs
                .into_iter()
                .map(|run| ProductHiveRun {
                    session_id: run.session_id,
                    title: run.title,
                    updated_at: run.updated_at,
                    project_dir: run.project_dir,
                    target_branch: run.target_branch,
                    agent_state: run.agent_state,
                    pending_tasks: run.pending_tasks,
                    in_progress_tasks: run.in_progress_tasks,
                    completed_tasks: run.completed_tasks,
                    failed_tasks: run.failed_tasks,
                    blocked_tasks: run.blocked_tasks,
                    diagnostic_summary: run.diagnostic.map(|diagnostic| diagnostic.summary),
                })
                .collect(),
        })
    }

    async fn list_schedules(&self) -> Result<Vec<ProductSchedule>> {
        let DesktopBackend::Mitsuro(backend) = self else {
            return Err(AgentError::NotImplemented(
                "Codex does not expose Mitsuro Hive schedules".to_owned(),
            ));
        };
        let schedules = backend
            .client()
            .list_hive_schedules()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(schedules
            .into_iter()
            .map(|schedule| ProductSchedule {
                id: schedule.id,
                session_id: schedule.controller_session_id,
                title: schedule.title,
                summary: schedule.summary,
                objective: schedule.objective,
                next_fire_at: schedule.next_fire_at,
                status: schedule.status,
                timezone: schedule.timezone,
                project_dir: schedule.project_dir,
                model: schedule.model,
                revision: schedule.revision,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_turn_keeps_backend_qualified_identity() {
        let request = ProductTurn {
            session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
            text: "hello".to_owned(),
            model: None,
        };
        assert_eq!(request.session_id.qualified(), "mitsuro-http:session-7");
    }

    #[test]
    fn product_turn_rejects_a_session_from_another_backend_before_io() {
        let backend = DesktopBackend::codex_stdio();
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let error = backend
            .run_product_turn_with_bridge_blocking(
                ProductTurn {
                    session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                    text: "hello".to_owned(),
                    model: None,
                },
                event_tx,
                Arc::new(LiveApprovalBridge::new()),
                Duration::from_secs(1),
            )
            .expect_err("mismatched origin must fail");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }
}
