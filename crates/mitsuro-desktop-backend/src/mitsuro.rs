//! Mitsuro HTTP/SSE backend adapter.

use async_trait::async_trait;
use futures::StreamExt as _;
use mitsuro_client::{
    ChatRequest, ChatStreamEvent, CreateSessionRequest, MitsuroClient, SessionType,
    UpdateSessionRequest,
};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::account::{
    CancelLoginAccountParams, CancelLoginAccountResponse, GetAccountParams,
    GetAccountRateLimitsResponse, GetAccountResponse, GetAccountTokenUsageResponse,
    LoginAccountParams, LoginAccountResponse, LogoutAccountResponse,
};
use crate::approvals::{ApprovalChoice, ApprovalKind, PendingApproval};
use crate::backend::AgentBackend;
use crate::environment::{
    CollaborationModeListParams, CollaborationModeListResponse, EnvironmentAddParams,
    EnvironmentAddResponse, EnvironmentInfoParams, EnvironmentInfoResponse,
    EnvironmentStatusParams, EnvironmentStatusResponse,
};
use crate::extensions::{
    ListMcpServerStatusParams, ListMcpServerStatusResponse, McpAuthStatus, McpServerInfo,
    McpServerStatus, McpServerToolCallParams, McpServerToolCallResponse, PluginAuthPolicy,
    PluginAvailability, PluginInstallPolicy, PluginInstalledParams, PluginInstalledResponse,
    PluginInterface, PluginListParams, PluginListResponse, PluginMarketplaceEntry,
    PluginReadParams, PluginReadResponse, PluginSource, PluginSummary,
};
use crate::fs::{
    fuzzy_score_name, FsGetMetadataParams, FsGetMetadataResponse, FsReadDirectoryEntry,
    FsReadDirectoryParams, FsReadDirectoryResponse, FsReadFileParams, FsReadFileResponse,
    FuzzyFileSearchMatchType, FuzzyFileSearchParams, FuzzyFileSearchResponse,
    FuzzyFileSearchResult, FuzzyFileSearchSessionStartParams, FuzzyFileSearchSessionStartResponse,
    FuzzyFileSearchSessionStopParams, FuzzyFileSearchSessionStopResponse,
    FuzzyFileSearchSessionUpdateParams, FuzzyFileSearchSessionUpdateResponse,
};
use crate::live_turn::{LiveApprovalBridge, LiveTurnOutcome};
use crate::process::{
    ProcessKillParams, ProcessKillResponse, ProcessResizePtyParams, ProcessResizePtyResponse,
    ProcessSpawnParams, ProcessSpawnResponse, ProcessWriteStdinParams, ProcessWriteStdinResponse,
};
use crate::protocol::{
    ConfigReadParams, ConfigReadResponse, InitializeResponse, ModelInfo, ModelListParams,
    ModelListResponse, SkillMetadata, SkillsListEntry, SkillsListParams, SkillsListResponse,
    ThreadArchiveParams, ThreadArchiveResponse, ThreadDeleteParams, ThreadDeleteResponse,
    ThreadForkParams, ThreadForkResponse, ThreadGoalClearParams, ThreadGoalClearResponse,
    ThreadGoalGetParams, ThreadGoalGetResponse, ThreadGoalSetParams, ThreadGoalSetResponse,
    ThreadListParams, ThreadListResponse, ThreadReadParams, ThreadReadResponse, ThreadResumeParams,
    ThreadResumeResponse, ThreadSearchParams, ThreadSearchResponse, ThreadSetNameParams,
    ThreadSetNameResponse, ThreadStartParams, ThreadStartResponse, ThreadUnarchiveParams,
    ThreadUnarchiveResponse, TurnInterruptParams, TurnInterruptResponse, TurnStartParams,
    TurnStartResponse,
};
use crate::types::{AgentError, ConnectionStatus, ItemKind, Result, TurnStreamEvent};

/// Adapter for a local or authenticated remote Mitsuro server.
#[derive(Debug)]
pub struct MitsuroServerBackend {
    client: MitsuroClient,
    status: RwLock<ConnectionStatus>,
    next_turn_id: AtomicU64,
}

impl MitsuroServerBackend {
    pub fn new() -> Self {
        Self::from_url("http://127.0.0.1:3000", None)
            .expect("the built-in Mitsuro loopback URL is valid")
    }

