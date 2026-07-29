//! APNs device registration and test endpoints

use axum::{
    extract::State,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::storage::{
    ApnsDeviceRegistration, ApnsDeviceStore, Database, ExpoPushDeviceStore,
    LiveActivityTokenRegistration, LiveActivityTokenStore,
};
use krusty_core::SessionManager;

use super::session_access::ensure_owned_session;
use crate::apns::{ApnsEventType, ApnsPayload, DEFAULT_APNS_BUNDLE_ID};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register_device))
        .route("/register", delete(unregister_device))
        .route("/status", get(status))
        .route("/test", post(send_test))
        .route("/live-activities/register", post(register_live_activity))
        .route("/live-activities/state", post(update_live_activity_state))
        .route(
            "/live-activities/unregister",
            post(unregister_live_activity),
        )
}

#[derive(Deserialize)]
struct RegisterRequest {
    device_token: String,
    bundle_id: Option<String>,
    notification_level: Option<String>,
    environment: Option<String>,
}

#[derive(Serialize)]
struct RegisterResponse {
    id: String,
    registered: bool,
}

async fn register_device(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let db = Database::new(&state.db_path)?;
    let store = ApnsDeviceStore::new(&db);
    let bundle_id = req.bundle_id.as_deref().unwrap_or(DEFAULT_APNS_BUNDLE_ID);
    let notification_level = req.notification_level.as_deref().unwrap_or("important");
    let environment = req.environment.as_deref().unwrap_or("production");
    validate_device_token(&req.device_token)?;
    validate_notification_level(notification_level)?;
    validate_environment(environment)?;
    let service = state
        .apns_service
        .as_ref()
        .ok_or_else(|| AppError::Internal("APNs not configured".into()))?;
    if !service.accepts_registration(bundle_id, environment) {
        return Err(AppError::BadRequest(
            "APNs bundle or environment does not match this server".into(),
        ));
    }
    let id = store.upsert(ApnsDeviceRegistration {
        user_id: user_id.as_deref(),
        device_token: &req.device_token,
        bundle_id,
        notification_level,
        environment,
    })?;
    if let Err(error) =
        ExpoPushDeviceStore::new(&db).remove_platform_for_user(user_id.as_deref(), "ios")
    {
        tracing::warn!(
            user_id = user_id.as_deref().unwrap_or("<single-tenant>"),
            error = %error,
            "Direct APNs registration succeeded but stale iOS Expo registrations could not be removed"
        );
    }
    Ok(Json(RegisterResponse {
        id,
        registered: true,
    }))
}

#[derive(Deserialize)]
struct UnregisterRequest {
    device_token: String,
}

async fn unregister_device(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<UnregisterRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let db = Database::new(&state.db_path)?;
    let store = ApnsDeviceStore::new(&db);
    let removed = store.remove_by_token_for_user(user_id.as_deref(), &req.device_token)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

#[derive(Serialize)]
struct ApnsStatusResponse {
    apns_configured: bool,
    device_count: usize,
    last_success_at: Option<String>,
    last_failure_at: Option<String>,
    last_failure_reason: Option<String>,
}

async fn status(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<ApnsStatusResponse>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let db = Database::new(&state.db_path)?;
    let store = ApnsDeviceStore::new(&db);
    let device_count = store.count_for_user(user_id.as_deref())?;
    let latest = store.get_for_user(user_id.as_deref())?.into_iter().next();

    Ok(Json(ApnsStatusResponse {
        apns_configured: state.apns_service.is_some(),
        device_count,
        last_success_at: latest
            .as_ref()
            .and_then(|device| device.last_success_at.clone()),
        last_failure_at: latest
            .as_ref()
            .and_then(|device| device.last_failure_at.clone()),
        last_failure_reason: latest.and_then(|device| device.last_failure_reason),
    }))
}

#[derive(Deserialize, Default)]
struct TestRequest {
    title: Option<String>,
    body: Option<String>,
    session_id: Option<String>,
}

#[derive(Serialize)]
struct TestResponse {
    accepted: bool,
    attempted: usize,
    sent: usize,
    failed: usize,
    stale_removed: usize,
}

async fn send_test(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<TestRequest>,
) -> Result<Json<TestResponse>, AppError> {
    let apns_service = state
        .apns_service
        .as_ref()
        .cloned()
        .ok_or_else(|| AppError::Internal("APNs not configured".into()))?;

    let user_id = user.and_then(|u| u.0.user_id);
    let title = req.title.unwrap_or_else(|| "Mitsuro".into());
    let body = req
        .body
        .unwrap_or_else(|| "Test notification from Mitsuro".into());

    let stats = apns_service
        .notify_user(
            user_id.as_deref(),
            ApnsPayload {
                title,
                body,
                session_id: req.session_id,
                category: None,
                data: None,
            },
            ApnsEventType::Test,
        )
        .await;

    Ok(Json(TestResponse {
        accepted: stats.sent > 0 && stats.failed == 0,
        attempted: stats.attempted,
        sent: stats.sent,
        failed: stats.failed,
        stale_removed: stats.stale_removed,
    }))
}

fn validate_device_token(token: &str) -> Result<(), AppError> {
    if token.len() < 32 || token.len() > 256 || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::BadRequest("Invalid APNs device token".into()));
    }
    Ok(())
}

