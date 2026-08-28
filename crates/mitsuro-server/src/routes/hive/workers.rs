//! Hive Worker CRUD and DM-lane endpoints.
//!
//! Workers are first-class durable identities. These routes keep ownership
//! exact-owner scoped (NULL owner = local profile, never a wildcard), freeze
//! model identity through the shared Hive model resolver, and treat delete as
//! archive so a Worker's history and documents are never destroyed.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};

use mitsuro_core::ai::models::ModelKey as CoreModelKey;
#[cfg(test)]
use mitsuro_core::storage::NewHiveWorker;
use mitsuro_core::storage::{
    display_name_from_slug, is_valid_crew_slug, Database, HiveDelivery, HiveDeliveryStatus,
    HiveDeliveryStore, HiveWorker, HiveWorkerAutonomy, HiveWorkerDocumentKind,
    HiveWorkerIntroductionStore, HiveWorkerStatus, HiveWorkerStore, SessionType,
    WorkerIntroductionProposalV1, WorkerIntroductionReviewProjection, WorkspaceMode,
    MAX_HIVE_PROFILE_DOCUMENT_BYTES, MAX_WORKER_INTRODUCTION_FACTS,
    WORKER_INTRODUCTION_PROPOSAL_VERSION,
};
use mitsuro_core::tools::registry::PermissionMode;
use mitsuro_core::SessionManager;

use super::super::session_access::{
    current_user_id, load_agent_state_or_idle, load_owned_session, session_visible_to_user,
};
use super::{
    idempotency_key_from_headers, open_session_manager, resolve_hive_model, ResolvedHiveModel,
};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::text::trimmed_nonempty;
use crate::AppState;