    pub fn from_url(base_url: impl Into<String>, bearer_token: Option<&str>) -> Result<Self> {
        let base_url = base_url.into();
        let client = match bearer_token {
            Some(token) => MitsuroClient::with_bearer_token(base_url, token),
            None => MitsuroClient::new(base_url),
        }
        .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(Self {
            client,
            status: RwLock::new(ConnectionStatus::Disconnected),
            next_turn_id: AtomicU64::new(1),
        })
    }

    pub fn from_env() -> Result<Self> {
        let url = std::env::var("MITSURO_SERVER_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_owned());
        let token = std::env::var("MITSURO_SERVER_TOKEN").ok();
        Self::from_url(url, token.as_deref())
    }

    pub fn client(&self) -> &MitsuroClient {
        &self.client
    }

    fn set_status(&self, status: ConnectionStatus) {
        if let Ok(mut current) = self.status.write() {
            *current = status;
        }
    }
}

impl Default for MitsuroServerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MitsuroServerBackend {
    pub async fn run_turn_streaming(
        &self,
        params: TurnStartParams,
        event_tx: std::sync::mpsc::Sender<TurnStreamEvent>,
        bridge: Arc<LiveApprovalBridge>,
        overall_timeout: Duration,
    ) -> Result<LiveTurnOutcome> {
        let thread_id = params.thread_id;
        let text = turn_input_text(&params.input);
        let turn_id = format!(
            "mitsuro-turn-{}",
            self.next_turn_id.fetch_add(1, Ordering::Relaxed)
        );
        let assistant_item_id = format!("{turn_id}-assistant");
        let reasoning_item_id = format!("{turn_id}-reasoning");
        let request = ChatRequest {
            session_id: Some(thread_id.clone()),
            message: text,
            content: Vec::new(),
            project_dir: None,
            working_dir: params.cwd,
            workspace_mode: None,
            target_branch: None,
            session_type: Some(SessionType::Code),
            model: params.model,
            model_key: None,
            thinking_enabled: None,
            fast_mode: None,
            permission_mode: None,
            mode: None,
            research_enabled: None,
        };

        let mut stream = self
            .client
            .chat_stream(request)
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        let _ = event_tx.send(TurnStreamEvent::TurnStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            turn: None,
        });

