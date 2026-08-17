use std::collections::BTreeMap;

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use mitsuro_core::storage::bootstrap_hive_home;
use mitsuro_core::storage::{
    is_valid_crew_slug, summarize_channel_bindings, summarize_crew_runtime, ApnsDeviceStore,
    AutonomousTaskStore, Database, DelegatedRunStore, HiveChannelBinding, HiveChannelKind,
    HiveCrewProfileDocumentKind, HiveCrewRuntimeSummary, HiveHomeDocument, HiveHomeProfile,
    HiveProfileDocument, HiveProfileDocumentKind, HiveProfileOwner, HiveProfileSnapshot,
    HiveProfileStore, HiveProfileStoreError, HiveRuntimeState, SessionInfo, SessionType,
};

use super::super::session_access::{current_user_id, session_visible_to_user};
use super::open_session_manager;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::text::trimmed_nonempty;
use crate::AppState;

const HIVE_HOME_PREVIEW_CHARS: usize = 160;

#[derive(Debug, Deserialize)]
pub(super) struct DocumentWriteRequest {
    pub(super) content: String,
    #[serde(default)]
    pub(super) expected_revision: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveHomeDocumentSummary {
    pub(super) file_name: String,
    pub(super) content: String,
    pub(super) preview: String,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveCrewMemberSummary {
    pub(super) slug: String,
    pub(super) identity: Option<HiveHomeDocumentSummary>,
    pub(super) soul: Option<HiveHomeDocumentSummary>,
    pub(super) memory: Option<HiveHomeDocumentSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveChannelSummary {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) kind: String,
    pub(super) source: String,
    pub(super) enabled: bool,
    pub(super) status: String,
    pub(super) detail: String,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveChannelsResponse {
    pub(super) items: Vec<HiveChannelSummary>,
    pub(super) apns_configured: bool,
    pub(super) apns_device_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveHomeResponse {
    pub(super) profile_id: Option<String>,
    pub(super) revision: Option<i64>,
    pub(super) soul: Option<HiveHomeDocumentSummary>,
    pub(super) identity: Option<HiveHomeDocumentSummary>,
    pub(super) user: Option<HiveHomeDocumentSummary>,
    pub(super) heartbeat: Option<HiveHomeDocumentSummary>,
    pub(super) memory: Option<HiveHomeDocumentSummary>,
    pub(super) channels: Option<HiveHomeDocumentSummary>,
    pub(super) crew: Vec<HiveCrewMemberSummary>,
    pub(super) crew_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveBootstrapResponse {
    pub(super) ok: bool,
    pub(super) created_files: Vec<String>,
    pub(super) home: HiveHomeResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveCrewResponse {
    pub(super) members: Vec<HiveCrewRuntimeMemberSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveCrewRuntimeMemberSummary {
    pub(super) slug: String,
    pub(super) known_to_home: bool,
    pub(super) status: String,
    pub(super) active_run_count: usize,
    pub(super) recent_run_count: usize,
    pub(super) failed_run_count: usize,
    pub(super) queued_task_count: usize,
    pub(super) active_task_count: usize,
    pub(super) completed_task_count: usize,
    pub(super) failed_task_count: usize,
    pub(super) latest_activity_at: Option<String>,
    pub(super) identity: Option<HiveHomeDocumentSummary>,
    pub(super) soul: Option<HiveHomeDocumentSummary>,
    pub(super) memory: Option<HiveHomeDocumentSummary>,
}

pub(super) async fn home(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveHomeResponse>, AppError> {
    let profile = load_or_bootstrap_profile(&state, user.as_ref())?;
    Ok(Json(build_hive_home_response_from_profile(&profile)))
}

pub(super) async fn bootstrap_home(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveBootstrapResponse>, AppError> {
    let owner = profile_owner(user.as_ref())?;
    let store = HiveProfileStore::new(Database::new(&state.db_path)?);
    if owner.is_local() {
        store
            .import_local_legacy_home(&owner, &mitsuro_core::paths::hive_dir())
            .map_err(map_profile_error)?;
    }
    let merged = store
        .bootstrap_defaults(&owner)
        .map_err(map_profile_error)?;
    let mut created_files = merged
        .inserted_documents
        .iter()
        .map(|kind| kind.preferred_file_name().to_string())
        .collect::<Vec<_>>();
    created_files.extend(
        merged
            .inserted_crew_documents
            .iter()
            .map(|(slug, kind)| format!("crew/{slug}/{}", kind.preferred_file_name())),
    );
    Ok(Json(HiveBootstrapResponse {
        ok: true,
        created_files,
        home: build_hive_home_response_from_profile(&merged.snapshot),
    }))
}

pub(super) async fn update_home_document(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(kind): Path<String>,
    Json(req): Json<DocumentWriteRequest>,
) -> Result<Json<HiveHomeResponse>, AppError> {
    let kind = HiveProfileDocumentKind::parse(&kind).ok_or_else(|| {
        AppError::BadRequest(
            "invalid Hive profile document kind; memory is managed by the memory API".to_string(),
        )
    })?;
    let content = trimmed_nonempty(Some(req.content.as_str()))
        .ok_or_else(|| AppError::BadRequest("content must not be empty".to_string()))?;
    let owner = profile_owner(user.as_ref())?;
    let store = HiveProfileStore::new(Database::new(&state.db_path)?);
    let snapshot = bootstrap_profile(&store, &owner)?;
    let updated = store
        .update_document(
            &owner,
            kind,
            content,
            req.expected_revision.unwrap_or(snapshot.revision),
        )
        .map_err(map_profile_error)?;

    Ok(Json(build_hive_home_response_from_profile(&updated)))
}

pub(super) async fn update_crew_document(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((slug, kind)): Path<(String, String)>,
    Json(req): Json<DocumentWriteRequest>,
) -> Result<Json<HiveHomeResponse>, AppError> {
    if !is_valid_crew_slug(&slug) {
        return Err(AppError::BadRequest("invalid crew slug".to_string()));
    }
    let kind = HiveCrewProfileDocumentKind::parse(&kind).ok_or_else(|| {
        AppError::BadRequest(
            "invalid Hive worker profile kind; worker memory is canonical memory, not identity"
                .to_string(),
        )
    })?;
    let content = trimmed_nonempty(Some(req.content.as_str()))
        .ok_or_else(|| AppError::BadRequest("content must not be empty".to_string()))?;
    let owner = profile_owner(user.as_ref())?;
    let store = HiveProfileStore::new(Database::new(&state.db_path)?);
    let snapshot = bootstrap_profile(&store, &owner)?;
    let updated = store
        .update_crew_document(
            &owner,
            &slug,
            kind,
            content,
            req.expected_revision.unwrap_or(snapshot.revision),
        )
        .map_err(map_profile_error)?;

    Ok(Json(build_hive_home_response_from_profile(&updated)))
}

pub(super) async fn crew(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveCrewResponse>, AppError> {
    let user_id = current_user_id(user.as_ref());
    let (sessions, runtime_states) = {
        let session_manager = open_session_manager(&state)?;
        let sessions = session_manager
            .list_sessions_for_user_by_type(None, user_id, SessionType::Hive)?
            .into_iter()
            .filter(|session| session_visible_to_user(session, user_id))
            .collect::<Vec<_>>();
        let runtime_states =
            mitsuro_core::storage::HiveRuntimeStateStore::new(Database::new(&state.db_path)?)
                .list_states_for_sessions(
                    &sessions
                        .iter()
                        .map(|session| session.id.clone())
                        .collect::<Vec<_>>(),
                )?;
        (sessions, runtime_states)
    };
    let task_store = AutonomousTaskStore::new(Database::new(&state.db_path)?);
    let delegated_store = DelegatedRunStore::new(Database::new(&state.db_path)?);

    let profile = load_or_bootstrap_profile(&state, user.as_ref())?;
    Ok(Json(build_hive_crew_response_from_profile_and_sessions(
        &profile,
        &sessions,
        &runtime_states,
        &task_store,
        &delegated_store,
    )?))
}

pub(super) async fn channels(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveChannelsResponse>, AppError> {
    let profile = load_or_bootstrap_profile(&state, user.as_ref())?;
    let db = Database::new(&state.db_path)?;
    let apns_store = ApnsDeviceStore::new(&db);
    let apns_device_count = apns_store.count_for_user(current_user_id(user.as_ref()))?;

    Ok(Json(build_hive_channels_response_from_profile(
        &state,
        &profile,
        apns_device_count,
    )))
}

fn profile_owner(user: Option<&CurrentUser>) -> Result<HiveProfileOwner, AppError> {
    HiveProfileOwner::from_user_id(current_user_id(user))
        .map_err(|error| AppError::BadRequest(error.to_string()))
}

fn bootstrap_profile(
    store: &HiveProfileStore,
    owner: &HiveProfileOwner,
) -> Result<HiveProfileSnapshot, AppError> {
    if owner.is_local() {
        store
            .import_local_legacy_home(owner, &mitsuro_core::paths::hive_dir())
            .map_err(map_profile_error)?;
    }
    store
        .bootstrap_defaults(owner)
        .map(|result| result.snapshot)
        .map_err(map_profile_error)
}

fn load_or_bootstrap_profile(
    state: &AppState,
    user: Option<&CurrentUser>,
) -> Result<HiveProfileSnapshot, AppError> {
    let owner = profile_owner(user)?;
    let store = HiveProfileStore::new(Database::new(&state.db_path)?);
    bootstrap_profile(&store, &owner)
}

fn map_profile_error(error: HiveProfileStoreError) -> AppError {
    let message = error.to_string();
    match error {
        HiveProfileStoreError::RevisionConflict { .. } => AppError::Conflict(message),
        HiveProfileStoreError::EmptyContent
        | HiveProfileStoreError::ContentTooLarge
        | HiveProfileStoreError::InvalidCrewSlug(_)
        | HiveProfileStoreError::InvalidOwner(_) => AppError::BadRequest(message),
        _ => AppError::Internal(message),
    }
}

fn summarize_profile_document<K>(
    document: Option<HiveProfileDocument<K>>,
    file_name: &str,
) -> Option<HiveHomeDocumentSummary> {
    document.map(|document| HiveHomeDocumentSummary {
        preview: truncate_preview(&document.content, HIVE_HOME_PREVIEW_CHARS),
        file_name: file_name.to_string(),
        content: document.content,
    })
}

fn build_hive_home_response_from_profile(profile: &HiveProfileSnapshot) -> HiveHomeResponse {
    HiveHomeResponse {
        profile_id: Some(profile.profile_id.clone()),
        revision: Some(profile.revision),
        soul: summarize_profile_document(
            profile.soul.clone(),
            HiveProfileDocumentKind::Soul.preferred_file_name(),
        ),
        identity: summarize_profile_document(
            profile.identity.clone(),
            HiveProfileDocumentKind::Identity.preferred_file_name(),
        ),
        user: summarize_profile_document(
            profile.user.clone(),
            HiveProfileDocumentKind::User.preferred_file_name(),
        ),
        heartbeat: summarize_profile_document(
            profile.heartbeat.clone(),
            HiveProfileDocumentKind::Heartbeat.preferred_file_name(),
        ),
        memory: None,
        channels: summarize_profile_document(
            profile.channels.clone(),
            HiveProfileDocumentKind::Channels.preferred_file_name(),
        ),
        crew_count: profile.crew.len(),
        crew: profile
            .crew
            .iter()
            .map(|member| HiveCrewMemberSummary {
                slug: member.slug.clone(),
                identity: summarize_profile_document(
                    member.identity.clone(),
                    HiveCrewProfileDocumentKind::Identity.preferred_file_name(),
                ),
                soul: summarize_profile_document(
                    member.soul.clone(),
                    HiveCrewProfileDocumentKind::Soul.preferred_file_name(),
                ),
                memory: None,
            })
            .collect(),
    }
}

fn legacy_profile_from_snapshot(profile: &HiveProfileSnapshot) -> HiveHomeProfile {
    let document = |value: &Option<HiveProfileDocument<HiveProfileDocumentKind>>,
                    file_name: &str| {
        value.as_ref().map(|document| HiveHomeDocument {
            file_name: file_name.to_string(),
            content: document.content.clone(),
        })
    };
    HiveHomeProfile {
        soul: document(&profile.soul, mitsuro_core::paths::HIVE_SOUL_FILE),
        identity: document(&profile.identity, mitsuro_core::paths::HIVE_IDENTITY_FILE),
        user: document(&profile.user, mitsuro_core::paths::HIVE_USER_FILE),
        heartbeat: document(&profile.heartbeat, mitsuro_core::paths::HIVE_HEARTBEAT_FILE),
        memory: None,
        channels: document(&profile.channels, mitsuro_core::paths::HIVE_CHANNELS_FILE),
        crew: profile
            .crew
            .iter()
            .map(|member| mitsuro_core::storage::HiveCrewProfile {
                slug: member.slug.clone(),
                identity: member.identity.as_ref().map(|document| HiveHomeDocument {
                    file_name: "IDENTITY.md".to_string(),
                    content: document.content.clone(),
                }),
                soul: member.soul.as_ref().map(|document| HiveHomeDocument {
                    file_name: "SOUL.md".to_string(),
                    content: document.content.clone(),
                }),
                memory: None,
            })
            .collect(),
    }
}

#[cfg(test)]
pub(super) fn build_hive_bootstrap_response_from_dir(
    hive_home: &std::path::Path,
) -> Result<HiveBootstrapResponse, AppError> {
    let result = bootstrap_hive_home(hive_home)
        .map_err(|error| AppError::Internal(format!("Failed to bootstrap Hive home: {}", error)))?;
    Ok(HiveBootstrapResponse {
        ok: true,
        created_files: result.created_files,
        home: build_hive_home_response_from_dir(hive_home),
    })
}

#[cfg(test)]
pub(super) fn build_hive_home_response_from_dir(hive_home: &std::path::Path) -> HiveHomeResponse {
    let profile = HiveHomeProfile::load_from(hive_home);

    HiveHomeResponse {
        profile_id: None,
        revision: None,
        soul: summarize_hive_home_document(profile.soul),
        identity: summarize_hive_home_document(profile.identity),
        user: summarize_hive_home_document(profile.user),
        heartbeat: summarize_hive_home_document(profile.heartbeat),
        memory: summarize_hive_home_document(profile.memory),
        channels: summarize_hive_home_document(profile.channels),
        crew_count: profile.crew.len(),
        crew: profile
            .crew
            .into_iter()
            .map(|member| HiveCrewMemberSummary {
                slug: member.slug,
                identity: summarize_hive_home_document(member.identity),
                soul: summarize_hive_home_document(member.soul),
                memory: summarize_hive_home_document(member.memory),
            })
            .collect(),
    }
}

#[cfg(test)]
pub(super) fn build_hive_crew_response_from_dir_and_sessions(
    hive_home: &std::path::Path,
    sessions: &[SessionInfo],
    runtime_states: &std::collections::HashMap<String, HiveRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<HiveCrewResponse, AppError> {
    let profile = HiveHomeProfile::load_from(hive_home);
    build_hive_crew_response_from_loaded_profile(
        &profile,
        sessions,
        runtime_states,
        task_store,
        delegated_store,
    )
}

fn build_hive_crew_response_from_profile_and_sessions(
    profile: &HiveProfileSnapshot,
    sessions: &[SessionInfo],
    runtime_states: &std::collections::HashMap<String, HiveRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<HiveCrewResponse, AppError> {
    build_hive_crew_response_from_loaded_profile(
        &legacy_profile_from_snapshot(profile),
        sessions,
        runtime_states,
        task_store,
        delegated_store,
    )
}

fn build_hive_crew_response_from_loaded_profile(
    profile: &HiveHomeProfile,
    sessions: &[SessionInfo],
    runtime_states: &std::collections::HashMap<String, HiveRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<HiveCrewResponse, AppError> {
    let profile_map = profile
        .crew
        .iter()
        .map(|member| (member.slug.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    let runtime = summarize_crew_runtime(
        profile,
        sessions,
        runtime_states,
        task_store,
        delegated_store,
    )
    .map_err(|error| AppError::Internal(format!("Failed to summarize Hive Workers: {}", error)))?;

    Ok(HiveCrewResponse {
        members: runtime
            .into_iter()
            .map(|member| {
                let profile = profile_map.get(member.slug.as_str());
                summarize_hive_crew_member(member, profile.copied())
            })
            .collect(),
    })
}

#[cfg(test)]
pub(super) fn build_hive_channels_response_from_dir(
    state: &AppState,
    hive_home: &std::path::Path,
    apns_device_count: usize,
) -> HiveChannelsResponse {
    let profile = HiveHomeProfile::load_from(hive_home);
    build_hive_channels_response_from_loaded_profile(state, &profile, apns_device_count)
}

fn build_hive_channels_response_from_profile(
    state: &AppState,
    profile: &HiveProfileSnapshot,
    apns_device_count: usize,
) -> HiveChannelsResponse {
    build_hive_channels_response_from_loaded_profile(
        state,
        &legacy_profile_from_snapshot(profile),
        apns_device_count,
    )
}

fn build_hive_channels_response_from_loaded_profile(
    state: &AppState,
    profile: &HiveHomeProfile,
    apns_device_count: usize,
) -> HiveChannelsResponse {
    let apns_configured = state.apns_service.is_some();

    HiveChannelsResponse {
        items: summarize_channel_bindings(profile)
            .into_iter()
            .map(|binding| summarize_hive_channel(binding, apns_configured, apns_device_count))
            .collect(),
        apns_configured,
        apns_device_count,
    }
}

fn summarize_hive_crew_member(
    runtime: HiveCrewRuntimeSummary,
    profile: Option<&mitsuro_core::storage::HiveCrewProfile>,
) -> HiveCrewRuntimeMemberSummary {
    HiveCrewRuntimeMemberSummary {
        slug: runtime.slug,
        known_to_home: runtime.known_to_home,
        status: runtime.status.as_str().to_string(),
        active_run_count: runtime.active_run_count,
        recent_run_count: runtime.recent_run_count,
        failed_run_count: runtime.failed_run_count,
        queued_task_count: runtime.queued_task_count,
        active_task_count: runtime.active_task_count,
        completed_task_count: runtime.completed_task_count,
        failed_task_count: runtime.failed_task_count,
        latest_activity_at: runtime.latest_activity_at,
        identity: summarize_hive_home_document(profile.and_then(|member| member.identity.clone())),
        soul: summarize_hive_home_document(profile.and_then(|member| member.soul.clone())),
        memory: summarize_hive_home_document(profile.and_then(|member| member.memory.clone())),
    }
}

fn summarize_hive_channel(
    binding: HiveChannelBinding,
    apns_configured: bool,
    apns_device_count: usize,
) -> HiveChannelSummary {
    let (status, detail) = match binding.kind {
        HiveChannelKind::MainThread => ("ready", binding.detail),
        HiveChannelKind::Crew => {
            if binding.enabled {
                ("ready", binding.detail)
            } else {
                ("inactive", binding.detail)
            }
        }
        HiveChannelKind::MobilePush => {
            if !binding.enabled {
                ("inactive", binding.detail)
            } else if !apns_configured {
                (
                    "attention",
                    format!("{} APNs is not configured on this server.", binding.detail),
                )
            } else if apns_device_count == 0 {
                (
                    "attention",
                    format!("{} No iPhone devices are registered yet.", binding.detail),
                )
            } else {
                (
                    "ready",
                    format!(
                        "{} {} device{} ready for push delivery.",
                        binding.detail,
                        apns_device_count,
                        if apns_device_count == 1 { "" } else { "s" }
                    ),
                )
            }
        }
        _ => {
            if binding.enabled {
                ("configured", binding.detail)
            } else {
                ("inactive", binding.detail)
            }
        }
    };

    HiveChannelSummary {
        id: binding.id,
        label: binding.label,
        kind: binding.kind.as_str().to_string(),
        source: binding.source.to_string(),
        enabled: binding.enabled,
        status: status.to_string(),
        detail: detail.trim().to_string(),
    }
}

fn summarize_hive_home_document(
    document: Option<HiveHomeDocument>,
) -> Option<HiveHomeDocumentSummary> {
    document.map(|document| HiveHomeDocumentSummary {
        preview: truncate_preview(&document.content, HIVE_HOME_PREVIEW_CHARS),
        file_name: document.file_name,
        content: document.content,
    })
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{}...", truncated)
}
