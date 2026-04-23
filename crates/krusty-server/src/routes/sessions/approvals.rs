use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::routes::chat::submit_tool_approval;
use crate::types::ToolApprovalRequest;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct SessionToolApprovalRequest {
    tool_call_id: String,
    approved: bool,
}

pub(super) async fn tool_approval_for_session(
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
