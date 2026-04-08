//! Session management endpoints

use std::path::{Path as StdPath, PathBuf};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use krusty_core::agent::summarizer::{generate_summary, SummarizationResult};
use krusty_core::agent::{
    build_project_context,
    pinch_context::{PinchContext, PinchContextInput},
};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{
    Database, DelegatedRunStore, FileActivityTracker, RankedFile, SessionType, WorkspaceMode,
};
use krusty_core::SessionManager;

use super::session_access::{
    current_user_id, ensure_owned_session, load_agent_state_or_idle, load_owned_session,
    request_workspace_scope,
};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::presence::{remove_presence, snapshot_presence, upsert_presence, SessionPresenceRecord};
use crate::routes::chat::submit_tool_approval;
use crate::types::{
    CreateSessionRequest, MessageResponse, PinchRequest, PinchResponse,
    SessionPresenceClientResponse, SessionPresenceHeartbeatRequest, SessionPresenceResponse,
    SessionResponse, SessionStateResponse, SessionTraceResponse, SessionWithMessagesResponse,
    ToolApprovalRequest, UpdateSessionRequest,
};
use crate::utils::messages::parse_stored_model_messages;
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::{
    normalize_resolved_requested_workspace, resolve_optional_workspace_path,
    resolve_session_working_dir, NormalizedWorkspace, WorkspaceNormalizationPolicy,
};
use crate::AppState;

/// Query params for listing sessions
#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    /// Filter sessions by working directory
    pub working_dir: Option<String>,
}

/// Query params for retrieving a session with messages (pagination)
#[derive(Debug, Deserialize)]
pub struct GetSessionQuery {
    /// Maximum number of messages to return
    pub limit: Option<usize>,
    /// Number of messages to skip (from the beginning)
    pub offset: Option<usize>,
}

/// Query params for retrieving a session trace.
#[derive(Debug, Deserialize)]
pub struct GetSessionTraceQuery {
    /// Maximum number of trace events to return.
    pub limit: Option<usize>,
    /// Return only events strictly after this persisted sequence.
    pub after_sequence: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SessionToolApprovalRequest {
    tool_call_id: String,
    approved: bool,
}

const PINCH_RANKED_FILE_LIMIT: usize = 20;
const PINCH_SUMMARY_FILE_CONTENT_LIMIT: usize = 10;
const PINCH_CONTEXT_FILE_CONTENT_LIMIT: usize = 5;

/// Build the sessions router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_sessions).post(create_session))
        .route("/directories", get(list_directories))
        .route(
            "/:id",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route("/:id/state", get(get_session_state))
        .route("/:id/trace", get(get_session_trace))
        .route(
            "/:id/presence",
            get(get_session_presence).put(heartbeat_session_presence),
        )
        .route(
            "/:id/presence/:client_id",
            axum::routing::delete(remove_session_presence),
        )
        .route("/:id/pinch", post(pinch_session))
        .route("/:id/tool-approval", post(tool_approval_for_session))
}

/// List all sessions, optionally filtered by working directory
async fn list_sessions(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionResponse>>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());
    let working_dir_filter = resolve_optional_workspace_path(
        query.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let sessions =
        session_manager.list_sessions_for_user(working_dir_filter.as_deref(), user_id)?;
    let response: Vec<SessionResponse> = sessions.into_iter().map(Into::into).collect();

    Ok(Json(response))
}

async fn tool_approval_for_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    user: Option<CurrentUser>,
    Json(req): Json<SessionToolApprovalRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    submit_tool_approval(
        &state,
        user.as_ref(),
        ToolApprovalRequest {
            session_id,
            tool_call_id: req.tool_call_id,
            approved: req.approved,
        },
    )
    .await
}

