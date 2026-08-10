//! Product-domain contracts shared by the native desktop UI and transport adapters.
//!
//! These types intentionally avoid Codex app-server method names. Transport-specific
//! protocol objects stay inside the adapter implementations.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    ActivityFields, AgentError, BackendKind, BackendSessionId, CollaborationMode,
    CollaborationModeSettings, CommandExecutionFields, DesktopBackend, FileChangeFields,
    FsReadDirectoryParams, FsReadFileParams, FuzzyFileSearchParams, ListMcpServerStatusParams,
    LiveApprovalBridge, LiveReviewOutcome, LiveTurnOutcome, ModelListParams, PluginListParams,
    Result, ReviewDelivery, ReviewStartParams, ReviewTarget, SessionDelegationProjection,
    SkillsListParams, ThreadCompactStartParams, ThreadDeleteParams, ThreadListParams,
    ThreadReadParams, ThreadSetNameParams, ThreadStartParams, TranscriptAudioSource,
    TranscriptImageSource, TranscriptMessage, TranscriptReferenceKind, TranscriptRole,
    TurnInterruptParams, TurnStartParams, TurnSteerParams, TurnStreamEvent,
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
    Reasoning,
    Plan,
    CommandExecution,
    FileChange,
    Activity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub body: String,
    pub item_id: Option<String>,
    pub command: Option<CommandExecutionFields>,
    pub file_change: Option<FileChangeFields>,
    pub activity: Option<ActivityFields>,
    pub images: Vec<ConversationImage>,
    pub audio: Vec<ConversationAudio>,
    pub references: Vec<ConversationReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationImage {
    LocalPath(String),
    Url(String),
    Embedded { media_type: String, data: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationAudio {
    LocalPath(String),
    Url(String),
    Embedded { media_type: String, data: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationReference {
    pub kind: ConversationReferenceKind,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationReferenceKind {
    Skill,
    Mention,
}

fn conversation_message_from_transcript(message: TranscriptMessage) -> ConversationMessage {
    ConversationMessage {
        role: match message.role {
            TranscriptRole::User => MessageRole::User,
            TranscriptRole::Assistant => MessageRole::Assistant,
            TranscriptRole::Reasoning => MessageRole::Reasoning,
            TranscriptRole::Plan => MessageRole::Plan,
            TranscriptRole::CommandExecution => MessageRole::CommandExecution,
            TranscriptRole::FileChange => MessageRole::FileChange,
            TranscriptRole::System => MessageRole::Activity,
        },
        body: message.body,
        item_id: message.item_id,
        command: message.command,
        file_change: message.file_change,
        activity: message.activity,
        images: message
            .images
            .into_iter()
            .map(|image| match image.source {
                TranscriptImageSource::LocalPath(path) => ConversationImage::LocalPath(path),
                TranscriptImageSource::Url(url) => ConversationImage::Url(url),
                TranscriptImageSource::Embedded { media_type, data } => {
                    ConversationImage::Embedded { media_type, data }
                }
            })
            .collect(),
        audio: message
            .audio
            .into_iter()
            .map(|audio| match audio.source {
                TranscriptAudioSource::LocalPath(path) => ConversationAudio::LocalPath(path),
                TranscriptAudioSource::Url(url) => ConversationAudio::Url(url),
                TranscriptAudioSource::Embedded { media_type, data } => {
                    ConversationAudio::Embedded { media_type, data }
                }
            })
            .collect(),
        references: message
            .references
            .into_iter()
            .map(|reference| ConversationReference {
                kind: match reference.kind {
                    TranscriptReferenceKind::Skill => ConversationReferenceKind::Skill,
                    TranscriptReferenceKind::Mention => ConversationReferenceKind::Mention,
                },
                name: reference.name,
                path: reference.path,
            })
            .collect(),
    }
}

pub(crate) fn conversation_messages_from_turn_values(
    turns: Vec<serde_json::Value>,
) -> Vec<ConversationMessage> {
    let thread = serde_json::json!({ "turns": turns });
    crate::extract_transcript_from_thread(&thread)
        .into_iter()
        .map(conversation_message_from_transcript)
        .collect()
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
    pub speed_options: Vec<ProductSpeedOption>,
    pub default_speed_mode: ProductSpeedMode,
    pub input_modalities: Vec<String>,
    pub upgrade: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductReasoningEffort {
    pub effort: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSpeedOption {
    pub mode: ProductSpeedMode,
    pub label: String,
    pub description: String,
}

/// Backend-specific response-speed controls shown in one product slot.
/// Codex service tiers and Mitsuro fast mode intentionally remain distinct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductSpeedMode {
    CodexStandard,
    CodexServiceTier(String),
    MitsuroStandard,
    MitsuroFast,
}

/// Backend-specific collaboration/workflow choices shown in one product slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductWorkMode {
    Codex {
        mode: crate::environment::ModeKind,
        model: String,
        reasoning_effort: Option<String>,
    },
    MitsuroBuild,
    MitsuroPlan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateSession {
    pub working_dir: Option<String>,
    pub model: Option<String>,
    pub ephemeral: bool,
    pub access_mode: Option<ProductAccessMode>,
    pub speed_mode: Option<ProductSpeedMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductTurn {
    pub session_id: BackendSessionId,
    pub text: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub working_dir: Option<String>,
    pub access_mode: Option<ProductAccessMode>,
    pub speed_mode: Option<ProductSpeedMode>,
    pub work_mode: Option<ProductWorkMode>,
    pub attachments: Vec<ProductAttachment>,
}

/// Backend-specific access choices rendered in one transport-neutral product slot.
/// Variants are intentionally not collapsed because Codex sandbox presets and Mitsuro
/// supervision modes have different semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductAccessMode {
    CodexReadOnly,
    CodexAuto,
    CodexFullAccess,
    MitsuroSupervised,
    MitsuroAutonomous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductAttachment {
    LocalImage { path: String },
    LocalAudio { path: String },
    Skill { name: String, path: String },
    Mention { name: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSteer {
    pub session_id: BackendSessionId,
    pub expected_turn_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductReviewTarget {
    UncommittedChanges,
    BaseBranch(String),
    Commit { sha: String, title: Option<String> },
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductReview {
    pub session_id: BackendSessionId,
    pub target: ProductReviewTarget,
    pub detached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductReviewStart {
    pub review_session_id: BackendSessionId,
    pub turn_id: String,
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
    pub install_policy: crate::PluginInstallPolicy,
    pub auth_policy: crate::PluginAuthPolicy,
    pub availability: crate::PluginAvailability,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub source: String,
    pub marketplace_path: Option<String>,
    pub remote_marketplace_name: Option<String>,
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

    async fn steer_session(&self, request: ProductSteer) -> Result<String>;

    async fn compact_session(&self, id: &BackendSessionId) -> Result<()>;

    async fn start_review(&self, request: ProductReview) -> Result<ProductReviewStart>;

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

    async fn terminate_background_process(&self, process_id: String) -> Result<()>;

    async fn hive_snapshot(&self) -> Result<ProductHiveSnapshot>;

    async fn list_schedules(&self) -> Result<Vec<ProductSchedule>>;
}

impl DesktopBackend {
    pub(crate) fn ensure_session_origin(&self, id: &BackendSessionId) -> Result<()> {
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
        validate_access_mode(self.kind(), request.access_mode)?;
        validate_speed_mode(self.kind(), request.speed_mode.as_ref())?;
        validate_work_mode(self.kind(), request.work_mode.as_ref())?;
        if request
            .attachments
            .iter()
            .any(|attachment| matches!(attachment, ProductAttachment::LocalAudio { .. }))
            && !self.capabilities().audio_attachments
        {
            return Err(AgentError::NotImplemented(format!(
                "{} does not accept audio attachments",
                self.kind().id()
            )));
        }
        if request
            .attachments
            .iter()
            .any(|attachment| matches!(attachment, ProductAttachment::Skill { .. }))
            && !self.capabilities().skill_inputs
        {
            return Err(AgentError::NotImplemented(format!(
                "{} does not accept Codex skill inputs",
                self.kind().id()
            )));
        }
        if request
            .attachments
            .iter()
            .any(|attachment| matches!(attachment, ProductAttachment::Mention { .. }))
            && !self.capabilities().mention_inputs
        {
            return Err(AgentError::NotImplemented(format!(
                "{} does not accept Codex mention inputs",
                self.kind().id()
            )));
        }
        let params = product_turn_params(request, self.kind());
        self.run_turn_with_bridge_blocking(params, event_tx, bridge, timeout)
    }

    pub fn run_product_review_with_bridge_blocking(
        &self,
        request: ProductReview,
        event_tx: std::sync::mpsc::Sender<TurnStreamEvent>,
        bridge: Arc<LiveApprovalBridge>,
        timeout: Duration,
    ) -> Result<LiveReviewOutcome> {
        self.ensure_session_origin(&request.session_id)?;
        if !self.capabilities().review {
            return Err(AgentError::NotImplemented(format!(
                "{} does not expose code review turns",
                self.kind().id()
            )));
        }
        let DesktopBackend::Codex(backend) = self else {
            return Err(AgentError::NotImplemented(
                "review streaming is unavailable for this backend".to_owned(),
            ));
        };
        let params = review_start_params(request);
        let runtime = Arc::clone(backend);
        let runner = Arc::clone(backend);
        runtime.block_on(async move {
            crate::run_live_review_with_bridge(
                runner.as_ref(),
                params,
                |event| {
                    let _ = event_tx.send(event);
                },
                bridge,
                timeout,
            )
            .await
        })
    }
}

fn product_turn_params(request: ProductTurn, backend: BackendKind) -> TurnStartParams {
    let mut params =
        TurnStartParams::text_with_model(request.session_id.raw, request.text, request.model);
    params.effort = request.reasoning_effort;
    params.cwd = request.working_dir;
    apply_access_to_turn_params(&mut params, backend, request.access_mode);
    apply_speed_to_turn_params(&mut params, backend, request.speed_mode);
    apply_work_to_turn_params(&mut params, backend, request.work_mode);
    for attachment in request.attachments {
        match attachment {
            ProductAttachment::LocalImage { path } => params.push_local_image(path),
            ProductAttachment::LocalAudio { path } => params.push_local_audio(path),
            ProductAttachment::Skill { name, path } => params.push_skill(name, path),
            ProductAttachment::Mention { name, path } => params.push_mention(name, path),
        }
    }
    params
}

fn validate_speed_mode(backend: BackendKind, mode: Option<&ProductSpeedMode>) -> Result<()> {
    let Some(mode) = mode else {
        return Ok(());
    };
    let valid = matches!(
        (backend, mode),
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            ProductSpeedMode::CodexStandard | ProductSpeedMode::CodexServiceTier(_)
        ) | (
            BackendKind::MitsuroHttp,
            ProductSpeedMode::MitsuroStandard | ProductSpeedMode::MitsuroFast
        ) | (
            BackendKind::Fixture,
            ProductSpeedMode::CodexStandard | ProductSpeedMode::CodexServiceTier(_)
        )
    );
    if valid {
        Ok(())
    } else {
        Err(AgentError::NotImplemented(format!(
            "{} does not accept the selected speed mode",
            backend.id()
        )))
    }
}

fn validate_work_mode(backend: BackendKind, mode: Option<&ProductWorkMode>) -> Result<()> {
    let Some(mode) = mode else {
        return Ok(());
    };
    let valid = matches!(
        (backend, mode),
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture,
            ProductWorkMode::Codex { .. }
        ) | (
            BackendKind::MitsuroHttp,
            ProductWorkMode::MitsuroBuild | ProductWorkMode::MitsuroPlan
        )
    );
    if valid {
        Ok(())
    } else {
        Err(AgentError::NotImplemented(format!(
            "{} does not accept the selected work mode",
            backend.id()
        )))
    }
}

fn apply_work_to_turn_params(
    params: &mut TurnStartParams,
    backend: BackendKind,
    mode: Option<ProductWorkMode>,
) {
    match (backend, mode) {
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture,
            Some(ProductWorkMode::Codex {
                mode,
                model,
                reasoning_effort,
            }),
        ) => {
            // Codex collaboration settings own model and effort whenever present.
            params.model = None;
            params.effort = None;
            params.collaboration_mode = Some(CollaborationMode {
                mode,
                settings: CollaborationModeSettings {
                    model,
                    reasoning_effort,
                    developer_instructions: None,
                },
            });
        }
        (BackendKind::MitsuroHttp, Some(ProductWorkMode::MitsuroBuild)) => {
            params.mitsuro_work_mode = Some("build".to_owned());
        }
        (BackendKind::MitsuroHttp, Some(ProductWorkMode::MitsuroPlan)) => {
            params.mitsuro_work_mode = Some("plan".to_owned());
        }
        _ => {}
    }
}

fn apply_speed_to_turn_params(
    params: &mut TurnStartParams,
    backend: BackendKind,
    mode: Option<ProductSpeedMode>,
) {
    match (backend, mode) {
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture,
            Some(ProductSpeedMode::CodexServiceTier(tier)),
        ) => params.service_tier = Some(tier),
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture,
            Some(ProductSpeedMode::CodexStandard),
        ) => params.service_tier = None,
        (BackendKind::MitsuroHttp, Some(ProductSpeedMode::MitsuroFast)) => {
            params.mitsuro_fast_mode = Some(true);
        }
        (BackendKind::MitsuroHttp, Some(ProductSpeedMode::MitsuroStandard)) => {
            params.mitsuro_fast_mode = Some(false);
        }
        _ => {}
    }
}

fn apply_speed_to_thread_params(
    params: &mut ThreadStartParams,
    backend: BackendKind,
    mode: Option<ProductSpeedMode>,
) {
    match (backend, mode) {
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture,
            Some(ProductSpeedMode::CodexServiceTier(tier)),
        ) => params.service_tier = Some(tier),
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture,
            Some(ProductSpeedMode::CodexStandard),
        ) => params.service_tier = None,
        _ => {}
    }
}

fn validate_access_mode(backend: BackendKind, mode: Option<ProductAccessMode>) -> Result<()> {
    let Some(mode) = mode else {
        return Ok(());
    };
    let valid = matches!(
        (backend, mode),
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            ProductAccessMode::CodexReadOnly
                | ProductAccessMode::CodexAuto
                | ProductAccessMode::CodexFullAccess
        ) | (
            BackendKind::MitsuroHttp,
            ProductAccessMode::MitsuroSupervised | ProductAccessMode::MitsuroAutonomous
        )
    );
    if valid {
        Ok(())
    } else {
        Err(AgentError::NotImplemented(format!(
            "{} does not accept the selected access mode",
            backend.id()
        )))
    }
}

fn absolute_workspace_roots(cwd: Option<&str>) -> Option<Vec<String>> {
    cwd.filter(|path| std::path::Path::new(path).is_absolute())
        .map(|path| vec![path.to_owned()])
}

fn apply_access_to_turn_params(
    params: &mut TurnStartParams,
    backend: BackendKind,
    mode: Option<ProductAccessMode>,
) {
    let roots = absolute_workspace_roots(params.cwd.as_deref());
    match (backend, mode) {
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            Some(ProductAccessMode::CodexReadOnly),
        ) => {
            params.permissions = Some(crate::READ_ONLY_PROFILE_ID.to_owned());
            params.runtime_workspace_roots = roots;
        }
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            Some(ProductAccessMode::CodexAuto),
        ) => {
            params.permissions = Some(crate::WORKSPACE_PROFILE_ID.to_owned());
            params.runtime_workspace_roots = roots;
        }
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            Some(ProductAccessMode::CodexFullAccess),
        ) => {
            params.permissions = Some(crate::FULL_ACCESS_PROFILE_ID.to_owned());
            params.runtime_workspace_roots = roots;
        }
        (BackendKind::MitsuroHttp, Some(ProductAccessMode::MitsuroSupervised)) => {
            params.mitsuro_permission_mode = Some("supervised".to_owned());
        }
        (BackendKind::MitsuroHttp, Some(ProductAccessMode::MitsuroAutonomous)) => {
            params.mitsuro_permission_mode = Some("autonomous".to_owned());
        }
        _ => {}
    }
}

fn apply_access_to_thread_params(
    params: &mut ThreadStartParams,
    backend: BackendKind,
    mode: Option<ProductAccessMode>,
) {
    let roots = absolute_workspace_roots(params.cwd.as_deref());
    match (backend, mode) {
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            Some(ProductAccessMode::CodexReadOnly),
        ) => {
            params.permissions = Some(crate::READ_ONLY_PROFILE_ID.to_owned());
            params.runtime_workspace_roots = roots;
        }
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            Some(ProductAccessMode::CodexAuto),
        ) => {
            params.permissions = Some(crate::WORKSPACE_PROFILE_ID.to_owned());
            params.runtime_workspace_roots = roots;
        }
        (
            BackendKind::CodexStdio | BackendKind::CodexWebSocket,
            Some(ProductAccessMode::CodexFullAccess),
        ) => {
            params.permissions = Some(crate::FULL_ACCESS_PROFILE_ID.to_owned());
            params.runtime_workspace_roots = roots;
        }
        (BackendKind::MitsuroHttp, Some(ProductAccessMode::MitsuroSupervised)) => {
            params.mitsuro_permission_mode = Some("supervised".to_owned());
        }
        (BackendKind::MitsuroHttp, Some(ProductAccessMode::MitsuroAutonomous)) => {
            params.mitsuro_permission_mode = Some("autonomous".to_owned());
        }
        _ => {}
    }
}

