use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use mitsuro_core::ai::models::ModelKey;
use mitsuro_core::hive::{DstPolicy, MisfireConfig, RecurrenceV1, RetryPolicy};
use mitsuro_core::storage::{
    Database, HiveController, HiveControllerEvent, HiveControllerEventStore, HiveControllerStore,
    HiveRun, HiveRunAttempt, HiveRunStore, HiveSchedule, HiveScheduleOccurrence, HiveScheduleStore,
    OverlapPolicy, OwnedHiveSchedule, SessionType,
};
use mitsuro_hive_protocol::ScheduleDefinition;

use super::super::session_access::{
    current_user_id, load_owned_hive_session_control_scope, load_owned_session_of_type,
};
use super::{idempotency_key_from_headers, open_session_manager, resolve_hive_model};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::resolve_optional_workspace_path;
use crate::AppState;

const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 200;

#[derive(Debug, Deserialize)]
pub(super) struct ScheduleWriteRequest {
    title: String,
    #[serde(default)]
    summary: String,
    objective: String,
    recurrence: RecurrenceV1,
    timezone: String,
    #[serde(default)]
    dst_policy: DstPolicy,
    #[serde(default)]
    priority: i32,
    project_dir: Option<String>,
    model: Option<String>,
    #[serde(default)]
    model_key: Option<ModelKey>,
    crew_slug: Option<String>,
    #[serde(default)]
    worker_id: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    misfire: MisfireConfig,
    #[serde(default = "default_overlap_policy")]
    overlap_policy: OverlapPolicy,
    #[serde(default)]
    retry: RetryPolicy,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListQuery {
    limit: Option<usize>,
    after_sequence: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ScheduleMutationResponse {
    schedule_id: String,
    revision: u64,
    status: String,
}

pub(super) async fn list_schedules(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<HiveSchedule>>, AppError> {
    let Some(controller) = owned_controller(&state, user.as_ref(), &session_id)? else {
        return Ok(Json(Vec::new()));
    };
    let store = HiveScheduleStore::new(Database::new(&state.db_path)?);
    Ok(Json(store.list_for_controller(
        &controller.id,
        list_limit(query.limit),
    )?))
}

/// User-scoped global schedule list for the Hive Schedule secondary surface.
///
/// Returns commitments across all of the caller's controllers, ordered by next
/// fire time so the UI can show "what's set" without browsing individual runs.
pub(super) async fn list_global_schedules(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<OwnedHiveSchedule>>, AppError> {
    let store = HiveScheduleStore::new(Database::new(&state.db_path)?);
    Ok(Json(store.list_for_user(
        current_user_id(user.as_ref()),
        list_limit(query.limit),
    )?))
}

pub(super) async fn create_schedule(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ScheduleWriteRequest>,
) -> Result<Response, AppError> {
    owned_hive_control_scope(&state, user.as_ref(), &session_id)?
        .require_exact_worker_schedule(request.worker_id.as_deref())?;
    let definition = schedule_definition(&state, user.as_ref(), &session_id, request).await?;
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    let result = state
        .hive_runtime
        .create_schedule_for_user(
            current_user_id(user.as_ref()),
            &session_id,
            definition,
            idempotency_key.as_deref(),
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    schedule_response(StatusCode::CREATED, result)
}

pub(super) async fn get_schedule(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, schedule_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let controller = require_owned_controller(&state, user.as_ref(), &session_id)?;
    let schedule = owned_schedule(&state, &controller, &schedule_id)?;
    let mut response = Json(schedule.clone()).into_response();
    insert_etag(response.headers_mut(), schedule.revision)?;
    Ok(response)
}

pub(super) async fn replace_schedule(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, schedule_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ScheduleWriteRequest>,
) -> Result<Response, AppError> {
    owned_hive_control_scope(&state, user.as_ref(), &session_id)?
        .require_exact_worker_schedule(request.worker_id.as_deref())?;
    let expected_revision = required_if_match(&headers)?;
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    let definition = schedule_definition(&state, user.as_ref(), &session_id, request).await?;
    let result = state
        .hive_runtime
        .replace_schedule_for_user(
            current_user_id(user.as_ref()),
            &session_id,
            &schedule_id,
            expected_revision,
            definition,
            idempotency_key.as_deref(),
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    schedule_response(StatusCode::OK, result)
}

pub(super) async fn cancel_schedule(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, schedule_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let controller = require_owned_controller(&state, user.as_ref(), &session_id)?;
    let schedule = owned_schedule(&state, &controller, &schedule_id)?;
    owned_hive_control_scope(&state, user.as_ref(), &session_id)?
        .require_exact_worker_schedule(schedule.worker_id.as_deref())?;
    let expected_revision = required_if_match(&headers)?;
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    let result = state
        .hive_runtime
        .set_schedule_status_for_user(
            current_user_id(user.as_ref()),
            &session_id,
            &schedule_id,
            expected_revision,
            "cancelled",
            idempotency_key.as_deref(),
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    schedule_response(StatusCode::OK, result)
}

pub(super) async fn pause_schedule(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, schedule_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    set_schedule_status(state, user, session_id, schedule_id, headers, "paused").await
}

pub(super) async fn resume_schedule(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, schedule_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    set_schedule_status(state, user, session_id, schedule_id, headers, "enabled").await
}

async fn set_schedule_status(
    state: AppState,
    user: Option<CurrentUser>,
    session_id: String,
    schedule_id: String,
    headers: HeaderMap,
    status: &'static str,
) -> Result<Response, AppError> {
    let controller = require_owned_controller(&state, user.as_ref(), &session_id)?;
    let schedule = owned_schedule(&state, &controller, &schedule_id)?;
    owned_hive_control_scope(&state, user.as_ref(), &session_id)?
        .require_exact_worker_schedule(schedule.worker_id.as_deref())?;
    let expected_revision = required_if_match(&headers)?;
    let idempotency_key = idempotency_key_from_headers(&headers)?;
    let result = state
        .hive_runtime
        .set_schedule_status_for_user(
            current_user_id(user.as_ref()),
            &session_id,
            &schedule_id,
            expected_revision,
            status,
            idempotency_key.as_deref(),
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    schedule_response(StatusCode::OK, result)
}

pub(super) async fn list_occurrences(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, schedule_id)): Path<(String, String)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<HiveScheduleOccurrence>>, AppError> {
    let controller = require_owned_controller(&state, user.as_ref(), &session_id)?;
    owned_schedule(&state, &controller, &schedule_id)?;
    let store = HiveScheduleStore::new(Database::new(&state.db_path)?);
    Ok(Json(
        store.list_occurrences(&schedule_id, list_limit(query.limit))?,
    ))
}

pub(super) async fn list_runs(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<HiveRun>>, AppError> {
    let Some(controller) = owned_controller(&state, user.as_ref(), &session_id)? else {
        return Ok(Json(Vec::new()));
    };
    let store = HiveRunStore::new(Database::new(&state.db_path)?);
    Ok(Json(store.list_for_controller(
        &controller.id,
        list_limit(query.limit),
    )?))
}

pub(super) async fn get_run(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, run_id)): Path<(String, String)>,
) -> Result<Json<HiveRun>, AppError> {
    let controller = require_owned_controller(&state, user.as_ref(), &session_id)?;
    let run = owned_run(&state, &controller, &run_id)?;
    Ok(Json(run))
}

pub(super) async fn list_attempts(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((session_id, run_id)): Path<(String, String)>,
) -> Result<Json<Vec<HiveRunAttempt>>, AppError> {
    let controller = require_owned_controller(&state, user.as_ref(), &session_id)?;
    owned_run(&state, &controller, &run_id)?;
    let store = HiveRunStore::new(Database::new(&state.db_path)?);
    Ok(Json(store.list_attempts(&run_id)?))
}

pub(super) async fn list_event_log(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<HiveControllerEvent>>, AppError> {
    let Some(controller) = owned_controller(&state, user.as_ref(), &session_id)? else {
        return Ok(Json(Vec::new()));
    };
    let store = HiveControllerEventStore::new(Database::new(&state.db_path)?);
    Ok(Json(store.list_after(
        &controller.id,
        query.after_sequence.unwrap_or(0),
        list_limit(query.limit),
    )?))
}

fn ensure_owned_hive_session(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
) -> Result<(), AppError> {
    load_owned_session_of_type(
        &open_session_manager(state)?,
        session_id,
        SessionType::Hive,
        "Hive",
        user,
    )?;
    Ok(())
}

fn owned_hive_control_scope(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
) -> Result<super::super::session_access::HiveSessionControlScope, AppError> {
    load_owned_hive_session_control_scope(state, &open_session_manager(state)?, session_id, user)
}

fn owned_controller(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
) -> Result<Option<HiveController>, AppError> {
    ensure_owned_hive_session(state, user, session_id)?;
    Ok(HiveControllerStore::new(Database::new(&state.db_path)?).get_by_session(session_id)?)
}

fn require_owned_controller(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
) -> Result<HiveController, AppError> {
    owned_controller(state, user, session_id)?.ok_or_else(|| {
        AppError::NotFound(format!(
            "Hive controller for session {session_id} not found"
        ))
    })
}

fn owned_schedule(
    state: &AppState,
    controller: &HiveController,
    schedule_id: &str,
) -> Result<HiveSchedule, AppError> {
    let schedule = HiveScheduleStore::new(Database::new(&state.db_path)?)
        .get_schedule(schedule_id)?
        .ok_or_else(|| AppError::NotFound(format!("Schedule {schedule_id} not found")))?;
    if schedule.controller_id != controller.id {
        return Err(AppError::NotFound(format!(
            "Schedule {schedule_id} not found"
        )));
    }
    Ok(schedule)
}

fn owned_run(
    state: &AppState,
    controller: &HiveController,
    run_id: &str,
) -> Result<HiveRun, AppError> {
    let run = HiveRunStore::new(Database::new(&state.db_path)?)
        .get_run(run_id)?
        .ok_or_else(|| AppError::NotFound(format!("Hive run {run_id} not found")))?;
    if run.controller_id != controller.id {
        return Err(AppError::NotFound(format!("Hive run {run_id} not found")));
    }
    Ok(run)
}

async fn schedule_definition(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
    request: ScheduleWriteRequest,
) -> Result<ScheduleDefinition, AppError> {
    let workspace_scope = super::super::session_access::request_workspace_scope(state, user);
    let project_dir = resolve_optional_workspace_path(
        request.project_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    let recurrence = serde_json::to_value(request.recurrence)
        .map_err(|error| AppError::BadRequest(format!("Invalid recurrence: {error}")))?;
    let dst_policy = serde_json::to_value(request.dst_policy)
        .map_err(|error| AppError::BadRequest(format!("Invalid DST policy: {error}")))?;
    let misfire = serde_json::to_value(request.misfire)
        .map_err(|error| AppError::BadRequest(format!("Invalid misfire policy: {error}")))?;
    let retry = serde_json::to_value(request.retry)
        .map_err(|error| AppError::BadRequest(format!("Invalid retry policy: {error}")))?;
    let session_manager = open_session_manager(state)?;
    let session = load_owned_session_of_type(
        &session_manager,
        session_id,
        SessionType::Hive,
        "Hive",
        user,
    )?;
    let explicit_model = trimmed_nonempty(request.model.as_deref()).map(ToOwned::to_owned);
    let inherit_session_model = explicit_model.is_none() && request.model_key.is_none();
    let requested_model = explicit_model.or_else(|| {
        inherit_session_model
            .then(|| trimmed_nonempty(session.model.as_deref()).map(ToOwned::to_owned))
            .flatten()
    });
    let requested_key = request.model_key.as_ref().or_else(|| {
        inherit_session_model
            .then_some(session.model_key.as_ref())
            .flatten()
    });
    if requested_model.is_none() && requested_key.is_none() {
        return Err(AppError::Conflict(
            "Hive session has no frozen model; start a new Hive session or provide an explicit schedule model".into(),
        ));
    }
    let resolved_model = resolve_hive_model(
        state,
        current_user_id(user),
        requested_model.as_deref(),
        requested_key,
    )
    .await?;
    let protocol_model_key = resolved_model.protocol_key()?;

    Ok(ScheduleDefinition {
        title: request.title,
        summary: request.summary,
        objective: request.objective,
        recurrence,
        timezone: request.timezone,
        dst_policy,
        priority: request.priority,
        project_dir,
        model: Some(resolved_model.model),
        model_key: Some(protocol_model_key),
        model_catalog_revision: resolved_model.catalog_revision,
        crew_slug: trimmed_nonempty(request.crew_slug.as_deref()).map(ToOwned::to_owned),
        worker_id: trimmed_nonempty(request.worker_id.as_deref()).map(ToOwned::to_owned),
        group_id: trimmed_nonempty(request.group_id.as_deref()).map(ToOwned::to_owned),
        misfire,
        overlap_policy: request.overlap_policy.as_str().to_string(),
        retry,
    })
}

fn default_overlap_policy() -> OverlapPolicy {
    OverlapPolicy::QueueOne
}

fn list_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT)
}

fn required_if_match(headers: &HeaderMap) -> Result<u64, AppError> {
    let raw = headers
        .get(header::IF_MATCH)
        .ok_or_else(|| AppError::BadRequest("If-Match is required".into()))?
        .to_str()
        .map_err(|_| AppError::BadRequest("If-Match is not valid ASCII".into()))?;
    if raw == "*" || raw.starts_with("W/") || raw.len() < 3 {
        return Err(AppError::BadRequest(
            "If-Match must be a strong quoted schedule revision, for example \"3\"".into(),
        ));
    }
    let revision = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| {
            AppError::BadRequest(
                "If-Match must be a strong quoted schedule revision, for example \"3\"".into(),
            )
        })?;
    revision
        .parse::<u64>()
        .map_err(|_| AppError::BadRequest("If-Match contains an invalid revision".into()))
}

fn insert_etag(headers: &mut HeaderMap, revision: u64) -> Result<(), AppError> {
    let value = HeaderValue::from_str(&format!("\"{revision}\""))
        .map_err(|error| AppError::Internal(format!("Failed to encode schedule ETag: {error}")))?;
    headers.insert(header::ETAG, value);
    Ok(())
}

fn schedule_response(
    status: StatusCode,
    result: mitsuro_hive_protocol::ScheduleResponse,
) -> Result<Response, AppError> {
    let mut response = (
        status,
        Json(ScheduleMutationResponse {
            schedule_id: result.schedule_id,
            revision: result.revision,
            status: result.status,
        }),
    )
        .into_response();
    insert_etag(response.headers_mut(), result.revision)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_match_requires_strong_quoted_numeric_revision() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("\"7\""));
        assert!(matches!(required_if_match(&headers), Ok(7)));

        headers.insert(header::IF_MATCH, HeaderValue::from_static("W/\"7\""));
        assert!(required_if_match(&headers).is_err());
        headers.insert(header::IF_MATCH, HeaderValue::from_static("*"));
        assert!(required_if_match(&headers).is_err());
    }

    #[test]
    fn list_limit_is_bounded() {
        assert_eq!(list_limit(None), DEFAULT_LIST_LIMIT);
        assert_eq!(list_limit(Some(usize::MAX)), MAX_LIST_LIMIT);
    }
}