/// List all directories that have sessions
async fn list_directories(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<Vec<String>>, AppError> {
    let session_manager = open_session_manager(&state)?;

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let directories = session_manager.list_session_directories_for_user(user_id)?;

    Ok(Json(directories))
}

/// Create a new session
async fn create_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), AppError> {
    let session_manager = open_session_manager(&state)?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    let title = req.title.as_deref().unwrap_or("New Session");
    let workspace = normalize_resolved_requested_workspace(
        req.working_dir.as_deref(),
        req.project_dir.as_deref(),
        req.workspace_mode,
        WorkspaceNormalizationPolicy {
            default_mode_without_paths: WorkspaceMode::Neutral,
            selected_fallback_dir: None,
        },
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    let target_branch = req.target_branch.as_deref().map(str::trim).and_then(|b| {
        if b.is_empty() {
            None
        } else {
            Some(b)
        }
    });
    let session_id = session_manager.create_session_for_user_with_config(
        title,
        trimmed_nonempty(req.model.as_deref()),
        workspace.working_dir.as_deref(),
        workspace.project_dir.as_deref(),
        workspace.workspace_mode,
        current_user_id(user.as_ref()),
        target_branch,
        req.session_type.unwrap_or(SessionType::Code),
    )?;

    let session = session_manager
        .get_session(&session_id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch created session".to_string()))?;

    Ok((StatusCode::CREATED, Json(session.into())))
}

/// Get a session with its messages, with optional pagination
async fn get_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<GetSessionQuery>,
) -> Result<Json<SessionWithMessagesResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_session(&session_manager, &id, user.as_ref())?;

    let raw_messages = session_manager.load_session_messages(&id)?;
    let offset = query.offset.unwrap_or(0);
    const MAX_MESSAGE_LIMIT: usize = 10_000;
    let limit = query
        .limit
        .unwrap_or(MAX_MESSAGE_LIMIT)
        .min(MAX_MESSAGE_LIMIT);

    let messages: Vec<MessageResponse> = raw_messages
        .into_iter()
        .skip(offset)
        .take(limit)
        .filter_map(
            |(role, content_json)| match serde_json::from_str(&content_json) {
                Ok(content) => Some(MessageResponse { role, content }),
                Err(e) => {
                    tracing::warn!("Failed to parse message content for role '{}': {}", role, e);
                    None
                }
            },
        )
        .collect();

    Ok(Json(SessionWithMessagesResponse {
        session: session.into(),
        messages,
    }))
}

/// Update a session's title
async fn update_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    if req.title.is_none()
        && req.working_dir.is_none()
        && req.project_dir.is_none()
        && req.workspace_mode.is_none()
        && req.mode.is_none()
        && req.model.is_none()
        && req.target_branch.is_none()
    {
        return Err(AppError::BadRequest(
            "At least one of title, working_dir, project_dir, workspace_mode, mode, model, or target_branch must be provided".to_string(),
        ));
    }

    if let Some(title) = req.title.as_deref() {
        session_manager.update_session_title(&id, title)?;
    }

    let workspace_update = resolve_workspace_update(
        &req,
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;

    if workspace_update.is_none() {
        let working_dir = resolve_optional_workspace_path(
            req.working_dir.as_deref(),
            &workspace_scope.base_dir,
            &workspace_scope.allowed_root,
        )?;
        if req.working_dir.is_some() {
            session_manager.update_session_working_dir(&id, working_dir.as_deref())?;
        }
    }

    if let Some(workspace) = workspace_update {
        session_manager.update_session_workspace(
            &id,
            workspace.project_dir.as_deref(),
            workspace.workspace_mode,
        )?;
    }

    if let Some(mode) = req.mode {
        session_manager.update_session_work_mode(&id, mode)?;
    }

    if let Some(model) = req.model.as_deref() {
        let normalized = trimmed_nonempty(Some(model));
        session_manager.update_session_model(&id, normalized)?;
    }

    if let Some(target_branch) = req.target_branch.as_deref() {
        let normalized = trimmed_nonempty(Some(target_branch));
        session_manager.update_session_target_branch(&id, normalized)?;
    }

    let session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch updated session".to_string()))?;

    Ok(Json(session.into()))
}