fn review_start_params(request: ProductReview) -> ReviewStartParams {
    let target = match request.target {
        ProductReviewTarget::UncommittedChanges => ReviewTarget::UncommittedChanges,
        ProductReviewTarget::BaseBranch(branch) => ReviewTarget::BaseBranch { branch },
        ProductReviewTarget::Commit { sha, title } => ReviewTarget::Commit { sha, title },
        ProductReviewTarget::Custom(instructions) => ReviewTarget::Custom { instructions },
    };
    ReviewStartParams {
        thread_id: request.session_id.raw,
        target,
        delivery: Some(if request.detached {
            ReviewDelivery::Detached
        } else {
            ReviewDelivery::Inline
        }),
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
        validate_access_mode(self.kind(), request.access_mode)?;
        validate_speed_mode(self.kind(), request.speed_mode.as_ref())?;
        let mut params = ThreadStartParams {
            cwd: request.working_dir,
            model: request.model,
            ephemeral: Some(request.ephemeral),
            ..Default::default()
        };
        apply_access_to_thread_params(&mut params, self.kind(), request.access_mode);
        apply_speed_to_thread_params(&mut params, self.kind(), request.speed_mode);
        let response = self.thread_start(params).await?;
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
            .map(conversation_message_from_transcript)
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
        let backend = self.kind();
        Ok(response
            .data
            .into_iter()
            .map(|model| {
                let speed_options = model
                    .service_tiers
                    .into_iter()
                    .map(|tier| ProductSpeedOption {
                        mode: match backend {
                            BackendKind::CodexStdio
                            | BackendKind::CodexWebSocket
                            | BackendKind::Fixture => ProductSpeedMode::CodexServiceTier(tier.id),
                            BackendKind::MitsuroHttp => ProductSpeedMode::MitsuroFast,
                        },
                        label: tier.name,
                        description: tier.description,
                    })
                    .collect();
                let default_speed_mode = match backend {
                    BackendKind::CodexStdio
                    | BackendKind::CodexWebSocket
                    | BackendKind::Fixture => model
                        .default_service_tier
                        .map(ProductSpeedMode::CodexServiceTier)
                        .unwrap_or(ProductSpeedMode::CodexStandard),
                    BackendKind::MitsuroHttp => ProductSpeedMode::MitsuroStandard,
                };
                ProductModel {
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
                    speed_options,
                    default_speed_mode,
                    input_modalities: model.input_modalities,
                    upgrade: model.upgrade,
                }
            })
            .collect())
    }

    async fn interrupt_session(&self, id: &BackendSessionId, turn_id: String) -> Result<()> {
        self.ensure_session_origin(id)?;
        self.turn_interrupt(TurnInterruptParams::new(id.raw.clone(), turn_id))
            .await?;
        Ok(())
    }

    async fn steer_session(&self, request: ProductSteer) -> Result<String> {
        self.ensure_session_origin(&request.session_id)?;
        let response = self
            .turn_steer(TurnSteerParams::text(
                request.session_id.raw,
                request.expected_turn_id,
                request.text,
            ))
            .await?;
        Ok(response.turn_id)
    }

    async fn compact_session(&self, id: &BackendSessionId) -> Result<()> {
        self.ensure_session_origin(id)?;
        if !self.capabilities().manual_compaction {
            return Err(AgentError::NotImplemented(format!(
                "{} does not expose manual thread compaction",
                self.kind().id()
            )));
        }
        self.thread_compact_start(ThreadCompactStartParams::new(id.raw.clone()))
            .await?;
        Ok(())
    }

    async fn start_review(&self, request: ProductReview) -> Result<ProductReviewStart> {
        self.ensure_session_origin(&request.session_id)?;
        if !self.capabilities().review {
            return Err(AgentError::NotImplemented(format!(
                "{} does not expose code review turns",
                self.kind().id()
            )));
        }
        let response = self.review_start(review_start_params(request)).await?;
        let turn_id = response
            .turn_id()
            .ok_or_else(|| {
                AgentError::Protocol("review/start response is missing turn.id".to_owned())
            })?
            .to_owned();
        Ok(ProductReviewStart {
            review_session_id: BackendSessionId::new(self.kind(), response.review_thread_id),
            turn_id,
        })
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
            .flat_map(|marketplace| {
                let marketplace_path = marketplace.path;
                let remote_marketplace_name =
                    marketplace_path.is_none().then_some(marketplace.name);
                marketplace.plugins.into_iter().map(move |plugin| {
                    (
                        plugin,
                        marketplace_path.clone(),
                        remote_marketplace_name.clone(),
                    )
                })
            })
            .map(|(plugin, marketplace_path, remote_marketplace_name)| {
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
                    install_policy: plugin.install_policy,
                    auth_policy: plugin.auth_policy,
                    availability: plugin.availability,
                    version: plugin.version.or(plugin.local_version),
                    capabilities: interface
                        .map(|item| item.capabilities.clone())
                        .unwrap_or_default(),
                    source: plugin.source.label(),
                    marketplace_path,
                    remote_marketplace_name,
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

    async fn terminate_background_process(&self, process_id: String) -> Result<()> {
        let DesktopBackend::Mitsuro(backend) = self else {
            return Err(AgentError::NotImplemented(
                "Codex thread terminals use the thread/backgroundTerminals contract".to_owned(),
            ));
        };
        backend
            .client()
            .kill_process(&process_id)
            .await
            .map_err(|error| AgentError::Other(error.to_string()))
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
            reasoning_effort: None,
            working_dir: None,
            access_mode: None,
            speed_mode: None,
            work_mode: None,
            attachments: Vec::new(),
        };
        assert_eq!(request.session_id.qualified(), "mitsuro-http:session-7");
    }

    #[test]
    fn product_turn_preserves_local_images_for_codex_wire_input() {
        let params = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                text: "inspect".to_owned(),
                model: Some("gpt-5".to_owned()),
                reasoning_effort: Some("high".to_owned()),
                working_dir: None,
                access_mode: None,
                speed_mode: None,
                work_mode: None,
                attachments: vec![ProductAttachment::LocalImage {
                    path: "/tmp/capture.png".to_owned(),
                }],
            },
            BackendKind::CodexStdio,
        );
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["threadId"], "thread-7");
        assert_eq!(value["effort"], "high");
        assert_eq!(value["input"][0]["text"], "inspect");
        assert_eq!(value["input"][1]["type"], "localImage");
        assert_eq!(value["input"][1]["path"], "/tmp/capture.png");
    }

    #[test]
    fn product_turn_preserves_local_audio_for_codex_wire_input() {
        let params = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                text: "transcribe".to_owned(),
                model: Some("gpt-5".to_owned()),
                reasoning_effort: None,
                working_dir: None,
                access_mode: None,
                speed_mode: None,
                work_mode: None,
                attachments: vec![ProductAttachment::LocalAudio {
                    path: "/tmp/recording.wav".to_owned(),
                }],
            },
            BackendKind::CodexStdio,
        );
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["input"][1]["type"], "localAudio");
        assert_eq!(value["input"][1]["path"], "/tmp/recording.wav");
    }

    #[test]
    fn product_turn_preserves_skill_and_mention_for_codex_wire_input() {
        let params = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                text: "use these".to_owned(),
                model: Some("gpt-5".to_owned()),
                reasoning_effort: None,
                working_dir: None,
                access_mode: None,
                speed_mode: None,
                work_mode: None,
                attachments: vec![
                    ProductAttachment::Skill {
                        name: "release".to_owned(),
                        path: "/skills/release/SKILL.md".to_owned(),
                    },
                    ProductAttachment::Mention {
                        name: "Cargo.toml".to_owned(),
                        path: "/workspace/Cargo.toml".to_owned(),
                    },
                ],
            },
            BackendKind::CodexStdio,
        );
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(
            value["input"][1],
            serde_json::json!({
                "type": "skill",
                "name": "release",
                "path": "/skills/release/SKILL.md"
            })
        );
        assert_eq!(
            value["input"][2],
            serde_json::json!({
                "type": "mention",
                "name": "Cargo.toml",
                "path": "/workspace/Cargo.toml"
            })
        );
    }

    #[test]
    fn codex_auto_access_serializes_schema_exact_named_profile() {
        let params = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                text: "modify the workspace".to_owned(),
                model: None,
                reasoning_effort: None,
                working_dir: Some("/workspace/project".to_owned()),
                access_mode: Some(ProductAccessMode::CodexAuto),
                speed_mode: None,
                work_mode: None,
                attachments: Vec::new(),
            },
            BackendKind::CodexStdio,
        );

        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["cwd"], "/workspace/project");
        assert_eq!(value["permissions"], crate::WORKSPACE_PROFILE_ID);
        assert!(value.get("approvalPolicy").is_none());
        assert!(value.get("approvalsReviewer").is_none());
        assert_eq!(
            value["runtimeWorkspaceRoots"],
            serde_json::json!(["/workspace/project"])
        );
        assert!(value.get("sandboxPolicy").is_none());
        assert!(value.get("mitsuroPermissionMode").is_none());
    }

    #[test]
    fn codex_thread_access_presets_keep_exact_named_profiles() {
        for (mode, profile) in [
            (
                ProductAccessMode::CodexReadOnly,
                crate::READ_ONLY_PROFILE_ID,
            ),
            (ProductAccessMode::CodexAuto, crate::WORKSPACE_PROFILE_ID),
            (
                ProductAccessMode::CodexFullAccess,
                crate::FULL_ACCESS_PROFILE_ID,
            ),
        ] {
            let mut params = ThreadStartParams {
                cwd: Some("/workspace/project".to_owned()),
                ..Default::default()
            };
            apply_access_to_thread_params(&mut params, BackendKind::CodexStdio, Some(mode));
            let value = serde_json::to_value(params).unwrap();
            assert_eq!(value["permissions"], profile);
            assert!(value.get("approvalPolicy").is_none());
            assert!(value.get("approvalsReviewer").is_none());
            assert!(value.get("sandbox").is_none());
            assert_eq!(
                value["runtimeWorkspaceRoots"],
                serde_json::json!(["/workspace/project"])
            );
            assert!(value.get("mitsuroPermissionMode").is_none());
        }
    }

    #[test]
    fn access_without_an_absolute_workspace_does_not_clear_runtime_roots() {
        let mut params = ThreadStartParams::default();
        apply_access_to_thread_params(
            &mut params,
            BackendKind::CodexStdio,
            Some(ProductAccessMode::CodexFullAccess),
        );
        let value = serde_json::to_value(params).unwrap();
        assert!(value.get("runtimeWorkspaceRoots").is_none());
    }

    #[test]
    fn mitsuro_access_stays_out_of_codex_wire_json() {
        let params = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                text: "inspect".to_owned(),
                model: None,
                reasoning_effort: None,
                working_dir: Some("/workspace/project".to_owned()),
                access_mode: Some(ProductAccessMode::MitsuroSupervised),
                speed_mode: None,
                work_mode: None,
                attachments: Vec::new(),
            },
            BackendKind::MitsuroHttp,
        );
        assert_eq!(
            params.mitsuro_permission_mode.as_deref(),
            Some("supervised")
        );
        let value = serde_json::to_value(params).unwrap();
        assert!(value.get("mitsuroPermissionMode").is_none());
        assert!(value.get("approvalPolicy").is_none());
        assert!(value.get("sandboxPolicy").is_none());
    }

    #[test]
    fn backend_speed_modes_keep_their_exact_wire_semantics() {
        let codex = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                text: "go faster".to_owned(),
                model: Some("gpt-5.6-sol".to_owned()),
                reasoning_effort: None,
                working_dir: None,
                access_mode: None,
                speed_mode: Some(ProductSpeedMode::CodexServiceTier("priority".to_owned())),
                work_mode: None,
                attachments: Vec::new(),
            },
            BackendKind::CodexStdio,
        );
        let value = serde_json::to_value(codex).unwrap();
        assert_eq!(value["serviceTier"], "priority");
        assert!(value.get("mitsuroFastMode").is_none());

        let mitsuro = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                text: "go faster".to_owned(),
                model: Some("grok-4.5".to_owned()),
                reasoning_effort: None,
                working_dir: None,
                access_mode: None,
                speed_mode: Some(ProductSpeedMode::MitsuroFast),
                work_mode: None,
                attachments: Vec::new(),
            },
            BackendKind::MitsuroHttp,
        );
        assert_eq!(mitsuro.mitsuro_fast_mode, Some(true));
        let value = serde_json::to_value(mitsuro).unwrap();
        assert_eq!(value["serviceTier"], serde_json::Value::Null);
        assert!(value.get("mitsuroFastMode").is_none());
    }

    #[test]
    fn standard_codex_speed_explicitly_clears_a_sticky_service_tier() {
        let params = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                text: "standard speed".to_owned(),
                model: None,
                reasoning_effort: None,
                working_dir: None,
                access_mode: None,
                speed_mode: Some(ProductSpeedMode::CodexStandard),
                work_mode: None,
                attachments: Vec::new(),
            },
            BackendKind::CodexStdio,
        );
        assert_eq!(
            serde_json::to_value(params).unwrap()["serviceTier"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn backend_work_modes_keep_their_exact_wire_semantics() {
        let codex = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                text: "make a plan".to_owned(),
                model: Some("gpt-5.6-sol".to_owned()),
                reasoning_effort: Some("high".to_owned()),
                working_dir: None,
                access_mode: None,
                speed_mode: Some(ProductSpeedMode::CodexStandard),
                work_mode: Some(ProductWorkMode::Codex {
                    mode: crate::environment::ModeKind::Plan,
                    model: "gpt-5.6-sol".to_owned(),
                    reasoning_effort: Some("medium".to_owned()),
                }),
                attachments: Vec::new(),
            },
            BackendKind::CodexStdio,
        );
        let value = serde_json::to_value(codex).unwrap();
        assert!(value.get("model").is_none());
        assert!(value.get("effort").is_none());
        assert_eq!(value["collaborationMode"]["mode"], "plan");
        assert_eq!(
            value["collaborationMode"]["settings"]["model"],
            "gpt-5.6-sol"
        );
        assert_eq!(
            value["collaborationMode"]["settings"]["reasoning_effort"],
            "medium"
        );
        assert!(value["collaborationMode"]["settings"]
            .get("developer_instructions")
            .is_none());

        let mitsuro = product_turn_params(
            ProductTurn {
                session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                text: "make a plan".to_owned(),
                model: Some("grok-4.5".to_owned()),
                reasoning_effort: None,
                working_dir: None,
                access_mode: None,
                speed_mode: Some(ProductSpeedMode::MitsuroStandard),
                work_mode: Some(ProductWorkMode::MitsuroPlan),
                attachments: Vec::new(),
            },
            BackendKind::MitsuroHttp,
        );
        assert_eq!(mitsuro.mitsuro_work_mode.as_deref(), Some("plan"));
        let value = serde_json::to_value(mitsuro).unwrap();
        assert!(value.get("mitsuroWorkMode").is_none());
        assert!(value.get("collaborationMode").is_none());
    }

    #[test]
    fn access_mode_for_another_backend_is_rejected_before_io() {
        let backend = DesktopBackend::codex_stdio();
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let error = backend
            .run_product_turn_with_bridge_blocking(
                ProductTurn {
                    session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                    text: "hello".to_owned(),
                    model: None,
                    reasoning_effort: None,
                    working_dir: None,
                    access_mode: Some(ProductAccessMode::MitsuroAutonomous),
                    speed_mode: None,
                    work_mode: None,
                    attachments: Vec::new(),
                },
                event_tx,
                Arc::new(LiveApprovalBridge::new()),
                Duration::from_secs(1),
            )
            .expect_err("mismatched access mode must fail before process I/O");
        assert!(error
            .to_string()
            .contains("does not accept the selected access mode"));
    }

    #[test]
    fn speed_mode_for_another_backend_is_rejected_before_io() {
        let backend = DesktopBackend::codex_stdio();
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let error = backend
            .run_product_turn_with_bridge_blocking(
                ProductTurn {
                    session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                    text: "hello".to_owned(),
                    model: None,
                    reasoning_effort: None,
                    working_dir: None,
                    access_mode: None,
                    speed_mode: Some(ProductSpeedMode::MitsuroFast),
                    work_mode: None,
                    attachments: Vec::new(),
                },
                event_tx,
                Arc::new(LiveApprovalBridge::new()),
                Duration::from_secs(1),
            )
            .expect_err("mismatched speed mode must fail before process I/O");
        assert!(error
            .to_string()
            .contains("does not accept the selected speed mode"));
    }

    #[test]
    fn work_mode_for_another_backend_is_rejected_before_io() {
        let backend = DesktopBackend::codex_stdio();
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let error = backend
            .run_product_turn_with_bridge_blocking(
                ProductTurn {
                    session_id: BackendSessionId::new(BackendKind::CodexStdio, "thread-7"),
                    text: "hello".to_owned(),
                    model: None,
                    reasoning_effort: None,
                    working_dir: None,
                    access_mode: None,
                    speed_mode: Some(ProductSpeedMode::CodexStandard),
                    work_mode: Some(ProductWorkMode::MitsuroPlan),
                    attachments: Vec::new(),
                },
                event_tx,
                Arc::new(LiveApprovalBridge::new()),
                Duration::from_secs(1),
            )
            .expect_err("mismatched work mode must fail before process I/O");
        assert!(error
            .to_string()
            .contains("does not accept the selected work mode"));
    }

    #[test]
    fn mitsuro_rejects_product_audio_before_network_io() {
        let backend = DesktopBackend::mitsuro_from_env().expect("default Mitsuro backend");
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let error = backend
            .run_product_turn_with_bridge_blocking(
                ProductTurn {
                    session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                    text: "transcribe".to_owned(),
                    model: None,
                    reasoning_effort: None,
                    working_dir: None,
                    access_mode: None,
                    speed_mode: None,
                    work_mode: None,
                    attachments: vec![ProductAttachment::LocalAudio {
                        path: "/tmp/recording.wav".to_owned(),
                    }],
                },
                event_tx,
                Arc::new(LiveApprovalBridge::new()),
                Duration::from_secs(1),
            )
            .expect_err("Mitsuro audio must fail before network I/O");
        assert!(error
            .to_string()
            .contains("does not accept audio attachments"));
    }

    #[test]
    fn mitsuro_rejects_product_skill_and_mention_before_network_io() {
        for (attachment, expected) in [
            (
                ProductAttachment::Skill {
                    name: "release".to_owned(),
                    path: "/skills/release/SKILL.md".to_owned(),
                },
                "does not accept Codex skill inputs",
            ),
            (
                ProductAttachment::Mention {
                    name: "Cargo.toml".to_owned(),
                    path: "/workspace/Cargo.toml".to_owned(),
                },
                "does not accept Codex mention inputs",
            ),
        ] {
            let backend = DesktopBackend::mitsuro_from_env().expect("default Mitsuro backend");
            let (event_tx, _event_rx) = std::sync::mpsc::channel();
            let error = backend
                .run_product_turn_with_bridge_blocking(
                    ProductTurn {
                        session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                        text: "use this".to_owned(),
                        model: None,
                        reasoning_effort: None,
                        working_dir: None,
                        access_mode: None,
                        speed_mode: None,
                        work_mode: None,
                        attachments: vec![attachment],
                    },
                    event_tx,
                    Arc::new(LiveApprovalBridge::new()),
                    Duration::from_secs(1),
                )
                .expect_err("Mitsuro references must fail before network I/O");
            assert!(error.to_string().contains(expected));
        }
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
                    reasoning_effort: None,
                    working_dir: None,
                    access_mode: None,
                    speed_mode: None,
                    work_mode: None,
                    attachments: Vec::new(),
                },
                event_tx,
                Arc::new(LiveApprovalBridge::new()),
                Duration::from_secs(1),
            )
            .expect_err("mismatched origin must fail");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }

    #[tokio::test]
    async fn product_steer_rejects_a_session_from_another_backend_before_io() {
        let backend = DesktopBackend::codex_stdio();
        let error = backend
            .steer_session(ProductSteer {
                session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                expected_turn_id: "turn-1".to_owned(),
                text: "change direction".to_owned(),
            })
            .await
            .expect_err("mismatched origin must fail");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }

    #[test]
    fn product_review_rejects_a_session_from_another_backend_before_io() {
        let backend = DesktopBackend::codex_stdio();
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let error = backend
            .run_product_review_with_bridge_blocking(
                ProductReview {
                    session_id: BackendSessionId::new(BackendKind::MitsuroHttp, "session-7"),
                    target: ProductReviewTarget::UncommittedChanges,
                    detached: false,
                },
                event_tx,
                Arc::new(LiveApprovalBridge::new()),
                Duration::from_secs(1),
            )
            .expect_err("mismatched origin must fail");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }

    #[test]
    fn hydrated_transcript_preserves_structured_activity_kinds() {
        let command = CommandExecutionFields {
            command: "cargo test".to_owned(),
            cwd: "/workspace".to_owned(),
            status: "completed".to_owned(),
            output: "ok".to_owned(),
        };
        let message = conversation_message_from_transcript(TranscriptMessage {
            role: TranscriptRole::CommandExecution,
            body: "$ cargo test (completed)\nok".to_owned(),
            item_id: Some("item-7".to_owned()),
            command: Some(command.clone()),
            file_change: None,
            activity: None,
            images: Vec::new(),
            audio: Vec::new(),
            references: Vec::new(),
        });

        assert_eq!(message.role, MessageRole::CommandExecution);
        assert_eq!(message.command, Some(command));
        assert_eq!(message.item_id.as_deref(), Some("item-7"));
    }
}