const CANONICAL_HIVE_SESSION_TYPE: &str = "hive";

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkerSummary {
    pub(super) id: String,
    pub(super) revision: u64,
    pub(super) slug: String,
    pub(super) display_name: String,
    pub(super) avatar_color: Option<String>,
    pub(super) model: Option<String>,
    pub(super) model_key: Option<CoreModelKey>,
    pub(super) permission_mode: String,
    pub(super) autonomy: String,
    pub(super) heartbeat_interval_secs: Option<u32>,
    pub(super) status: String,
    pub(super) dm_session_id: Option<String>,
    /// Agent state of the DM session when one is bound ("idle", "running",
    /// ...), the cheap liveness enrichment for list rows.
    pub(super) dm_agent_state: Option<String>,
    /// Cheap lifecycle projection for roster recovery controls.
    pub(super) introduction_status: Option<String>,
    pub(super) introduction_last_error: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkersResponse {
    pub(super) workers: Vec<HiveWorkerSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkerDetailResponse {
    #[serde(flatten)]
    pub(super) worker: HiveWorkerSummary,
    pub(super) identity: Option<String>,
    pub(super) soul: Option<String>,
    pub(super) introduction: Option<HiveWorkerIntroductionResponse>,
    pub(super) attention: Vec<mitsuro_hive_protocol::WorkerLaneAttention>,
}

/// Exact classification of one user-addressable Hive conversation. A Worker
/// DM carries its complete durable Worker projection, including archived
/// Workers that the active roster intentionally omits.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum HiveWorkerSessionBindingResponse {
    WorkerDm {
        session_id: String,
        worker: Box<HiveWorkerDetailResponse>,
    },
    PrimaryHive {
        session_id: String,
    },
}

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkerIntroductionResponse {
    pub(super) run_id: Option<String>,
    pub(super) status: String,
    pub(super) prompt_version: u32,
    pub(super) opening_message_id: Option<i64>,
    pub(super) proposal: Option<WorkerIntroductionProposalV1>,
    pub(super) proposal_revision: u32,
    pub(super) review_projection: WorkerIntroductionReviewProjection,
    pub(super) last_error: Option<String>,
    pub(super) completed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfirmWorkerIntroductionRequest {
    pub(super) proposal_id: String,
    pub(super) proposal_revision: u32,
    pub(super) selected_facts: Vec<mitsuro_hive_protocol::WorkerIntroductionSelectedFact>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KeepTalkingWorkerIntroductionRequest {
    pub(super) proposal_id: String,
    pub(super) proposal_revision: u32,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListWorkerDeliveriesQuery {
    pub(super) status: Option<String>,
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkerDelivery {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) from_worker_id: Option<String>,
    pub(super) to_worker_id: String,
    pub(super) group_id: Option<String>,
    pub(super) body: String,
    pub(super) priority: String,
    pub(super) status: String,
    pub(super) attempt_count: u32,
    pub(super) max_attempts: u32,
    pub(super) available_at: String,
    pub(super) delivered_at: Option<String>,
    pub(super) acked_at: Option<String>,
    pub(super) last_error: Option<String>,
    pub(super) run_id: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkerDeliveriesResponse {
    pub(super) deliveries: Vec<HiveWorkerDelivery>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkerDmResponse {
    pub(super) worker_id: String,
    pub(super) session_id: String,
    pub(super) title: String,
    pub(super) session_type: &'static str,
    pub(super) permission_mode: String,
    pub(super) created: bool,
    pub(super) agent_state: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateWorkerRequest {
    pub(super) slug: String,
    pub(super) display_name: Option<String>,
    pub(super) avatar_color: Option<String>,
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) model_key: Option<CoreModelKey>,
    pub(super) permission_mode: Option<PermissionMode>,
    pub(super) autonomy: Option<HiveWorkerAutonomy>,
    pub(super) heartbeat_interval_secs: Option<u32>,
    pub(super) identity: Option<String>,
    pub(super) soul: Option<String>,
}

/// Partial update: absent fields keep their current value.
#[derive(Debug, Deserialize)]
pub(super) struct UpdateWorkerRequest {
    pub(super) expected_revision: u64,
    pub(super) display_name: Option<String>,
    pub(super) avatar_color: Option<String>,
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) model_key: Option<CoreModelKey>,
    pub(super) permission_mode: Option<PermissionMode>,
    pub(super) autonomy: Option<HiveWorkerAutonomy>,
    pub(super) heartbeat_interval_secs: Option<u32>,
    pub(super) identity: Option<String>,
    pub(super) soul: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetWorkerStatusRequest {
    pub(super) expected_revision: u64,
}

pub(super) async fn list_workers(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveWorkersResponse>, AppError> {
    let store = open_worker_store(&state)?;
    let session_manager = open_session_manager(&state)?;
    let introduction_db = Database::new(&state.db_path)?;
    let introduction_store = HiveWorkerIntroductionStore::new(&introduction_db);
    let workers = store.list_for_owner(current_user_id(user.as_ref()), false)?;

    let mut summaries = Vec::with_capacity(workers.len());
    for worker in workers {
        let dm_agent_state = dm_agent_state(&session_manager, &worker)?;
        let introduction = introduction_store.get_by_worker(&worker.id)?;
        let mut summary = summarize_worker(worker, dm_agent_state);
        summary.introduction_status = introduction
            .as_ref()
            .map(|row| row.status.as_str().to_string());
        summary.introduction_last_error = introduction.and_then(|row| row.last_error);
        summaries.push(summary);
    }
    Ok(Json(HiveWorkersResponse { workers: summaries }))
}

/// Quiet creation exists only to seed focused route tests. Every public Worker
/// creation path is assistant-first and routes through
/// [`create_worker_introduction`].
#[cfg(test)]
pub(super) async fn create_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<CreateWorkerRequest>,
) -> Result<(StatusCode, Json<HiveWorkerDetailResponse>), AppError> {
    let user_id = current_user_id(user.as_ref());
    let slug = req.slug.trim().to_string();
    if !is_valid_crew_slug(&slug) {
        return Err(AppError::BadRequest(
            "invalid Worker slug; use 1-64 lowercase letters, digits, hyphens, or underscores"
                .to_string(),
        ));
    }
    let identity = validate_document(req.identity.as_deref(), "identity")?;
    let soul = validate_document(req.soul.as_deref(), "soul")?;
    if let Some(interval) = req.heartbeat_interval_secs {
        if interval == 0 {
            return Err(AppError::BadRequest(
                "heartbeat_interval_secs must be positive".to_string(),
            ));
        }
    }

    let store = open_worker_store(&state)?;
    if store.get_by_slug(user_id, &slug)?.is_some() {
        return Err(AppError::Conflict(format!(
            "A Worker with slug '{slug}' already exists"
        )));
    }

    let resolved_model =
        resolve_optional_worker_model(&state, user_id, req.model.as_deref(), &req.model_key)
            .await?;
    let worker = store.create(&NewHiveWorker {
        user_id: user_id.map(ToOwned::to_owned),
        slug,
        display_name: trimmed_nonempty(req.display_name.as_deref()).map(ToOwned::to_owned),
        avatar_color: trimmed_nonempty(req.avatar_color.as_deref()).map(ToOwned::to_owned),
        model: resolved_model.as_ref().map(|model| model.model.clone()),
        model_key: resolved_model.as_ref().map(|model| model.key.clone()),
        model_catalog_revision: resolved_model
            .as_ref()
            .and_then(|model| model.catalog_revision.clone()),
        permission_mode: req.permission_mode.unwrap_or_default(),
        autonomy: req.autonomy.unwrap_or_default(),
        heartbeat_interval_secs: req.heartbeat_interval_secs,
        dm_session_id: None,
        memory_namespace_id: None,
    })?;

    if let Some(identity) = identity {
        store.upsert_document(&worker.id, HiveWorkerDocumentKind::Identity, identity)?;
    }
    if let Some(soul) = soul {
        store.upsert_document(&worker.id, HiveWorkerDocumentKind::Soul, soul)?;
    }

    let detail = load_worker_detail(&state, &store, worker, None)?;
    Ok((StatusCode::CREATED, Json(detail)))
}

/// Atomic public create-and-meet path. The daemon owns the Worker, DM,
/// controller, Introduction ledger, and queued run in one idempotent
/// transaction. No user-authored message is inserted.
pub(super) async fn create_worker_introduction(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    headers: HeaderMap,
    Json(req): Json<CreateWorkerRequest>,
) -> Result<(StatusCode, Json<HiveWorkerDetailResponse>), AppError> {
    let idempotency_key = idempotency_key_from_headers(&headers)?.ok_or_else(|| {
        AppError::BadRequest(
            "Idempotency-Key is required when creating and meeting a Worker".into(),
        )
    })?;
    let user_id = current_user_id(user.as_ref());
    let slug = req.slug.trim().to_string();
    if !is_valid_crew_slug(&slug) {
        return Err(AppError::BadRequest(
            "invalid Worker slug; use 1-64 lowercase letters, digits, hyphens, or underscores"
                .to_string(),
        ));
    }
    let identity = validate_document(req.identity.as_deref(), "identity")?;
    let soul = validate_document(req.soul.as_deref(), "soul")?;
    if req.heartbeat_interval_secs == Some(0) {
        return Err(AppError::BadRequest(
            "heartbeat_interval_secs must be positive".to_string(),
        ));
    }

    // Introduction is never allowed to inherit a mutable default. Resolve the
    // submitted selection to one exact provider/auth/transport identity before
    // the daemon commits anything.
    let resolved_model = resolve_hive_model(
        &state,
        user_id,
        req.model.as_deref(),
        req.model_key.as_ref(),
    )
    .await?;
    let protocol_model_key = resolved_model.protocol_key()?;
    let display_name = trimmed_nonempty(req.display_name.as_deref())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| display_name_from_slug(&slug));
    let permission_mode = req.permission_mode.unwrap_or_default();
    let autonomy = req.autonomy.unwrap_or_default();
    let result = state
        .hive_runtime
        .create_worker_introduction_for_user(
            user_id,
            mitsuro_hive_protocol::CreateWorkerIntroductionCommand {
                slug,
                display_name,
                avatar_color: trimmed_nonempty(req.avatar_color.as_deref()).map(ToOwned::to_owned),
                model: resolved_model.model,
                model_key: protocol_model_key,
                model_catalog_revision: resolved_model.catalog_revision,
                permission_mode: permission_mode.as_str().to_string(),
                autonomy: autonomy.as_str().to_string(),
                heartbeat_interval_secs: req.heartbeat_interval_secs,
                identity: identity.map(ToOwned::to_owned),
                soul: soul.map(ToOwned::to_owned),
            },
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;

    let store = open_worker_store(&state)?;
    let worker = store
        .get(&result.worker_id)?
        .ok_or_else(|| AppError::Internal("created Worker could not be reloaded".into()))?;
    if worker.user_id.as_deref() != user_id
        || worker.dm_session_id.as_deref() != Some(result.session_id.as_str())
    {
        return Err(AppError::Internal(
            "created Worker did not preserve its owner or private conversation binding".into(),
        ));
    }
    let detail = load_worker_detail(&state, &store, worker, Some("idle".to_string()))?;
    Ok((StatusCode::CREATED, Json(detail)))
}

pub(super) async fn retry_worker_introduction(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    let idempotency_key = introduction_action_key(&headers, "retry")?;
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    ensure_not_archived(&worker, "retry its Introduction")?;
    let result = state
        .hive_runtime
        .retry_worker_introduction_for_user(user_id, &id, &idempotency_key)
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    load_introduction_action_result(&state, &store, user_id, &id, result)
}

pub(super) async fn skip_worker_introduction(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    let idempotency_key = introduction_action_key(&headers, "skip")?;
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    ensure_not_archived(&worker, "skip its Introduction")?;
    let result = state
        .hive_runtime
        .skip_worker_introduction_for_user(user_id, &id, &idempotency_key)
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    load_introduction_action_result(&state, &store, user_id, &id, result)
}

pub(super) async fn confirm_worker_introduction(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ConfirmWorkerIntroductionRequest>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    let idempotency_key = introduction_action_key(&headers, "confirm")?;
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    ensure_not_archived(&worker, "confirm its Introduction")?;
    let result = state
        .hive_runtime
        .confirm_worker_introduction_for_user(
            user_id,
            mitsuro_hive_protocol::ConfirmWorkerIntroductionCommand {
                worker_id: id.clone(),
                proposal_id: req.proposal_id,
                proposal_revision: req.proposal_revision,
                selected_facts: req.selected_facts,
            },
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    load_introduction_action_result(&state, &store, user_id, &id, result)
}

pub(super) async fn keep_talking_worker_introduction(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<KeepTalkingWorkerIntroductionRequest>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    let idempotency_key = introduction_action_key(&headers, "keep talking with")?;
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    ensure_not_archived(&worker, "continue its Introduction")?;
    let result = state
        .hive_runtime
        .return_worker_introduction_to_context_for_user(
            user_id,
            mitsuro_hive_protocol::ReturnWorkerIntroductionToContextCommand {
                worker_id: id.clone(),
                proposal_id: req.proposal_id,
                proposal_revision: req.proposal_revision,
                decision: mitsuro_hive_protocol::WorkerIntroductionReturnDecision::KeepTalking,
            },
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    load_introduction_action_result(&state, &store, user_id, &id, result)
}

fn introduction_action_key(headers: &HeaderMap, action: &str) -> Result<String, AppError> {
    idempotency_key_from_headers(headers)?.ok_or_else(|| {
        AppError::BadRequest(format!(
            "Idempotency-Key is required to {action} a Worker Introduction"
        ))
    })
}

pub(super) fn load_introduction_action_result(
    state: &AppState,
    store: &HiveWorkerStore,
    user_id: Option<&str>,
    requested_worker_id: &str,
    result: mitsuro_hive_protocol::WorkerIntroductionActionResponse,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    if result.worker_id != requested_worker_id {
        return Err(AppError::Internal(
            "Introduction action returned a different Worker".into(),
        ));
    }
    let worker = store
        .get(requested_worker_id)?
        .ok_or_else(|| AppError::Internal("Worker could not be reloaded".into()))?;
    if worker.user_id.as_deref() != user_id
        || worker.dm_session_id.as_deref() != Some(result.session_id.as_str())
    {
        return Err(AppError::Internal(
            "Introduction action did not preserve owner or private conversation binding".into(),
        ));
    }
    let session_manager = open_session_manager(state)?;
    let dm_agent_state = dm_agent_state(&session_manager, &worker)?;
    let detail = load_worker_detail(state, store, worker, dm_agent_state)?;
    let introduction = detail.introduction.as_ref().ok_or_else(|| {
        AppError::Internal("Introduction action did not leave durable lifecycle state".into())
    })?;
    if introduction.run_id != result.run_id {
        return Err(AppError::Internal(
            "Introduction action response does not match the durable lifecycle run".into(),
        ));
    }
    match result.status.as_str() {
        // The scheduler can advance the exact retry run before this HTTP
        // projection reloads it. The run id is the stable acceptance fence;
        // return the newer durable lifecycle state instead of turning a
        // successful idempotent retry into a false 500.
        "queued"
            if matches!(
                introduction.status.as_str(),
                "queued"
                    | "running"
                    | "awaiting_context"
                    | "review_ready"
                    | "confirmed"
                    | "failed"
                    | "needs_recovery"
                    | "skipped"
            ) => {}
        "skipped" if introduction.status == "skipped" && result.autonomy_eligible => {}
        "confirmed" if introduction.status == "confirmed" && result.autonomy_eligible => {}
        // Returning a proposal to context can race the next user exchange and
        // its dedicated review. The action's stable run/owner binding is the
        // acceptance fence; project any newer durable lifecycle.
        "awaiting_context"
            if !result.autonomy_eligible
                && matches!(
                    introduction.status.as_str(),
                    "awaiting_context"
                        | "review_ready"
                        | "confirmed"
                        | "skipped"
                        | "failed"
                        | "needs_recovery"
                ) => {}
        "queued" | "skipped" | "confirmed" | "awaiting_context" => {
            return Err(AppError::Internal(
                "Introduction action response conflicts with durable lifecycle state".into(),
            ));
        }
        _ => {
            return Err(AppError::Internal(
                "Introduction action returned an unsupported lifecycle status".into(),
            ));
        }
    }
    Ok(Json(detail))
}

pub(super) async fn get_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    let session_manager = open_session_manager(&state)?;
    let dm_agent_state = dm_agent_state(&session_manager, &worker)?;
    Ok(Json(load_worker_detail(
        &state,
        &store,
        worker,
        dm_agent_state,
    )?))
}

pub(super) async fn get_worker_by_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
) -> Result<Json<HiveWorkerSessionBindingResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_session(&session_manager, &session_id, user.as_ref())?;
    if session.session_type != SessionType::Hive {
        return Err(AppError::NotFound(format!(
            "Hive session {session_id} not found"
        )));
    }
    if session_manager.is_internal_hive_group_lane(&session_id)? {
        // Keep this route independently fail-closed even if the shared owned
        // session loader's hidden-lane filtering changes later.
        return Err(AppError::NotFound(format!(
            "Hive session {session_id} not found"
        )));
    }

    let store = open_worker_store(&state)?;
    let Some(worker) = store.get_by_dm_session(&session_id)? else {
        return Ok(Json(HiveWorkerSessionBindingResponse::PrimaryHive {
            session_id,
        }));
    };
    let user_id = current_user_id(user.as_ref());
    if worker.user_id.as_deref() != user_id
        || worker.dm_session_id.as_deref() != Some(session_id.as_str())
    {
        return Err(AppError::NotFound(format!(
            "Hive session {session_id} not found"
        )));
    }

    let dm_agent_state = dm_agent_state(&session_manager, &worker)?;
    let detail = load_worker_detail(&state, &store, worker, dm_agent_state)?;
    Ok(Json(HiveWorkerSessionBindingResponse::WorkerDm {
        session_id,
        worker: Box::new(detail),
    }))
}

pub(super) async fn update_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateWorkerRequest>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    let idempotency_key = worker_mutation_key(&headers, "update")?;
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    ensure_not_archived(&worker, "update")?;
    if req.expected_revision == 0 {
        return Err(AppError::BadRequest(
            "expected_revision must be at least 1".into(),
        ));
    }

    let identity = validate_document(req.identity.as_deref(), "identity")?;
    let soul = validate_document(req.soul.as_deref(), "soul")?;
    if let Some(interval) = req.heartbeat_interval_secs {
        if interval == 0 {
            return Err(AppError::BadRequest(
                "heartbeat_interval_secs must be positive".to_string(),
            ));
        }
    }
    let display_name = match req.display_name.as_deref() {
        Some(value) => Some(
            trimmed_nonempty(Some(value))
                .ok_or_else(|| AppError::BadRequest("display_name must not be empty".to_string()))?
                .to_string(),
        ),
        None => None,
    };

    let resolved_model =
        resolve_optional_worker_model(&state, user_id, req.model.as_deref(), &req.model_key)
            .await?;
    let documents = store.documents(&worker.id)?;
    let current_document = |kind: HiveWorkerDocumentKind| {
        documents
            .iter()
            .find(|document| document.kind == kind)
            .map(|document| document.content.clone())
    };
    let final_model = resolved_model
        .as_ref()
        .map(|model| model.model.clone())
        .or_else(|| worker.model.clone());
    let final_model_key = match resolved_model.as_ref() {
        Some(model) => Some(model.protocol_key()?),
        None => worker
            .model_key
            .as_ref()
            .map(core_model_key_to_protocol)
            .transpose()?,
    };
    let result = state
        .hive_runtime
        .update_worker_for_user(
            user_id,
            mitsuro_hive_protocol::UpdateWorkerCommand {
                worker_id: worker.id.clone(),
                expected_revision: req.expected_revision,
                display_name: display_name.unwrap_or_else(|| worker.display_name.clone()),
                avatar_color: trimmed_nonempty(req.avatar_color.as_deref())
                    .map(ToOwned::to_owned)
                    .or_else(|| worker.avatar_color.clone()),
                model: final_model,
                model_key: final_model_key,
                model_catalog_revision: resolved_model
                    .as_ref()
                    .and_then(|model| model.catalog_revision.clone())
                    .or_else(|| worker.model_catalog_revision.clone()),
                permission_mode: req
                    .permission_mode
                    .unwrap_or(worker.permission_mode)
                    .as_str()
                    .to_string(),
                autonomy: req.autonomy.unwrap_or(worker.autonomy).as_str().to_string(),
                heartbeat_interval_secs: req
                    .heartbeat_interval_secs
                    .or(worker.heartbeat_interval_secs),
                identity: identity
                    .map(ToOwned::to_owned)
                    .or_else(|| current_document(HiveWorkerDocumentKind::Identity)),
                soul: soul
                    .map(ToOwned::to_owned)
                    .or_else(|| current_document(HiveWorkerDocumentKind::Soul)),
            },
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    let updated = store
        .get(&worker.id)?
        .ok_or_else(|| worker_not_found(&id))?;
    validate_worker_mutation_result(&result, &updated, req.expected_revision)?;
    let session_manager = open_session_manager(&state)?;
    let dm_agent_state = dm_agent_state(&session_manager, &updated)?;
    Ok(Json(load_worker_detail(
        &state,
        &store,
        updated,
        dm_agent_state,
    )?))
}

pub(super) async fn pause_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SetWorkerStatusRequest>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    set_worker_status(
        &state,
        user,
        &id,
        req.expected_revision,
        mitsuro_hive_protocol::WorkerTargetStatus::Paused,
        "pause",
        &headers,
    )
    .await
}

pub(super) async fn resume_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SetWorkerStatusRequest>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    set_worker_status(
        &state,
        user,
        &id,
        req.expected_revision,
        mitsuro_hive_protocol::WorkerTargetStatus::Active,
        "resume",
        &headers,
    )
    .await
}

/// Archive (never hard-delete): history, documents, and the DM session
/// survive; the slug is freed for future Workers by the schema's
/// active-scope unique index. Returns a JSON body (not 204) because the
/// shared client decodes every successful response.
pub(super) async fn archive_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SetWorkerStatusRequest>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    set_worker_status(
        &state,
        user,
        &id,
        req.expected_revision,
        mitsuro_hive_protocol::WorkerTargetStatus::Archived,
        "archive",
        &headers,
    )
    .await
}

/// Ensure the Worker's private DM session exists and is bound, mirroring the
/// durable companion ensure: parentless, workspace-neutral, titled with the
/// Worker's display name, frozen to the Worker's model identity.
pub(super) async fn ensure_worker_dm(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<HiveWorkerDmResponse>, AppError> {
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    ensure_not_archived(&worker, "open a DM with")?;

    let session_manager = open_session_manager(&state)?;
    if let Some(dm_session_id) = worker.dm_session_id.as_deref() {
        if let Some(session) = session_manager.get_session(dm_session_id)? {
            if session_visible_to_user(&session, user_id) {
                let agent_state = load_agent_state_or_idle(&session_manager, &session.id)?.state;
                return Ok(Json(HiveWorkerDmResponse {
                    worker_id: worker.id,
                    session_id: session.id,
                    title: session.title,
                    session_type: CANONICAL_HIVE_SESSION_TYPE,
                    permission_mode: session.permission_mode.as_str().to_string(),
                    created: false,
                    agent_state,
                }));
            }
        }
        // A dangling binding cannot normally happen (the schema clears it when
        // the session is deleted); recover by creating a fresh DM lane.
    }

    let session_id = session_manager.create_session_for_user_with_config_and_permission(
        &worker.display_name,
        worker.model.as_deref(),
        None,
        None,
        WorkspaceMode::Neutral,
        user_id,
        None,
        SessionType::Hive,
        worker.permission_mode,
    )?;
    if let Some(key) = worker.model_key.as_ref() {
        session_manager.update_session_model_selection(
            &session_id,
            Some(key),
            worker.model_catalog_revision.as_deref(),
        )?;
    }
    store.bind_dm_session(&worker.id, Some(&session_id))?;

    Ok(Json(HiveWorkerDmResponse {
        worker_id: worker.id,
        session_id,
        title: worker.display_name,
        session_type: CANONICAL_HIVE_SESSION_TYPE,
        permission_mode: worker.permission_mode.as_str().to_string(),
        created: true,
        agent_state: "idle".to_string(),
    }))
}

pub(super) async fn list_worker_deliveries(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<ListWorkerDeliveriesQuery>,
) -> Result<Json<HiveWorkerDeliveriesResponse>, AppError> {
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    let status = match query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(value) => Some(HiveDeliveryStatus::parse(value).ok_or_else(|| {
            AppError::BadRequest(format!("unsupported delivery status: {value}"))
        })?),
        None => None,
    };
    let deliveries = HiveDeliveryStore::new(Database::new(&state.db_path)?).list_for_worker(
        &worker.id,
        status,
        query.limit.unwrap_or(50),
    )?;
    Ok(Json(HiveWorkerDeliveriesResponse {
        deliveries: deliveries.into_iter().map(summarize_delivery).collect(),
    }))
}

fn summarize_delivery(delivery: HiveDelivery) -> HiveWorkerDelivery {
    HiveWorkerDelivery {
        id: delivery.id,
        kind: delivery.kind.as_str().to_string(),
        from_worker_id: delivery.from_worker_id,
        to_worker_id: delivery.to_worker_id,
        group_id: delivery.group_id,
        body: delivery.body,
        priority: delivery.priority.as_str().to_string(),
        status: delivery.status.as_str().to_string(),
        attempt_count: delivery.attempt_count,
        max_attempts: delivery.max_attempts,
        available_at: delivery.available_at,
        delivered_at: delivery.delivered_at,
        acked_at: delivery.acked_at,
        last_error: delivery.last_error,
        run_id: delivery.run_id,
        created_at: delivery.created_at,
        updated_at: delivery.updated_at,
    }
}

async fn set_worker_status(
    state: &AppState,
    user: Option<CurrentUser>,
    id: &str,
    expected_revision: u64,
    status: mitsuro_hive_protocol::WorkerTargetStatus,
    action: &str,
    headers: &HeaderMap,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    if expected_revision == 0 {
        return Err(AppError::BadRequest(
            "expected_revision must be at least 1".into(),
        ));
    }
    let idempotency_key = worker_mutation_key(headers, action)?;
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(state)?;
    let worker = load_owned_worker(&store, id, user.as_ref())?;
    let result = state
        .hive_runtime
        .set_worker_status_for_user(
            user_id,
            &worker.id,
            expected_revision,
            status,
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    let updated = store.get(&worker.id)?.ok_or_else(|| worker_not_found(id))?;
    validate_worker_mutation_result(&result, &updated, expected_revision)?;
    let session_manager = open_session_manager(state)?;
    let dm_agent_state = dm_agent_state(&session_manager, &updated)?;
    Ok(Json(load_worker_detail(
        state,
        &store,
        updated,
        dm_agent_state,
    )?))
}

fn worker_mutation_key(headers: &HeaderMap, action: &str) -> Result<String, AppError> {
    idempotency_key_from_headers(headers)?.ok_or_else(|| {
        AppError::BadRequest(format!("Idempotency-Key is required to {action} a Worker"))
    })
}

fn validate_worker_mutation_result(
    result: &mitsuro_hive_protocol::WorkerMutationResponse,
    worker: &HiveWorker,
    expected_revision: u64,
) -> Result<(), AppError> {
    if result.worker_id != worker.id
        || result.revision != worker.revision
        || result.status != worker.status.as_str()
        || result.revision < expected_revision
    {
        return Err(AppError::Internal(
            "Worker mutation response conflicts with durable Worker state".into(),
        ));
    }
    Ok(())
}

fn core_model_key_to_protocol(
    key: &CoreModelKey,
) -> Result<mitsuro_hive_protocol::ModelKey, AppError> {
    serde_json::from_value(serde_json::to_value(key)?).map_err(|error| {
        AppError::Internal(format!(
            "stored Worker model key cannot cross the daemon protocol: {error}"
        ))
    })
}

fn open_worker_store(state: &AppState) -> Result<HiveWorkerStore, AppError> {
    Ok(HiveWorkerStore::new(Database::new(&state.db_path)?))
}

/// Exact-owner load: a Worker owned by another user (or by the local NULL
/// profile) is indistinguishable from a missing one.
fn load_owned_worker(
    store: &HiveWorkerStore,
    id: &str,
    user: Option<&CurrentUser>,
) -> Result<HiveWorker, AppError> {
    let worker = store.get(id)?.ok_or_else(|| worker_not_found(id))?;
    if worker.user_id.as_deref() != current_user_id(user) {
        return Err(worker_not_found(id));
    }
    Ok(worker)
}

fn worker_not_found(id: &str) -> AppError {
    AppError::NotFound(format!("Worker {id} not found"))
}

fn ensure_not_archived(worker: &HiveWorker, action: &str) -> Result<(), AppError> {
    if worker.status == HiveWorkerStatus::Archived {
        return Err(AppError::Conflict(format!(
            "Cannot {action} an archived Worker"
        )));
    }
    Ok(())
}

async fn resolve_optional_worker_model(
    state: &AppState,
    user_id: Option<&str>,
    model: Option<&str>,
    model_key: &Option<CoreModelKey>,
) -> Result<Option<ResolvedHiveModel>, AppError> {
    let model = trimmed_nonempty(model);
    if model.is_none() && model_key.is_none() {
        return Ok(None);
    }
    resolve_hive_model(state, user_id, model, model_key.as_ref())
        .await
        .map(Some)
}

fn validate_document<'a>(
    content: Option<&'a str>,
    label: &str,
) -> Result<Option<&'a str>, AppError> {
    let Some(raw) = content else { return Ok(None) };
    let trimmed = trimmed_nonempty(Some(raw))
        .ok_or_else(|| AppError::BadRequest(format!("{label} document must not be empty")))?;
    if trimmed.len() > MAX_HIVE_PROFILE_DOCUMENT_BYTES {
        return Err(AppError::BadRequest(format!(
            "{label} document exceeds the {MAX_HIVE_PROFILE_DOCUMENT_BYTES}-byte limit"
        )));
    }
    Ok(Some(trimmed))
}

fn dm_agent_state(
    session_manager: &SessionManager,
    worker: &HiveWorker,
) -> Result<Option<String>, AppError> {
    let Some(dm_session_id) = worker.dm_session_id.as_deref() else {
        return Ok(None);
    };
    Ok(Some(
        load_agent_state_or_idle(session_manager, dm_session_id)?.state,
    ))
}

fn summarize_worker(worker: HiveWorker, dm_agent_state: Option<String>) -> HiveWorkerSummary {
    HiveWorkerSummary {
        id: worker.id,
        revision: worker.revision,
        slug: worker.slug,
        display_name: worker.display_name,
        avatar_color: worker.avatar_color,
        model: worker.model,
        model_key: worker.model_key,
        permission_mode: worker.permission_mode.as_str().to_string(),
        autonomy: worker.autonomy.as_str().to_string(),
        heartbeat_interval_secs: worker.heartbeat_interval_secs,
        status: worker.status.as_str().to_string(),
        dm_session_id: worker.dm_session_id,
        dm_agent_state,
        introduction_status: None,
        introduction_last_error: None,
        created_at: worker.created_at,
        updated_at: worker.updated_at,
    }
}

fn load_worker_detail(
    state: &AppState,
    store: &HiveWorkerStore,
    worker: HiveWorker,
    dm_agent_state: Option<String>,
) -> Result<HiveWorkerDetailResponse, AppError> {
    let documents = store.documents(&worker.id)?;
    let document = |kind: HiveWorkerDocumentKind| {
        documents
            .iter()
            .find(|document| document.kind == kind)
            .map(|document| document.content.clone())
    };
    let introduction_db = Database::new(&state.db_path)?;
    let introduction_store = HiveWorkerIntroductionStore::new(&introduction_db);
    let expected_worker_id = worker.id.clone();
    let expected_session_id = worker.dm_session_id.clone();
    let introduction = introduction_store
        .get_by_worker(&worker.id)?
        .map(
            |introduction| -> Result<HiveWorkerIntroductionResponse, AppError> {
                let proposal = introduction
                    .proposal
                    .map(serde_json::from_value::<WorkerIntroductionProposalV1>)
                    .transpose()
                    .map_err(|error| {
                        AppError::Internal(format!(
                            "stored Worker Introduction proposal is not strict V1: {error}"
                        ))
                    })?;
                if let Some(proposal) = proposal.as_ref() {
                    if proposal.schema_version != WORKER_INTRODUCTION_PROPOSAL_VERSION
                        || proposal.worker_id != expected_worker_id
                        || Some(proposal.session_id.as_str()) != expected_session_id.as_deref()
                        || proposal.revision != introduction.proposal_revision
                        || proposal.proposal_id.trim().is_empty()
                        || proposal.facts.is_empty()
                        || proposal.facts.len() > MAX_WORKER_INTRODUCTION_FACTS
                    {
                        return Err(AppError::Internal(
                            "stored Worker Introduction proposal binding is invalid".into(),
                        ));
                    }
                }
                if matches!(introduction.status.as_str(), "review_ready" | "confirmed")
                    && proposal.is_none()
                {
                    return Err(AppError::Internal(
                        "reviewed Worker Introduction has no strict V1 proposal".into(),
                    ));
                }
                if !matches!(introduction.status.as_str(), "review_ready" | "confirmed")
                    && proposal.is_some()
                {
                    return Err(AppError::Internal(
                        "Worker Introduction proposal escaped the review-ready lifecycle".into(),
                    ));
                }
                let review_projection = introduction_store
                    .get_review_projection(&worker.id)?
                    .ok_or_else(|| {
                        AppError::Internal(
                            "Worker Introduction review projection disappeared".into(),
                        )
                    })?;
                if review_projection.worker_id != expected_worker_id
                    || review_projection.lifecycle_status != introduction.status
                {
                    return Err(AppError::Internal(
                        "Worker Introduction review projection binding is invalid".into(),
                    ));
                }
                Ok(HiveWorkerIntroductionResponse {
                    run_id: introduction.run_id,
                    status: introduction.status.as_str().to_string(),
                    prompt_version: introduction.prompt_version,
                    opening_message_id: introduction.opening_message_id,
                    proposal,
                    proposal_revision: introduction.proposal_revision,
                    review_projection,
                    last_error: introduction.last_error,
                    completed_at: introduction.completed_at,
                })
            },
        )
        .transpose()?;
    let mut summary = summarize_worker(worker, dm_agent_state);
    summary.introduction_status = introduction.as_ref().map(|row| row.status.clone());
    summary.introduction_last_error = introduction.as_ref().and_then(|row| row.last_error.clone());
    Ok(HiveWorkerDetailResponse {
        identity: document(HiveWorkerDocumentKind::Identity),
        soul: document(HiveWorkerDocumentKind::Soul),
        introduction,
        attention: load_worker_attention(&introduction_db, &expected_worker_id)?,
        worker: summary,
    })
}

fn load_worker_attention(
    db: &Database,
    worker_id: &str,
) -> Result<Vec<mitsuro_hive_protocol::WorkerLaneAttention>, AppError> {
    let mut statement = db
        .conn()
        .prepare(
            "SELECT controller.id, controller.session_id, run.id
         FROM hive_controllers controller
         JOIN hive_runs run ON run.controller_id = controller.id
         WHERE controller.worker_id = ?1 AND run.worker_id = ?1
           AND run.status = 'recovery_required'
         ORDER BY controller.session_id ASC, run.updated_at ASC, run.id ASC",
        )
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let rows = statement
        .query_map([worker_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let mut attention = Vec::<mitsuro_hive_protocol::WorkerLaneAttention>::new();
    for row in rows {
        let (controller_id, session_id, run_id) =
            row.map_err(|error| AppError::Internal(error.to_string()))?;
        if let Some(existing) = attention
            .iter_mut()
            .find(|entry| entry.controller_id == controller_id)
        {
            existing.recovery_run_ids.push(run_id);
        } else {
            attention.push(mitsuro_hive_protocol::WorkerLaneAttention {
                session_id,
                controller_id,
                recovery_run_ids: vec![run_id],
                reason: "A prior Worker run has uncertain effects and requires explicit recovery"
                    .into(),
            });
        }
    }
    Ok(attention)
}