fn resolve_workspace_update(
    req: &UpdateSessionRequest,
    workspace_base: &StdPath,
    allowed_root: &StdPath,
) -> Result<Option<NormalizedWorkspace>, AppError> {
    if req.project_dir.is_none() && req.workspace_mode.is_none() {
        return Ok(None);
    }

    let project_hint = trimmed_nonempty(req.project_dir.as_deref())
        .or(trimmed_nonempty(req.working_dir.as_deref()));
    let workspace = normalize_resolved_requested_workspace(
        req.working_dir.as_deref(),
        req.project_dir.as_deref(),
        req.workspace_mode,
        WorkspaceNormalizationPolicy {
            default_mode_without_paths: WorkspaceMode::Neutral,
            selected_fallback_dir: None,
        },
        workspace_base,
        allowed_root,
    )?;

    match workspace.workspace_mode {
        WorkspaceMode::Neutral if project_hint.is_some() => Err(AppError::BadRequest(
            "workspace mode 'neutral' cannot include a project_dir".to_string(),
        )),
        WorkspaceMode::Selected | WorkspaceMode::Created if workspace.project_dir.is_none() => {
            Err(AppError::BadRequest(
                "workspace modes 'selected' and 'created' require a project_dir".to_string(),
            ))
        }
        _ => Ok(Some(workspace)),
    }
}

/// Delete a session
async fn delete_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    session_manager.delete_session(&id)?;

    let mut locks = state.session_locks.write().await;
    locks.remove(&id);

    Ok(StatusCode::NO_CONTENT)
}

/// Get session agent state
///
/// Returns the current agent execution state (idle, streaming, tool_executing, etc.)
/// Used by frontend to determine if session has active processing.
async fn get_session_state(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<SessionStateResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_session(&session_manager, &id, user.as_ref())?;

    // Get agent state
    let agent_state = load_agent_state_or_idle(&session_manager, &id)?;
    let recovery = session_manager.load_recovery_state(&id)?;
    let live_partial_assistant =
        live_partial_assistant_for_state(&agent_state.state, recovery.as_ref());
    let last_event_sequence = session_manager.load_runtime_trace_latest_sequence(&id)?;
    let delegated_tools = state
        .delegated_state
        .read()
        .await
        .get(&id)
        .cloned()
        .unwrap_or_default();
    let recent_delegated_runs = DelegatedRunStore::new(Database::new(&state.db_path)?)
        .list_runs_for_session(&id, 20)?
        .into_iter()
        .map(Into::into)
        .collect();

    Ok(Json(SessionStateResponse {
        id,
        agent_state: agent_state.state,
        started_at: agent_state.started_at,
        last_event_at: agent_state.last_event_at,
        mode: session.work_mode,
        recovery,
        live_partial_assistant,
        delegated_tools,
        recent_delegated_runs,
        last_event_sequence,
    }))
}

/// Get compact runtime trace summary and recent events for a session.
async fn get_session_trace(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<GetSessionTraceQuery>,
) -> Result<Json<SessionTraceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    const DEFAULT_TRACE_LIMIT: usize = 200;
    const MAX_TRACE_LIMIT: usize = 1_000;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_TRACE_LIMIT)
        .min(MAX_TRACE_LIMIT);
    let summary = session_manager.load_runtime_trace_summary(&id)?;
    let latest_sequence = session_manager.load_runtime_trace_latest_sequence(&id)?;
    let events = match query.after_sequence {
        Some(after_sequence) => {
            session_manager.load_runtime_trace_events_after(&id, after_sequence, Some(limit))?
        }
        None => session_manager.load_runtime_trace_events(&id, Some(limit))?,
    };

    Ok(Json(SessionTraceResponse {
        id,
        summary,
        events,
        latest_sequence,
    }))
}

async fn get_session_presence(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<SessionPresenceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    let mut registry = state.session_presence.write().await;
    Ok(Json(map_presence_snapshot(snapshot_presence(
        &mut registry,
        &id,
    ))))
}

async fn heartbeat_session_presence(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<SessionPresenceHeartbeatRequest>,
) -> Result<Json<SessionPresenceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    let mut registry = state.session_presence.write().await;
    let snapshot = upsert_presence(
        &mut registry,
        &id,
        SessionPresenceRecord {
            client_id: req.client_id,
            surface: req.surface,
            capability: req.capability,
            user_id: current_user_id(user.as_ref()).map(ToOwned::to_owned),
            last_seen_at: chrono::Utc::now(),
            last_event_sequence: req.last_event_sequence,
        },
    );

    Ok(Json(map_presence_snapshot(snapshot)))
}

