use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use krusty_core::agent::learning::{
    GovernedLearningReviewResult, GovernedLearningReviewService, LearningReviewServiceError,
};
use krusty_core::storage::{Database, LearningCandidate, LearningCandidateStatus};

use super::super::session_access::current_user_id;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
pub(super) struct LearningCandidateListQuery {
    status: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(super) struct LearningCandidateListResponse {
    candidates: Vec<LearningCandidate>,
    status: String,
    limit: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct LearningCandidateReviewResponse {
    candidate: LearningCandidate,
    memory_id: Option<String>,
    replayed: bool,
}

pub(super) async fn list_candidates(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<LearningCandidateListQuery>,
) -> Result<Json<LearningCandidateListResponse>, AppError> {
    let (status, status_label) = parse_status(query.status.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let service = open_service(&state)?;
    let candidates = service
        .list_candidates(current_user_id(user.as_ref()), status, limit)
        .map_err(map_review_error)?;
    Ok(Json(LearningCandidateListResponse {
        candidates,
        status: status_label,
        limit,
    }))
}

pub(super) async fn accept_candidate(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<LearningCandidateReviewResponse>, AppError> {
    let result = open_service(&state)?
        .accept_pending(&id, current_user_id(user.as_ref()))
        .map_err(map_review_error)?;
    Ok(Json(result.into()))
}

pub(super) async fn reject_candidate(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<LearningCandidateReviewResponse>, AppError> {
    let result = open_service(&state)?
        .reject_pending(&id, current_user_id(user.as_ref()))
        .map_err(map_review_error)?;
    Ok(Json(result.into()))
}

fn open_service(state: &AppState) -> Result<GovernedLearningReviewService, AppError> {
    Ok(GovernedLearningReviewService::new(Database::new(
        &state.db_path,
    )?))
}

fn parse_status(raw: Option<&str>) -> Result<(Option<LearningCandidateStatus>, String), AppError> {
    let status = raw.unwrap_or("pending").trim();
    if status == "all" {
        return Ok((None, "all".to_string()));
    }
    let parsed = status.parse::<LearningCandidateStatus>().map_err(|_| {
        AppError::BadRequest(
            "status must be pending, accepted, auto_accepted, rejected, tombstoned, or all"
                .to_string(),
        )
    })?;
    Ok((Some(parsed), parsed.to_string()))
}

fn map_review_error(error: LearningReviewServiceError) -> AppError {
    match error {
        LearningReviewServiceError::NotFound => {
            AppError::NotFound("learning candidate not found".to_string())
        }
        LearningReviewServiceError::Conflict { status } => {
            AppError::Conflict(format!("learning candidate is already {status}"))
        }
        LearningReviewServiceError::Policy(reason) => {
            AppError::BadRequest(format!("learning candidate cannot be promoted: {reason}"))
        }
        LearningReviewServiceError::InvalidEvidence(reason) => {
            AppError::BadRequest(format!("learning candidate evidence is invalid: {reason}"))
        }
        LearningReviewServiceError::Storage(source) => {
            tracing::error!(error = ?source, "Governed Mako learning review failed");
            AppError::Internal("learning candidate review failed".to_string())
        }
    }
}

impl From<GovernedLearningReviewResult> for LearningCandidateReviewResponse {
    fn from(result: GovernedLearningReviewResult) -> Self {
        Self {
            candidate: result.candidate,
            memory_id: result.memory_id,
            replayed: result.replayed,
        }
    }
}

#[cfg(test)]
mod tests {
    use krusty_core::storage::LearningCandidateStatus;

    use super::{parse_status, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT};

    #[test]
    fn list_status_is_closed_and_defaults_pending() {
        let (default, label) = match parse_status(None) {
            Ok(value) => value,
            Err(_) => panic!("default status should parse"),
        };
        assert_eq!(default, Some(LearningCandidateStatus::Pending));
        assert_eq!(label, "pending");
        assert!(matches!(parse_status(Some("all")), Ok((None, _))));
        assert!(parse_status(Some("unknown")).is_err());
        assert!(DEFAULT_LIST_LIMIT <= MAX_LIST_LIMIT);
    }
}
