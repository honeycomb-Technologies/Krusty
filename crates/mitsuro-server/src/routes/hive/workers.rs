//! Hive Worker CRUD and DM-lane endpoints.
//!
//! Workers are first-class durable identities. These routes keep ownership
//! exact-owner scoped (NULL owner = local profile, never a wildcard), freeze
//! model identity through the shared Hive model resolver, and treat delete as
//! archive so a Worker's history and documents are never destroyed.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use mitsuro_core::ai::models::ModelKey as CoreModelKey;
use mitsuro_core::storage::{
    is_valid_crew_slug, Database, HiveWorker, HiveWorkerAutonomy, HiveWorkerDocumentKind,
    HiveWorkerProfileUpdate, HiveWorkerStatus, HiveWorkerStore, NewHiveWorker, SessionType,
    WorkspaceMode, MAX_HIVE_PROFILE_DOCUMENT_BYTES,
};
use mitsuro_core::tools::registry::PermissionMode;
use mitsuro_core::SessionManager;

use super::super::session_access::{
    current_user_id, load_agent_state_or_idle, session_visible_to_user,
};
use super::{open_session_manager, resolve_hive_model, OkResponse, ResolvedHiveModel};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::text::trimmed_nonempty;
use crate::AppState;

const CANONICAL_HIVE_SESSION_TYPE: &str = "hive";

#[derive(Debug, Serialize)]
pub(super) struct HiveWorkerSummary {
    pub(super) id: String,
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

pub(super) async fn list_workers(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveWorkersResponse>, AppError> {
    let store = open_worker_store(&state)?;
    let session_manager = open_session_manager(&state)?;
    let workers = store.list_for_owner(current_user_id(user.as_ref()), false)?;

    let mut summaries = Vec::with_capacity(workers.len());
    for worker in workers {
        let dm_agent_state = dm_agent_state(&session_manager, &worker)?;
        summaries.push(summarize_worker(worker, dm_agent_state));
    }
    Ok(Json(HiveWorkersResponse { workers: summaries }))
}

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

    let detail = load_worker_detail(&store, worker, None)?;
    Ok((StatusCode::CREATED, Json(detail)))
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
    Ok(Json(load_worker_detail(&store, worker, dm_agent_state)?))
}

pub(super) async fn update_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkerRequest>,
) -> Result<Json<HiveWorkerDetailResponse>, AppError> {
    let user_id = current_user_id(user.as_ref());
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    ensure_not_archived(&worker, "update")?;

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
    let profile_update = HiveWorkerProfileUpdate {
        display_name: display_name.unwrap_or_else(|| worker.display_name.clone()),
        avatar_color: match trimmed_nonempty(req.avatar_color.as_deref()) {
            Some(color) => Some(color.to_string()),
            None => worker.avatar_color.clone(),
        },
        model: resolved_model
            .as_ref()
            .map(|model| model.model.clone())
            .or_else(|| worker.model.clone()),
        model_key: resolved_model
            .as_ref()
            .map(|model| model.key.clone())
            .or_else(|| worker.model_key.clone()),
        model_catalog_revision: resolved_model
            .as_ref()
            .map(|model| model.catalog_revision.clone())
            .unwrap_or_else(|| worker.model_catalog_revision.clone()),
        permission_mode: req.permission_mode.unwrap_or(worker.permission_mode),
    };
    store
        .update_profile(&worker.id, &profile_update)?
        .ok_or_else(|| worker_not_found(&id))?;

    if req.autonomy.is_some() || req.heartbeat_interval_secs.is_some() {
        store.set_autonomy(
            &worker.id,
            req.autonomy.unwrap_or(worker.autonomy),
            req.heartbeat_interval_secs
                .or(worker.heartbeat_interval_secs),
        )?;
    }

    if let Some(identity) = identity {
        store.upsert_document(&worker.id, HiveWorkerDocumentKind::Identity, identity)?;
    }
    if let Some(soul) = soul {
        store.upsert_document(&worker.id, HiveWorkerDocumentKind::Soul, soul)?;
    }

    // The DM session row is the runtime source of truth for chat turns, so a
    // Worker-level model or permission change must reach it to take effect.
    let session_manager = open_session_manager(&state)?;
    if let Some(dm_session_id) = worker.dm_session_id.as_deref() {
        if let Some(model) = resolved_model.as_ref() {
            session_manager.update_session_model_selection(
                dm_session_id,
                Some(&model.key),
                model.catalog_revision.as_deref(),
            )?;
        }
        if let Some(permission_mode) = req.permission_mode {
            if permission_mode != worker.permission_mode {
                session_manager.update_session_permission_mode(dm_session_id, permission_mode)?;
            }
        }
    }

    let updated = store
        .get(&worker.id)?
        .ok_or_else(|| worker_not_found(&id))?;
    let dm_agent_state = dm_agent_state(&session_manager, &updated)?;
    Ok(Json(load_worker_detail(&store, updated, dm_agent_state)?))
}

pub(super) async fn pause_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<HiveWorkerSummary>, AppError> {
    set_worker_status(&state, user, &id, HiveWorkerStatus::Paused, "pause").await
}

pub(super) async fn resume_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<HiveWorkerSummary>, AppError> {
    set_worker_status(&state, user, &id, HiveWorkerStatus::Active, "resume").await
}

/// Archive (never hard-delete): history, documents, and the DM session
/// survive; the slug is freed for future Workers by the schema's
/// active-scope unique index. Returns a JSON body (not 204) because the
/// shared client decodes every successful response.
pub(super) async fn archive_worker(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let store = open_worker_store(&state)?;
    let worker = load_owned_worker(&store, &id, user.as_ref())?;
    if worker.status != HiveWorkerStatus::Archived {
        store.set_status(&worker.id, HiveWorkerStatus::Archived)?;
    }
    Ok(Json(OkResponse { ok: true }))
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

async fn set_worker_status(
    state: &AppState,
    user: Option<CurrentUser>,
    id: &str,
    status: HiveWorkerStatus,
    action: &str,
) -> Result<Json<HiveWorkerSummary>, AppError> {
    let store = open_worker_store(state)?;
    let worker = load_owned_worker(&store, id, user.as_ref())?;
    ensure_not_archived(&worker, action)?;
    store.set_status(&worker.id, status)?;
    let updated = store.get(&worker.id)?.ok_or_else(|| worker_not_found(id))?;
    let session_manager = open_session_manager(state)?;
    let dm_agent_state = dm_agent_state(&session_manager, &updated)?;
    Ok(Json(summarize_worker(updated, dm_agent_state)))
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
        created_at: worker.created_at,
        updated_at: worker.updated_at,
    }
}

fn load_worker_detail(
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
    Ok(HiveWorkerDetailResponse {
        identity: document(HiveWorkerDocumentKind::Identity),
        soul: document(HiveWorkerDocumentKind::Soul),
        worker: summarize_worker(worker, dm_agent_state),
    })
}