async fn remove_session_presence(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((id, client_id)): Path<(String, String)>,
) -> Result<Json<SessionPresenceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    let mut registry = state.session_presence.write().await;
    Ok(Json(map_presence_snapshot(remove_presence(
        &mut registry,
        &id,
        &client_id,
    ))))
}

fn live_partial_assistant_for_state(
    agent_state: &str,
    recovery: Option<&krusty_core::storage::SessionRecoveryState>,
) -> Option<krusty_core::storage::PartialAssistantState> {
    if matches!(
        agent_state,
        "streaming" | "tool_executing" | "awaiting_input"
    ) {
        return recovery.map(|recovery| recovery.partial_assistant.clone());
    }
    None
}

fn map_presence_snapshot(
    snapshot: crate::presence::SessionPresenceSnapshot,
) -> SessionPresenceResponse {
    SessionPresenceResponse {
        session_id: snapshot.session_id,
        active_viewers: snapshot.active_viewers,
        active_controllers: snapshot.active_controllers,
        stale_clients: snapshot.stale_clients,
        clients: snapshot
            .clients
            .into_iter()
            .map(|client| SessionPresenceClientResponse {
                client_id: client.client_id,
                surface: client.surface,
                capability: client.capability,
                user_id: client.user_id,
                last_seen_at: client.last_seen_at,
                last_event_sequence: client.last_event_sequence,
                stale: client.stale,
            })
            .collect(),
    }
}

/// Pinch a session - create a child session with summarized context
async fn pinch_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<PinchRequest>,
) -> Result<Json<PinchResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let source_session = load_owned_session(&session_manager, &id, user.as_ref())?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    // Load messages and convert to ModelMessage format
    let raw_messages = session_manager.load_session_messages(&id)?;
    let messages = parse_stored_model_messages(&id, raw_messages, "pinch context");

    if messages.is_empty() {
        return Err(AppError::BadRequest(
            "Cannot pinch session with no messages".to_string(),
        ));
    }

    let working_dir = resolve_session_working_dir(
        source_session.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    let ranked_files = ranked_files_for_pinch(&session_manager, &id);
    let file_contents = load_key_file_contents(
        &id,
        &working_dir,
        &ranked_files,
        PINCH_SUMMARY_FILE_CONTENT_LIMIT,
    );
    let project_context = load_project_context(&working_dir);

    // Generate summary using AI if configured, otherwise use defaults.
    let summary_model = source_session.model.as_deref();
    let summary_result = if let Some(ai_client) = state.resolve_ai_client(summary_model).await {
        generate_summary(
            &ai_client,
            &messages,
            req.preservation_hints.as_deref(),
            &ranked_files,
            &file_contents,
            project_context.as_deref(),
            summary_model,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("Summarization failed, using defaults: {}", e);
            SummarizationResult::default()
        })
    } else {
        SummarizationResult::default()
    };

    // Create pinch context
    let active_plan = load_active_plan_markdown_for_pinch(&state, &id);
    let key_file_contents = file_contents
        .iter()
        .take(PINCH_CONTEXT_FILE_CONTENT_LIMIT)
        .cloned()
        .collect();

    let pinch_ctx = PinchContext::from_input(PinchContextInput {
        source_session_id: id.clone(),
        source_session_title: source_session.title.clone(),
        summary: summary_result.clone(),
        ranked_files,
        preservation_hints: req.preservation_hints,
        direction: req.direction,
        project_context,
        key_file_contents,
        active_plan,
    });

    // Create the child session
    let new_title = format!("{} (continued)", source_session.title);
    let working_dir_for_child = working_dir.to_string_lossy().to_string();
    let model_for_child = source_session.model.as_deref();
    let new_session_id = session_manager.create_linked_session(
        &new_title,
        &id,
        &pinch_ctx,
        model_for_child,
        Some(working_dir_for_child.as_str()),
        source_session.target_branch.as_deref(),
    )?;

    // Inject the pinch context as first message in new session
    // Save as "system" message (matches TUI behavior) as proper JSON array
    let system_msg_text = pinch_ctx.to_system_message();
    let system_msg_json =
        serde_json::json!([{ "type": "text", "text": system_msg_text }]).to_string();
    session_manager.save_message(&new_session_id, "system", &system_msg_json)?;

    // Get the new session info
    let new_session = session_manager
        .get_session(&new_session_id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch new session".to_string()))?;

    Ok(Json(PinchResponse {
        session: new_session.into(),
        summary: summary_result.work_summary,
        key_decisions: summary_result.key_decisions,
        pending_tasks: summary_result.pending_tasks,
    }))
}

