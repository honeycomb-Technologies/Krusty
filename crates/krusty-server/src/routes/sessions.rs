//! Session management endpoints

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use krusty_core::agent::pinch_context::{PinchContext, PinchContextInput};
use krusty_core::agent::summarizer::{generate_summary, SummarizationResult};
use krusty_core::ai::types::{Content, ModelMessage, Role};
use krusty_core::{storage::Database, SessionManager};

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{
    CreateSessionRequest, MessageResponse, PinchRequest, PinchResponse, SessionResponse,
    SessionStateResponse, SessionWithMessagesResponse, UpdateSessionRequest,
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
        .route("/:id/pinch", post(pinch_session))
}

/// List all sessions, optionally filtered by working directory
async fn list_sessions(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionResponse>>, AppError> {
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let sessions = session_manager.list_sessions_for_user(query.working_dir.as_deref(), user_id)?;
    let response: Vec<SessionResponse> = sessions.into_iter().map(Into::into).collect();

    Ok(Json(response))
}

/// List all directories that have sessions
async fn list_directories(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<Vec<String>>, AppError> {
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let directories = session_manager.list_session_directories_for_user(user_id)?;

    Ok(Json(directories))
}

/// Create a new session
async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), AppError> {
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    let title = req.title.as_deref().unwrap_or("New Session");
    let target_branch = req.target_branch.as_deref().map(str::trim).and_then(|b| {
        if b.is_empty() {
            None
        } else {
            Some(b)
        }
    });
    let session_id = session_manager.create_session_with_target_branch(
        title,
        req.model.as_deref(),
        req.working_dir.as_deref(),
        target_branch,
    )?;

    let session = session_manager
        .get_session(&session_id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch created session".to_string()))?;

    Ok((StatusCode::CREATED, Json(session.into())))
}

/// Get a session with its messages, with optional pagination
async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<GetSessionQuery>,
) -> Result<Json<SessionWithMessagesResponse>, AppError> {
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    let session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", id)))?;

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
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    // Verify session exists
    let _session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", id)))?;

    // Verify ownership in multi-tenant mode
    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    if !session_manager.verify_session_ownership(&id, user_id)? {
        return Err(AppError::NotFound(format!("Session {} not found", id)));
    }

    if req.title.is_none()
        && req.working_dir.is_none()
        && req.mode.is_none()
        && req.model.is_none()
        && req.target_branch.is_none()
    {
        return Err(AppError::BadRequest(
            "At least one of title, working_dir, mode, model, or target_branch must be provided"
                .to_string(),
        ));
    }

    if let Some(title) = req.title.as_deref() {
        session_manager.update_session_title(&id, title)?;
    }

    if let Some(working_dir) = req.working_dir.as_deref() {
        let trimmed = working_dir.trim();
        let normalized = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        session_manager.update_session_working_dir(&id, normalized)?;
    }

    if let Some(mode) = req.mode {
        session_manager.update_session_work_mode(&id, mode)?;
    }

    if let Some(model) = req.model.as_deref() {
        let normalized = if model.is_empty() { None } else { Some(model) };
        session_manager.update_session_model(&id, normalized)?;
    }

    if let Some(target_branch) = req.target_branch.as_deref() {
        let normalized = target_branch.trim();
        let normalized = if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        };
        session_manager.update_session_target_branch(&id, normalized)?;
    }

    let session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch updated session".to_string()))?;

    Ok(Json(session.into()))
}

/// Delete a session
async fn delete_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    // Verify session exists
    let _session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", id)))?;

    // Verify ownership in multi-tenant mode
    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    if !session_manager.verify_session_ownership(&id, user_id)? {
        return Err(AppError::NotFound(format!("Session {} not found", id)));
    }

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
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    // Verify session exists
    let session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", id)))?;

    // Verify ownership in multi-tenant mode
    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    if !session_manager.verify_session_ownership(&id, user_id)? {
        return Err(AppError::NotFound(format!("Session {} not found", id)));
    }

    // Get agent state
    let agent_state =
        session_manager
            .get_agent_state(&id)
            .unwrap_or_else(|| krusty_core::storage::AgentState {
                state: "idle".to_string(),
                started_at: None,
                last_event_at: None,
            });

    Ok(Json(SessionStateResponse {
        id,
        agent_state: agent_state.state,
        started_at: agent_state.started_at,
        last_event_at: agent_state.last_event_at,
        mode: session.work_mode,
    }))
}

/// Pinch a session - create a child session with summarized context
async fn pinch_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PinchRequest>,
) -> Result<Json<PinchResponse>, AppError> {
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    // Get source session
    let source_session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", id)))?;

    // Load messages and convert to ModelMessage format
    let raw_messages = session_manager.load_session_messages(&id)?;
    let messages: Vec<ModelMessage> = raw_messages
        .into_iter()
        .filter_map(|(role, content_json)| {
            let role = match role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            let content: Vec<Content> = serde_json::from_str(&content_json).ok()?;
            Some(ModelMessage { role, content })
        })
        .collect();

    if messages.is_empty() {
        return Err(AppError::BadRequest(
            "Cannot pinch session with no messages".to_string(),
        ));
    }

    // Generate summary using AI if configured, otherwise use defaults.
    let summary_model = source_session.model.as_deref();
    let summary_result = if let Some(ai_client) = state.resolve_ai_client(summary_model).await {
        generate_summary(
            &ai_client,
            &messages,
            req.preservation_hints.as_deref(),
            &[],  // ranked files
            &[],  // file contents
            None, // CLAUDE.md
            None, // project context
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
    let pinch_ctx = PinchContext::from_input(PinchContextInput {
        source_session_id: id.clone(),
        source_session_title: source_session.title.clone(),
        summary: summary_result.clone(),
        ranked_files: vec![], // No ranked files for now
        preservation_hints: req.preservation_hints,
        direction: req.direction,
        project_context: None,     // No project context for now
        key_file_contents: vec![], // No key file contents for now
        active_plan: None,         // No active plan for now
    });

    // Create the child session
    let new_title = format!("{} (continued)", source_session.title);
    let default_working_dir = state.working_dir.to_string_lossy().to_string();
    let working_dir_for_child = source_session
        .working_dir
        .as_deref()
        .unwrap_or(default_working_dir.as_str());
    let model_for_child = source_session.model.as_deref();
    let new_session_id = session_manager.create_linked_session(
        &new_title,
        &id,
        &pinch_ctx,
        model_for_child,
        Some(working_dir_for_child),
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