        let deadline = tokio::time::Instant::now() + overall_timeout;
        let mut event_count = 1usize;
        let mut approvals_answered = 0usize;
        let mut completed = false;
        while let Ok(Some(next)) = tokio::time::timeout_at(deadline, stream.next()).await {
            let event = next.map_err(|error| AgentError::Other(error.to_string()))?;
            match event {
                ChatStreamEvent::TextDelta { delta }
                | ChatStreamEvent::TextDeltaWithCitations { delta, .. } => {
                    let _ = event_tx.send(TurnStreamEvent::AgentMessageDelta {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        item_id: assistant_item_id.clone(),
                        delta,
                    });
                    event_count += 1;
                }
                ChatStreamEvent::ThinkingDelta { thinking } => {
                    let _ = event_tx.send(TurnStreamEvent::ReasoningTextDelta {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        item_id: reasoning_item_id.clone(),
                        content_index: None,
                        delta: thinking,
                    });
                    event_count += 1;
                }
                ChatStreamEvent::ToolCallStart { id, name }
                | ChatStreamEvent::ToolExecuting { id, name } => {
                    let _ = event_tx.send(TurnStreamEvent::ItemStarted {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        item_id: id,
                        kind: ItemKind::CommandExecution,
                        item: Some(serde_json::json!({"name": name})),
                    });
                    event_count += 1;
                }
                ChatStreamEvent::ToolOutputDelta { id, delta } => {
                    let _ = event_tx.send(TurnStreamEvent::CommandExecutionOutputDelta {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        item_id: id,
                        delta,
                    });
                    event_count += 1;
                }
                ChatStreamEvent::ToolResult {
                    id,
                    output,
                    is_error,
                } => {
                    let _ = event_tx.send(TurnStreamEvent::ItemCompleted {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        item_id: id,
                        kind: ItemKind::CommandExecution,
                        text: Some(output),
                        item: Some(serde_json::json!({"isError": is_error})),
                    });
                    event_count += 1;
                }
                ChatStreamEvent::PlanUpdate { items } => {
                    let text = items
                        .into_iter()
                        .map(|item| {
                            format!(
                                "- [{}] {}",
                                if item.completed { "x" } else { " " },
                                item.content
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let _ = event_tx.send(TurnStreamEvent::PlanDelta {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        item_id: format!("{turn_id}-plan"),
                        delta: text,
                    });
                    event_count += 1;
                }
                ChatStreamEvent::ToolApprovalRequired {
                    id,
                    name,
                    arguments,
                } => {
                    let pending = PendingApproval {
                        request_id: crate::protocol::JsonRpcId::String(id.clone()),
                        method: "mitsuro/tool-approval".to_owned(),
                        kind: ApprovalKind::CommandExecution,
                        title: "Approve tool".to_owned(),
                        summary: name,
                        detail: arguments.to_string(),
                        thread_id: Some(thread_id.clone()),
                        turn_id: Some(turn_id.clone()),
                        raw_params: arguments,
                    };
                    let _ = event_tx.send(TurnStreamEvent::ApprovalRequested(pending));
                    event_count += 1;
                    let choice = tokio::task::spawn_blocking({
                        let bridge = Arc::clone(&bridge);
                        move || bridge.wait()
                    })
                    .await
                    .unwrap_or(ApprovalChoice::Reject);
                    self.client
                        .approve_tool(&thread_id, &id, matches!(choice, ApprovalChoice::Approve))
                        .await
                        .map_err(|error| AgentError::Other(error.to_string()))?;
                    approvals_answered += 1;
                }
                ChatStreamEvent::Finish { stop_reason, .. } => {
                    let _ = event_tx.send(TurnStreamEvent::TurnCompleted {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        status: Some(stop_reason),
                        turn: None,
                    });
                    event_count += 1;
                    completed = true;
                    break;
                }
                ChatStreamEvent::Error { error } => return Err(AgentError::Other(error)),
                other => {
                    let _ = event_tx.send(TurnStreamEvent::Other {
                        method: format!("mitsuro/{other:?}"),
                        params: None,
                    });
                    event_count += 1;
                }
            }
        }

        Ok(LiveTurnOutcome {
            event_count,
            approvals_answered,
            completed,
        })
    }
}

fn turn_input_text(input: &[Value]) -> String {
    input
        .iter()
        .filter_map(|value| value.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn message_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(message_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => map
            .get("text")
            .or_else(|| map.get("content"))
            .map(message_text)
            .unwrap_or_else(|| value.to_string()),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

fn session_json(session: &mitsuro_client::SessionInfo) -> Value {
    serde_json::json!({
        "id": session.id,
        "name": session.title,
        "preview": session.title,
        "cwd": session.working_dir,
        "modelProvider": session.model_key.as_ref().map(|key| key.provider.clone()),
        "ephemeral": false,
        "archived": false,
    })
}

fn collect_fuzzy_matches(
    root: &str,
    query: &str,
    entries: Vec<mitsuro_client::FileTreeEntry>,
    matches: &mut Vec<FuzzyFileSearchResult>,
) {
    for entry in entries {
        if let Some((score, indices)) = fuzzy_score_name(query, &entry.name) {
            matches.push(FuzzyFileSearchResult {
                root: root.to_owned(),
                path: entry.path.clone(),
                match_type: if entry.is_dir {
                    FuzzyFileSearchMatchType::Directory
                } else {
                    FuzzyFileSearchMatchType::File
                },
                file_name: entry.name,
                score,
                indices: Some(indices),
            });
        }
        if let Some(children) = entry.children {
            collect_fuzzy_matches(root, query, children, matches);
        }
    }
}

#[async_trait]
impl AgentBackend for MitsuroServerBackend {
    fn name(&self) -> &'static str {
        "mitsuro"
    }

    fn status(&self) -> ConnectionStatus {
        self.status
            .read()
            .map(|s| s.clone())
            .unwrap_or(ConnectionStatus::Disconnected)
    }

    fn supports_method(&self, method: &str) -> bool {
        matches!(
            method,
            "initialize"
                | "thread/list"
                | "thread/start"
                | "thread/read"
                | "thread/name/set"
                | "thread/delete"
                | "turn/start"
                | "turn/interrupt"
                | "model/list"
                | "skills/list"
                | "fs/readDirectory"
                | "fs/readFile"
                | "fuzzyFileSearch"
                | "mcpServerStatus/list"
                | "plugin/list"
        )
    }

    async fn call_raw(&self, method: &str, _params: Value) -> Result<Value> {
        Err(AgentError::NotImplemented(format!(
            "MitsuroServerBackend::call_raw({method}) — not implemented"
        )))
    }

    async fn connect(&self) -> Result<InitializeResponse> {
        self.set_status(ConnectionStatus::Connecting);
        match self.client.health().await {
            Ok(health) => {
                self.set_status(ConnectionStatus::Ready);
                Ok(InitializeResponse {
                    codex_home: String::new(),
                    platform_family: "mitsuro-http".to_owned(),
                    platform_os: std::env::consts::OS.to_owned(),
                    user_agent: format!("mitsuro-server/{}", health.version),
                })
            }
            Err(error) => {
                self.set_status(ConnectionStatus::Error(error.to_string()));
                Err(AgentError::Other(error.to_string()))
            }
        }
    }

    async fn thread_list(&self, params: ThreadListParams) -> Result<ThreadListResponse> {
        let mut sessions = self
            .client
            .list_sessions()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        if let Some(limit) = params.limit {
            sessions.truncate(limit as usize);
        }
        Ok(ThreadListResponse {
            data: sessions.iter().map(session_json).collect(),
            next_cursor: None,
            backwards_cursor: None,
        })
    }

    async fn thread_start(&self, params: ThreadStartParams) -> Result<ThreadStartResponse> {
        let session = self
            .client
            .create_session(CreateSessionRequest {
                title: None,
                model: params.model.clone(),
                model_key: None,
                project_dir: None,
                working_dir: params.cwd.clone(),
                workspace_mode: None,
                target_branch: None,
                session_type: Some(SessionType::Code),
                permission_mode: None,
            })
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(ThreadStartResponse {
            thread: session_json(&session),
            model: session.model,
            model_provider: session.model_key.map(|key| key.provider),
            cwd: session.working_dir,
        })
    }

    async fn thread_read(&self, params: ThreadReadParams) -> Result<ThreadReadResponse> {
        let transcript = self
            .client
            .get_session(&params.thread_id)
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        let items = transcript
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let text = message_text(&message.content);
                if message.role == "user" {
                    serde_json::json!({
                        "id": format!("mitsuro-message-{index}"),
                        "type": "userMessage",
                        "content": [{"type": "text", "text": text}],
                    })
                } else {
                    serde_json::json!({
                        "id": format!("mitsuro-message-{index}"),
                        "type": "agentMessage",
                        "text": text,
                    })
                }
            })
            .collect::<Vec<_>>();
        let mut thread = session_json(&transcript.session);
        if let Some(object) = thread.as_object_mut() {
            object.insert(
                "turns".to_owned(),
                serde_json::json!([{"id": "mitsuro-history", "items": items}]),
            );
        }
        Ok(ThreadReadResponse { thread })
    }

    async fn model_list(&self, _params: ModelListParams) -> Result<ModelListResponse> {
        let response = self
            .client
            .list_models()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        let default = response.default_model;
        let data = response
            .models
            .into_iter()
            .map(|model| ModelInfo {
                id: model.id.clone(),
                model: model.id.clone(),
                display_name: model.display_name,
                description: format!("{} provider", model.provider),
                hidden: false,
                is_default: default.as_deref() == Some(model.id.as_str()),
                default_reasoning_effort: model
                    .default_reasoning_level
                    .map(|effort| format!("{effort:?}").to_lowercase())
                    .unwrap_or_default(),
                supported_reasoning_efforts: Vec::new(),
                upgrade: None,
            })
            .collect();
        Ok(ModelListResponse {
            data,
            next_cursor: None,
        })
    }

    async fn config_read(&self, _params: ConfigReadParams) -> Result<ConfigReadResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::config_read — not implemented".into(),
        ))
    }

