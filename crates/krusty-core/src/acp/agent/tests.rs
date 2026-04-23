use std::sync::Arc;

use agent_client_protocol::{
    Agent, InitializeRequest, LoadSessionRequest, NewSessionRequest, ProtocolVersion,
};
use tempfile::tempdir;
use tokio::sync::{Mutex, RwLock};

use super::{negotiate_protocol_version, KrustyAgent};
use crate::acp::processor::PromptProcessor;
use crate::acp::session::SessionManager;
use crate::agent::loop_events::LoopStopReason;
use crate::storage::{
    Database, PartialAssistantState, RecoveryDecision, RecoveryStatus,
    SessionManager as StorageSessionManager, SessionRecoveryState,
};
use crate::tools::registry::ToolRegistry;

fn agent_with_storage(storage: Arc<Mutex<StorageSessionManager>>) -> KrustyAgent {
    let tools = Arc::new(ToolRegistry::new());
    KrustyAgent {
        sessions: Arc::new(SessionManager::with_storage(storage)),
        tools: tools.clone(),
        client_capabilities: RwLock::new(None),
        api_key: RwLock::new(None),
        processor: RwLock::new(PromptProcessor::new(tools)),
        notification_tx: RwLock::new(None),
        current_model: RwLock::new(None),
        available_models: RwLock::new(Vec::new()),
    }
}

#[tokio::test]
async fn test_agent_creation() {
    let agent = KrustyAgent::new();
    assert_eq!(agent.sessions().session_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_new_session() -> anyhow::Result<()> {
    let agent = KrustyAgent::new();

    let request = NewSessionRequest::new("/tmp");
    let response = agent.new_session(request).await?;

    assert!(agent.sessions().has_session(&response.session_id));
    Ok(())
}

#[test]
fn negotiate_protocol_version_rejects_legacy_and_future_versions() {
    let legacy: ProtocolVersion = serde_json::from_str("\"1.0.0\"").expect("legacy version");
    let future: ProtocolVersion = serde_json::from_str("2").expect("future version");

    assert_eq!(negotiate_protocol_version(legacy), ProtocolVersion::LATEST);
    assert_eq!(negotiate_protocol_version(future), ProtocolVersion::LATEST);
}

#[tokio::test]
async fn initialize_negotiates_to_supported_protocol_version() -> anyhow::Result<()> {
    let agent = KrustyAgent::new();
    let future_version: ProtocolVersion = serde_json::from_str("2")?;

    let response = agent
        .initialize(InitializeRequest::new(future_version))
        .await?;

    assert_eq!(response.protocol_version, ProtocolVersion::LATEST);
    Ok(())
}

#[tokio::test]
async fn load_session_restores_recovery_only_storage_session() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let db_path = dir.path().join("test.db");
    let db = Database::new(&db_path)?;
    let storage = Arc::new(Mutex::new(StorageSessionManager::new(db)));
    let storage_session_id = {
        let storage = storage.lock().await;
        storage.create_session("Recovered Session", None, Some("/tmp"))?
    };
    {
        let storage = storage.lock().await;
        storage.update_recovery_state(
            &storage_session_id,
            &SessionRecoveryState::new(
                RecoveryStatus::Interrupted,
                Some(LoopStopReason::ProviderError),
                Some("provider failed".to_string()),
                PartialAssistantState {
                    text: "Partial answer".to_string(),
                    thinking: String::new(),
                    tool_calls: Vec::new(),
                },
                RecoveryDecision::Resumable {
                    latest_user_objective: "Finish the answer".to_string(),
                },
            ),
        )?;
    }

    let agent = agent_with_storage(storage);
    let request = LoadSessionRequest::new(storage_session_id.clone(), "/tmp");
    agent.load_session(request).await?;

    assert_eq!(agent.sessions().session_count(), 1);
    Ok(())
}