fn validate_notification_level(level: &str) -> Result<(), AppError> {
    if matches!(level, "all" | "important" | "silent") {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "notification_level must be all, important, or silent".into(),
        ))
    }
}

fn validate_environment(environment: &str) -> Result<(), AppError> {
    if matches!(environment, "sandbox" | "production") {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "environment must be sandbox or production".into(),
        ))
    }
}

#[derive(Deserialize)]
struct LiveActivityRegisterRequest {
    session_id: String,
    push_token: String,
    bundle_id: Option<String>,
    environment: Option<String>,
    content_state: serde_json::Value,
    started_at_ms: i64,
}

async fn register_live_activity(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<LiveActivityRegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    let user_id = user
        .as_ref()
        .and_then(|current| current.0.user_id.as_deref());
    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    ensure_owned_session(&session_manager, &req.session_id, user.as_ref())?;
    validate_live_activity_token(&req.push_token)?;
    validate_content_state(&req.content_state)?;
    if req.started_at_ms <= 0 {
        return Err(AppError::BadRequest(
            "started_at_ms must be a positive Unix timestamp".into(),
        ));
    }

    let bundle_id = req.bundle_id.as_deref().unwrap_or(DEFAULT_APNS_BUNDLE_ID);
    let environment = req.environment.as_deref().unwrap_or("production");
    validate_environment(environment)?;
    let service = state
        .apns_service
        .as_ref()
        .ok_or_else(|| AppError::Internal("APNs not configured".into()))?;
    if !service.accepts_registration(bundle_id, environment) {
        return Err(AppError::BadRequest(
            "Live Activity bundle or environment does not match this server".into(),
        ));
    }

    let db = Database::new(&state.db_path)?;
    let id = LiveActivityTokenStore::new(&db).upsert(LiveActivityTokenRegistration {
        user_id,
        session_id: &req.session_id,
        push_token: &req.push_token,
        bundle_id,
        environment,
        content_state: &req.content_state,
        started_at_ms: req.started_at_ms,
    })?;
    Ok(Json(RegisterResponse {
        id,
        registered: true,
    }))
}

#[derive(Deserialize)]
struct LiveActivityStateRequest {
    session_id: String,
    push_token: String,
    content_state: serde_json::Value,
}

async fn update_live_activity_state(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<LiveActivityStateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = user
        .as_ref()
        .and_then(|current| current.0.user_id.as_deref());
    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    ensure_owned_session(&session_manager, &req.session_id, user.as_ref())?;
    validate_live_activity_token(&req.push_token)?;
    validate_content_state(&req.content_state)?;

    let db = Database::new(&state.db_path)?;
    let updated = LiveActivityTokenStore::new(&db).update_state_for_user(
        user_id,
        &req.session_id,
        &req.push_token,
        &req.content_state,
    )?;
    Ok(Json(serde_json::json!({ "updated": updated })))
}

#[derive(Deserialize)]
struct LiveActivityUnregisterRequest {
    session_id: String,
    push_token: String,
}

async fn unregister_live_activity(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<LiveActivityUnregisterRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = user
        .as_ref()
        .and_then(|current| current.0.user_id.as_deref());
    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    ensure_owned_session(&session_manager, &req.session_id, user.as_ref())?;
    validate_live_activity_token(&req.push_token)?;

    let db = Database::new(&state.db_path)?;
    let removed =
        LiveActivityTokenStore::new(&db).end_for_user(user_id, &req.session_id, &req.push_token)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

fn validate_live_activity_token(token: &str) -> Result<(), AppError> {
    if token.len() < 32 || token.len() > 512 || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::BadRequest(
            "Invalid Live Activity push token".into(),
        ));
    }
    Ok(())
}

fn validate_content_state(content_state: &serde_json::Value) -> Result<(), AppError> {
    if !content_state.is_object() {
        return Err(AppError::BadRequest(
            "content_state must be a JSON object".into(),
        ));
    }
    let encoded = serde_json::to_vec(content_state)
        .map_err(|error| AppError::BadRequest(format!("Invalid content_state: {error}")))?;
    if encoded.len() > 3_000 {
        return Err(AppError::BadRequest("content_state is too large".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_content_state, validate_live_activity_token};

    #[test]
    fn live_activity_registration_rejects_malformed_tokens_and_state() {
        assert!(validate_live_activity_token(&"a".repeat(64)).is_ok());
        assert!(validate_live_activity_token("not-a-hex-token").is_err());
        assert!(validate_content_state(&serde_json::json!({
            "status": "working",
            "toolCount": 0,
        }))
        .is_ok());
        assert!(validate_content_state(&serde_json::json!(["not", "an", "object"])).is_err());
    }
}
