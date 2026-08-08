//! Product-domain contracts shared by the native desktop UI and transport adapters.
//!
//! These types intentionally avoid Codex app-server method names. Transport-specific
//! protocol objects stay inside the adapter implementations.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    AgentError, BackendKind, BackendSessionId, DesktopBackend, LiveApprovalBridge, LiveTurnOutcome,
    ModelListParams, Result, ThreadDeleteParams, ThreadListParams, ThreadReadParams,
    ThreadSetNameParams, ThreadStartParams, TranscriptRole, TurnInterruptParams, TurnStartParams,
    TurnStreamEvent,
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
        Ok(SessionConversation { session, messages })
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