fn open_session_manager(state: &AppState) -> Result<SessionManager, AppError> {
    Ok(SessionManager::new(Database::new(&state.db_path)?))
}

fn ranked_files_for_pinch(session_manager: &SessionManager, session_id: &str) -> Vec<RankedFile> {
    match FileActivityTracker::new(session_manager.db(), session_id.to_string())
        .get_ranked_files(PINCH_RANKED_FILE_LIMIT)
    {
        Ok(ranked_files) => ranked_files,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "Failed to load ranked files for pinch context"
            );
            Vec::new()
        }
    }
}

fn load_project_context(working_dir: &StdPath) -> Option<String> {
    let context = build_project_context(working_dir);
    (!context.trim().is_empty()).then_some(context)
}

fn load_active_plan_markdown_for_pinch(state: &AppState, session_id: &str) -> Option<String> {
    match PlanManager::new((*state.db_path).clone()).and_then(|pm| pm.get_active_plan(session_id)) {
        Ok(Some(plan)) => Some(plan.to_markdown()),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "Failed to load active plan for pinch context"
            );
            None
        }
    }
}

fn load_key_file_contents(
    session_id: &str,
    working_dir: &StdPath,
    ranked_files: &[RankedFile],
    limit: usize,
) -> Vec<(String, String)> {
    ranked_files
        .iter()
        .take(limit)
        .filter_map(|file| {
            let path = if StdPath::new(&file.path).is_absolute() {
                PathBuf::from(&file.path)
            } else {
                working_dir.join(&file.path)
            };

            match std::fs::read_to_string(&path) {
                Ok(content) => Some((file.path.clone(), content)),
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        path = %path.display(),
                        error = %error,
                        "Failed to load key file content for pinch context"
                    );
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::extract::{Path, Query, State};
    use axum::Json;
    use chrono::Utc;
    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::plan::{PlanFile, PlanManager};
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::Database;
    use krusty_core::tools::registry::ToolRegistry;

    use super::*;
    use crate::auth::{AuthenticatedUser, CurrentUser};
    use crate::AppState;

    fn create_test_state() -> (AppState, PathBuf) {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-server-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("krusty.db");
        Database::new(&db_path).expect("database should initialize");
        let working_dir = temp_dir.join("workspace");
        std::fs::create_dir_all(&working_dir).expect("workspace should exist");

        (
            AppState {
                server_port: 3000,
                db_path: Arc::new(db_path),
                working_dir: Arc::new(working_dir.clone()),
                ai_client: None,
                tool_registry: Arc::new(ToolRegistry::new()),
                process_registry: Arc::new(ProcessRegistry::new()),
                model_registry: create_model_registry(),
                credential_store: Arc::new(RwLock::new(CredentialStore::default())),
                mcp_manager: Arc::new(McpManager::new(working_dir.clone())),
                hook_manager: Arc::new(RwLock::new(UserHookManager::new())),
                skills_manager: Arc::new(RwLock::new(SkillsManager::with_defaults(&working_dir))),
                cancellation: AgentCancellation::new(),
                session_locks: Arc::new(RwLock::new(HashMap::new())),
                session_inputs: Arc::new(RwLock::new(HashMap::new())),
                session_presence: Arc::new(RwLock::new(HashMap::new())),
                delegated_state: Arc::new(RwLock::new(HashMap::new())),
                remote_access: Arc::new(RwLock::new(crate::remote_access::RemoteAccessConfig {
                    enabled: true,
                    token: "test-token".to_string(),
                })),
                active_agent_streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                peak_rss_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                peak_virtual_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                push_service: None,
                apns_service: None,
                oauth_flows: Arc::new(Mutex::new(HashMap::new())),
                mako_runtime: crate::mako_runtime::MakoRuntimeManager::new(),
            },
            temp_dir,
        )
    }

    fn create_test_user(state: &AppState, user_id: &str) {
        let db = Database::new(&state.db_path).expect("database should open");
        db.conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
                (user_id, format!("{user_id}@example.com"), "free"),
            )
            .expect("user should insert");
    }

    fn current_user(user_id: &str, home_dir: &std::path::Path) -> CurrentUser {
        CurrentUser(AuthenticatedUser {
            user_id: Some(user_id.to_string()),
            home_dir: Some(home_dir.to_path_buf()),
        })
    }

    #[tokio::test]
    async fn create_session_persists_user_ownership() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let result = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Owned Session".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await;
        let (_, Json(response)) = match result {
            Ok(response) => response,
            Err(_) => panic!("session creation should succeed"),
        };

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session = session_manager
            .get_session(&response.id)
            .expect("session lookup should succeed")
            .expect("session should exist");

        assert_eq!(session.user_id.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn create_session_resolves_relative_workspace_paths_within_user_root() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        std::fs::create_dir_all(&user_root).expect("user root should exist");

        let (_, Json(created)) = create_session(
            State(state),
            Some(current_user("alice", &user_root)),
            Json(CreateSessionRequest {
                title: Some("Relative Workspace".to_string()),
                model: None,
                project_dir: Some("repo".to_string()),
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Selected),
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let expected = user_root.join("repo").to_string_lossy().to_string();
        assert_eq!(created.project_dir.as_deref(), Some(expected.as_str()));
        assert_eq!(created.working_dir.as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn get_session_rejects_foreign_owner() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        create_test_user(&state, "bob");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session_for_user("Owned Session", None, None, Some("alice"))
            .expect("session creation should succeed");
        session_manager
            .save_message(&session_id, "user", r#"[{"type":"text","text":"hello"}]"#)
            .expect("message should save");

        let result = get_session(
            State(state),
            Some(current_user("bob", std::path::Path::new("/tmp"))),
            Path(session_id),
            Query(GetSessionQuery {
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn load_owned_session_rejects_legacy_userless_session_for_authenticated_user() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session("Legacy Session", None, None)
            .expect("session creation should succeed");
        let user = current_user("alice", std::path::Path::new("/tmp"));

        let result = super::load_owned_session(&session_manager, &session_id, Some(&user));

        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_sessions_resolves_relative_working_dir_filter_within_user_root() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let repo_dir = user_root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo dir should exist");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        session_manager
            .create_session_for_user_with_config(
                "Scoped Session",
                None,
                Some(repo_dir.to_string_lossy().as_ref()),
                Some(repo_dir.to_string_lossy().as_ref()),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Code,
            )
            .expect("session creation should succeed");

        let Json(response) = list_sessions(
            State(state),
            Some(current_user("alice", &user_root)),
            Query(ListSessionsQuery {
                working_dir: Some("repo".to_string()),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session list should succeed"));

        assert_eq!(response.len(), 1);
        assert_eq!(
            response[0].working_dir.as_deref(),
            Some(repo_dir.to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn presence_heartbeat_tracks_active_controller_for_owned_session() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session_for_user("Owned Session", None, None, Some("alice"))
            .expect("session creation should succeed");

        let Json(response) = heartbeat_session_presence(
            State(state),
            Some(current_user("alice", std::path::Path::new("/tmp"))),
            Path(session_id),
            Json(SessionPresenceHeartbeatRequest {
                client_id: "client-1".to_string(),
                surface: "web".to_string(),
                capability: crate::presence::PresenceCapability::Controller,
                last_event_sequence: Some(12),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("presence heartbeat should succeed"));

        assert_eq!(response.active_viewers, 1);
        assert_eq!(response.active_controllers, 1);
        assert_eq!(response.clients.len(), 1);
        assert_eq!(response.clients[0].client_id, "client-1");
        assert_eq!(response.clients[0].last_event_sequence, Some(12));
    }

    #[tokio::test]
    async fn pinch_session_includes_project_context_and_ranked_files() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let workspace = state.working_dir.as_ref();
        let source_file = workspace.join("src/lib.rs");
        std::fs::create_dir_all(source_file.parent().expect("parent dir should exist"))
            .expect("src dir should exist");
        std::fs::write(
            workspace.join("AGENTS.md"),
            "# Workspace Rules\nPreserve session context.\n",
        )
        .expect("project instructions should write");
        std::fs::write(
            &source_file,
            "pub fn important() -> &'static str { \"hello\" }\n",
        )
        .expect("source file should write");

        let session_manager = match open_session_manager(&state) {
            Ok(session_manager) => session_manager,
            Err(_) => panic!("session manager should open"),
        };
        let session_id = session_manager
            .create_session_for_user(
                "Pinch Source",
                Some("claude-3-5-sonnet"),
                Some(workspace.to_string_lossy().as_ref()),
                Some("alice"),
            )
            .expect("session should create");

        let user_message =
            serde_json::json!([{ "type": "text", "text": "Continue refining the server pinch flow." }])
                .to_string();
        let assistant_message =
            serde_json::json!([{ "type": "text", "text": "I inspected the route and found missing continuation context." }])
                .to_string();
        session_manager
            .save_message(&session_id, "user", &user_message)
            .expect("user message should save");
        session_manager
            .save_message(&session_id, "assistant", &assistant_message)
            .expect("assistant message should save");

        let plan_manager =
            PlanManager::new((*state.db_path).clone()).expect("plan manager should open");
        let plan = PlanFile::from_markdown(
            r#"# Plan: Server Pinch Follow-up

Created: 2026-04-06 12:00 UTC
Session: placeholder
Working Directory: placeholder
Status: in_progress

---

## Phase 1: Continuation

- [ ] Task 1.1: Keep session continuity
"#,
        )
        .expect("plan should parse");
        plan_manager
            .save_plan_for_session(&session_id, &plan)
            .expect("plan should save");

        session_manager
            .db()
            .conn()
            .execute(
                "INSERT INTO file_activity (session_id, file_path, read_count, write_count, edit_count, last_accessed, user_referenced)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    &session_id,
                    "src/lib.rs",
                    2_i64,
                    1_i64,
                    0_i64,
                    Utc::now().to_rfc3339(),
                    1_i64,
                ),
            )
            .expect("file activity should insert");

        let Json(response) = pinch_session(
            State(state.clone()),
            Some(current_user("alice", workspace)),
            Path(session_id.clone()),
            Json(PinchRequest {
                preservation_hints: Some("Keep the route semantics intact.".to_string()),
                direction: Some("Continue the server audit.".to_string()),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("pinch should succeed"));

        let messages = session_manager
            .load_session_messages(&response.session.id)
            .expect("child messages should load");
        let (role, system_message_json) = messages.first().expect("system message should exist");

        assert_eq!(role, "system");
        assert!(system_message_json.contains("Project Instructions"));
        assert!(system_message_json.contains("[PROJECT INSTRUCTIONS -"));
        assert!(system_message_json.contains("Key Files (by importance)"));
        assert!(system_message_json.contains("src/lib.rs"));
        assert!(system_message_json.contains("Key File Contents (Pre-loaded)"));
        assert!(system_message_json.contains("pub fn important()"));
        assert!(system_message_json.contains("## Active Plan"));
        assert!(system_message_json.contains("Task 1.1: Keep session continuity"));
    }

    #[tokio::test]
    async fn pinch_session_resolves_legacy_relative_working_dir_against_user_home() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let repo_dir = user_root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo dir should exist");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session_for_user_with_config(
                "Legacy Relative Session",
                None,
                Some("repo"),
                Some("repo"),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Code,
            )
            .expect("session should create");
        let user_message =
            serde_json::json!([{ "type": "text", "text": "Continue from the last session." }])
                .to_string();
        let assistant_message =
            serde_json::json!([{ "type": "text", "text": "I will resume the work." }]).to_string();
        session_manager
            .save_message(&session_id, "user", &user_message)
            .expect("user message should save");
        session_manager
            .save_message(&session_id, "assistant", &assistant_message)
            .expect("assistant message should save");

        let Json(response) = pinch_session(
            State(state),
            Some(current_user("alice", &user_root)),
            Path(session_id),
            Json(PinchRequest {
                preservation_hints: None,
                direction: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("pinch should succeed"));

        let expected = repo_dir.to_string_lossy().to_string();
        assert_eq!(
            response.session.working_dir.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            response.session.project_dir.as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn live_partial_assistant_only_surfaces_for_active_states() {
        let recovery = krusty_core::storage::SessionRecoveryState::new(
            krusty_core::storage::RecoveryStatus::Streaming,
            None,
            None,
            krusty_core::storage::PartialAssistantState {
                text: "partial".to_string(),
                thinking: "reasoning".to_string(),
                tool_calls: Vec::new(),
            },
            krusty_core::storage::RecoveryDecision::Resumable {
                latest_user_objective: "finish task".to_string(),
            },
        );

        assert!(super::live_partial_assistant_for_state("idle", Some(&recovery)).is_none());

        let live = super::live_partial_assistant_for_state("streaming", Some(&recovery))
            .expect("active state should surface live partial");
        assert_eq!(live.text, "partial");
        assert_eq!(live.thinking, "reasoning");
    }

    #[tokio::test]
    async fn session_routes_normalize_blank_model_input_to_none() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Whitespace Model".to_string()),
                model: Some("   ".to_string()),
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        assert_eq!(created.model, None);

        let Json(updated) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id.clone()),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                mode: None,
                model: Some("  gpt-5  ".to_string()),
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session update should succeed"));

        assert_eq!(updated.model.as_deref(), Some("gpt-5"));

        let Json(cleared) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                mode: None,
                model: Some("   ".to_string()),
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session update should succeed"));

        assert_eq!(cleared.model, None);
    }

    #[tokio::test]
    async fn session_routes_apply_workspace_updates() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let project_dir = state.working_dir.join("demo-app");
        std::fs::create_dir_all(&project_dir).expect("project dir should exist");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Workspace Update".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let Json(updated) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id.clone()),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: Some(project_dir.to_string_lossy().to_string()),
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Created),
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("workspace update should succeed"));

        assert_eq!(
            updated.project_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            updated.working_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );
        assert_eq!(updated.workspace_mode, WorkspaceMode::Created);

        let Json(neutral) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Neutral),
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("neutral workspace update should succeed"));

        assert_eq!(neutral.project_dir, None);
        assert_eq!(neutral.working_dir, None);
        assert_eq!(neutral.workspace_mode, WorkspaceMode::Neutral);
    }

    #[tokio::test]
    async fn session_routes_reject_invalid_workspace_payloads() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Workspace Validation".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let result = update_session(
            State(state),
            Some(current_user("alice", std::path::Path::new("/tmp"))),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Created),
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await;

        match result {
            Err(AppError::BadRequest(message)) => {
                assert_eq!(
                    message,
                    "workspace modes 'selected' and 'created' require a project_dir"
                );
            }
            Ok(_) => panic!("invalid workspace update should fail"),
            Err(_) => panic!("invalid workspace update should fail with bad request"),
        }
    }

    #[tokio::test]
    async fn session_routes_reject_working_dir_updates_outside_user_root() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let outside_root = temp_dir.join("outside");
        std::fs::create_dir_all(&user_root).expect("user root should exist");
        std::fs::create_dir_all(&outside_root).expect("outside root should exist");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", &user_root)),
            Json(CreateSessionRequest {
                title: Some("Workspace Validation".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let result = update_session(
            State(state),
            Some(current_user("alice", &user_root)),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: Some(outside_root.to_string_lossy().to_string()),
                workspace_mode: None,
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