    async fn thread_search(&self, _params: ThreadSearchParams) -> Result<ThreadSearchResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_search — not implemented".into(),
        ))
    }

    async fn thread_name_set(&self, params: ThreadSetNameParams) -> Result<ThreadSetNameResponse> {
        self.client
            .update_session(
                &params.thread_id,
                UpdateSessionRequest {
                    title: Some(params.name),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(ThreadSetNameResponse::default())
    }

    async fn thread_archive(&self, _params: ThreadArchiveParams) -> Result<ThreadArchiveResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_archive — not implemented".into(),
        ))
    }

    async fn thread_unarchive(
        &self,
        _params: ThreadUnarchiveParams,
    ) -> Result<ThreadUnarchiveResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_unarchive — not implemented".into(),
        ))
    }

    async fn thread_delete(&self, params: ThreadDeleteParams) -> Result<ThreadDeleteResponse> {
        self.client
            .delete_session(&params.thread_id)
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(ThreadDeleteResponse::default())
    }

    async fn thread_fork(&self, _params: ThreadForkParams) -> Result<ThreadForkResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_fork — not implemented".into(),
        ))
    }

    async fn thread_resume(&self, _params: ThreadResumeParams) -> Result<ThreadResumeResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_resume — not implemented".into(),
        ))
    }

    async fn thread_goal_get(&self, _params: ThreadGoalGetParams) -> Result<ThreadGoalGetResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_goal_get — not implemented".into(),
        ))
    }

    async fn thread_goal_set(&self, _params: ThreadGoalSetParams) -> Result<ThreadGoalSetResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_goal_set — not implemented".into(),
        ))
    }

    async fn thread_goal_clear(
        &self,
        _params: ThreadGoalClearParams,
    ) -> Result<ThreadGoalClearResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::thread_goal_clear — not implemented".into(),
        ))
    }

    async fn skills_list(&self, _params: SkillsListParams) -> Result<SkillsListResponse> {
        let skills = self
            .client
            .list_skills()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?
            .into_iter()
            .map(|skill| SkillMetadata {
                name: skill.name,
                description: skill.description,
                enabled: skill.enabled,
                path: skill.path,
                scope: skill.source,
                short_description: None,
            })
            .collect();
        Ok(SkillsListResponse {
            data: vec![SkillsListEntry {
                cwd: String::new(),
                skills,
                errors: Vec::new(),
            }],
        })
    }

    async fn turn_start(&self, _params: TurnStartParams) -> Result<TurnStartResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::turn_start — not implemented".into(),
        ))
    }

    async fn turn_interrupt(&self, params: TurnInterruptParams) -> Result<TurnInterruptResponse> {
        self.client
            .cancel_session(&params.thread_id)
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(TurnInterruptResponse::default())
    }

    async fn process_spawn(&self, _params: ProcessSpawnParams) -> Result<ProcessSpawnResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::process_spawn — not implemented".into(),
        ))
    }

    async fn process_write_stdin(
        &self,
        _params: ProcessWriteStdinParams,
    ) -> Result<ProcessWriteStdinResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::process_write_stdin — not implemented".into(),
        ))
    }

    async fn process_resize_pty(
        &self,
        _params: ProcessResizePtyParams,
    ) -> Result<ProcessResizePtyResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::process_resize_pty — not implemented".into(),
        ))
    }

    async fn process_kill(&self, _params: ProcessKillParams) -> Result<ProcessKillResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::process_kill — not implemented".into(),
        ))
    }

    async fn fs_read_directory(
        &self,
        params: FsReadDirectoryParams,
    ) -> Result<FsReadDirectoryResponse> {
        let tree = self
            .client
            .file_tree(&params.path, 1)
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(FsReadDirectoryResponse {
            entries: tree
                .entries
                .into_iter()
                .map(|entry| {
                    if entry.is_dir {
                        FsReadDirectoryEntry::directory(entry.name)
                    } else {
                        FsReadDirectoryEntry::file(entry.name)
                    }
                })
                .collect(),
        })
    }

    async fn fs_read_file(&self, params: FsReadFileParams) -> Result<FsReadFileResponse> {
        let file = self
            .client
            .read_file(&params.path)
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        Ok(FsReadFileResponse::from_text(&file.content))
    }

    async fn fs_get_metadata(&self, _params: FsGetMetadataParams) -> Result<FsGetMetadataResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::fs_get_metadata — not implemented".into(),
        ))
    }

    async fn fuzzy_file_search(
        &self,
        params: FuzzyFileSearchParams,
    ) -> Result<FuzzyFileSearchResponse> {
        let mut files = Vec::new();
        for root in &params.roots {
            let tree = self
                .client
                .file_tree(root, 10)
                .await
                .map_err(|error| AgentError::Other(error.to_string()))?;
            collect_fuzzy_matches(root, &params.query, tree.entries, &mut files);
        }
        files.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.path.cmp(&right.path))
        });
        files.truncate(200);
        Ok(FuzzyFileSearchResponse { files })
    }

    async fn fuzzy_file_search_session_start(
        &self,
        _params: FuzzyFileSearchSessionStartParams,
    ) -> Result<FuzzyFileSearchSessionStartResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::fuzzy_file_search_session_start — not implemented".into(),
        ))
    }

    async fn fuzzy_file_search_session_update(
        &self,
        _params: FuzzyFileSearchSessionUpdateParams,
    ) -> Result<FuzzyFileSearchSessionUpdateResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::fuzzy_file_search_session_update — not implemented".into(),
        ))
    }

    async fn fuzzy_file_search_session_stop(
        &self,
        _params: FuzzyFileSearchSessionStopParams,
    ) -> Result<FuzzyFileSearchSessionStopResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::fuzzy_file_search_session_stop — not implemented".into(),
        ))
    }

    async fn mcp_server_status_list(
        &self,
        _params: ListMcpServerStatusParams,
    ) -> Result<ListMcpServerStatusResponse> {
        let data = self
            .client
            .list_mcp_servers()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?
            .into_iter()
            .map(|server| McpServerStatus {
                name: server.name.clone(),
                server_info: Some(McpServerInfo {
                    name: server.name,
                    version: String::new(),
                    title: None,
                    description: Some(server.status),
                    website_url: None,
                }),
                tools: server
                    .tools
                    .into_iter()
                    .filter_map(|tool| {
                        let name = tool.get("name")?.as_str()?.to_owned();
                        Some((name, tool))
                    })
                    .collect(),
                resources: Vec::new(),
                resource_templates: Vec::new(),
                auth_status: if server.connected {
                    McpAuthStatus::Unsupported
                } else {
                    McpAuthStatus::NotLoggedIn
                },
            })
            .collect();
        Ok(ListMcpServerStatusResponse {
            data,
            next_cursor: None,
        })
    }

    async fn mcp_server_tool_call(
        &self,
        _params: McpServerToolCallParams,
    ) -> Result<McpServerToolCallResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::mcp_server_tool_call — not implemented".into(),
        ))
    }

    async fn plugin_list(&self, _params: PluginListParams) -> Result<PluginListResponse> {
        let overview = self
            .client
            .list_extensions()
            .await
            .map_err(|error| AgentError::Other(error.to_string()))?;
        let plugins = overview
            .extensions
            .into_iter()
            .map(|extension| PluginSummary {
                id: extension.id,
                name: extension.name.clone(),
                source: PluginSource::Local {
                    path: extension.path,
                },
                installed: true,
                enabled: true,
                install_policy: PluginInstallPolicy::NotAvailable,
                auth_policy: PluginAuthPolicy::OnUse,
                availability: PluginAvailability::Available,
                version: Some(extension.version.clone()),
                local_version: Some(extension.version),
                remote_plugin_id: None,
                interface: Some(PluginInterface {
                    display_name: Some(extension.name),
                    short_description: Some(format!(
                        "{} tool(s) · {} command(s)",
                        extension.tools.len(),
                        extension.commands.len()
                    )),
                    category: Some("agent extension".to_owned()),
                    capabilities: extension
                        .tools
                        .into_iter()
                        .chain(extension.commands)
                        .collect(),
                    ..Default::default()
                }),
                keywords: vec!["mitsuro".to_owned(), "extension".to_owned()],
                extra: Default::default(),
            })
            .collect();
        Ok(PluginListResponse {
            marketplaces: vec![PluginMarketplaceEntry {
                name: "Mitsuro agent extensions".to_owned(),
                path: None,
                plugins,
                interface: None,
            }],
            marketplace_load_errors: overview.diagnostics,
            featured_plugin_ids: Vec::new(),
        })
    }

    async fn plugin_read(&self, _params: PluginReadParams) -> Result<PluginReadResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::plugin_read — not implemented".into(),
        ))
    }

    async fn plugin_installed(
        &self,
        _params: PluginInstalledParams,
    ) -> Result<PluginInstalledResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::plugin_installed — not implemented".into(),
        ))
    }

    async fn environment_info(
        &self,
        _params: EnvironmentInfoParams,
    ) -> Result<EnvironmentInfoResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::environment_info — not implemented".into(),
        ))
    }

    async fn environment_status(
        &self,
        _params: EnvironmentStatusParams,
    ) -> Result<EnvironmentStatusResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::environment_status — not implemented".into(),
        ))
    }

    async fn environment_add(
        &self,
        _params: EnvironmentAddParams,
    ) -> Result<EnvironmentAddResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::environment_add — not implemented".into(),
        ))
    }

    async fn collaboration_mode_list(
        &self,
        _params: CollaborationModeListParams,
    ) -> Result<CollaborationModeListResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::collaboration_mode_list — not implemented".into(),
        ))
    }

    async fn account_read(&self, _params: GetAccountParams) -> Result<GetAccountResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::account_read — not implemented".into(),
        ))
    }

    async fn account_login_start(
        &self,
        _params: LoginAccountParams,
    ) -> Result<LoginAccountResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::account_login_start — not implemented".into(),
        ))
    }

    async fn account_login_cancel(
        &self,
        _params: CancelLoginAccountParams,
    ) -> Result<CancelLoginAccountResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::account_login_cancel — not implemented".into(),
        ))
    }

    async fn account_logout(&self) -> Result<LogoutAccountResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::account_logout — not implemented".into(),
        ))
    }

    async fn account_usage_read(&self) -> Result<GetAccountTokenUsageResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::account_usage_read — not implemented".into(),
        ))
    }

    async fn account_rate_limits_read(&self) -> Result<GetAccountRateLimitsResponse> {
        Err(AgentError::NotImplemented(
            "MitsuroServerBackend::account_rate_limits_read — not implemented".into(),
        ))
    }

    async fn disconnect(&self) -> Result<()> {
        if let Ok(mut s) = self.status.write() {
            *s = ConnectionStatus::Disconnected;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_codex_text_input_for_mitsuro_chat() {
        let input = vec![
            serde_json::json!({"type": "text", "text": "hello"}),
            serde_json::json!({"type": "text", "text": "world"}),
        ];
        assert_eq!(turn_input_text(&input), "hello\nworld");
    }

    #[tokio::test]
    async fn live_server_read_only_contract() {
        if std::env::var_os("MITSURO_RUN_SERVER_IT").is_none() {
            eprintln!("skip: set MITSURO_RUN_SERVER_IT=1 for local read-only contract check");
            return;
        }
        let backend = MitsuroServerBackend::from_env().expect("backend configuration");
        let init = backend.connect().await.expect("health/connect");
        assert_eq!(init.platform_family, "mitsuro-http");
        let sessions = backend
            .thread_list(ThreadListParams {
                limit: Some(3),
                ..Default::default()
            })
            .await
            .expect("session list");
        assert!(sessions.data.len() <= 3);
        let models = backend
            .model_list(ModelListParams {
                limit: Some(100),
                ..Default::default()
            })
            .await
            .expect("model list");
        assert!(!models.data.is_empty());
        let workspace = std::env::current_dir()
            .expect("current directory")
            .display()
            .to_string();
        let files = backend
            .fs_read_directory(FsReadDirectoryParams::new(workspace))
            .await
            .expect("file directory");
        assert!(!files.entries.is_empty());
        let skills = backend
            .skills_list(SkillsListParams::default())
            .await
            .expect("skills list");
        assert!(skills.skill_count() > 0);
        backend
            .mcp_server_status_list(ListMcpServerStatusParams::default())
            .await
            .expect("MCP list");
        backend
            .plugin_list(PluginListParams::default())
            .await
            .expect("extension list");
        backend
            .client()
            .list_processes()
            .await
            .expect("process list");
        backend.client().hive_current().await.expect("Hive current");
        backend
            .client()
            .list_hive_schedules()
            .await
            .expect("Hive schedules");
        backend.disconnect().await.expect("disconnect");
    }
}
