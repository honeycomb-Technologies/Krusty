//! Push notification subscription endpoints

use axum::{
    extract::State,
    routing::{delete, get, post},
    Json, Router,
};
use base64ct::{Base64UrlUnpadded, Encoding};
use serde::{Deserialize, Serialize};
use web_push_native::p256::PublicKey;

use krusty_core::storage::{Database, PushDeliveryAttemptStore, PushSubscriptionStore};

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::push::{normalize_push_endpoint, PushEventType, PushPayload};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vapid-public-key", get(vapid_public_key))
        .route("/status", get(status))
        .route("/test", post(send_test_notification))
        .route("/subscribe", post(subscribe))
        .route("/subscribe", delete(unsubscribe))
}

#[derive(Serialize)]
struct VapidKeyResponse {
    public_key: String,
}

async fn vapid_public_key(
    State(state): State<AppState>,
) -> Result<Json<VapidKeyResponse>, AppError> {
    let push_service = state
        .push_service
        .as_ref()
        .ok_or_else(|| AppError::Internal("Push notifications not configured".into()))?;

    Ok(Json(VapidKeyResponse {
        public_key: push_service.vapid_public_key_base64url().to_string(),
    }))
}

#[derive(Serialize)]
struct PushStatusResponse {
    push_configured: bool,
    subscription_count: usize,
    last_attempt_at: Option<String>,
    last_success_at: Option<String>,
    last_failure_at: Option<String>,
    last_failure_reason: Option<String>,
    recent_failures_24h: usize,
}

async fn status(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<PushStatusResponse>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let db = Database::new(&state.db_path)?;
    let sub_store = PushSubscriptionStore::new(&db);
    let attempts = PushDeliveryAttemptStore::new(&db);
    let summary = attempts.summary_for_user(user_id.as_deref())?;
    let subscription_count = sub_store.count_for_user(user_id.as_deref())?;

    Ok(Json(PushStatusResponse {
        push_configured: state.push_service.is_some(),
        subscription_count,
        last_attempt_at: summary.last_attempt_at,
        last_success_at: summary.last_success_at,
        last_failure_at: summary.last_failure_at,
        last_failure_reason: summary.last_failure_reason,
        recent_failures_24h: summary.recent_failures_24h,
    }))
}

#[derive(Deserialize, Default)]
struct PushTestRequest {
    session_id: Option<String>,
    title: Option<String>,
    body: Option<String>,
}

#[derive(Serialize)]
struct PushTestResponse {
    accepted: bool,
    attempted: usize,
    sent: usize,
    stale_removed: usize,
    failed: usize,
}

async fn send_test_notification(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<PushTestRequest>,
) -> Result<Json<PushTestResponse>, AppError> {
    let push_service = state
        .push_service
        .as_ref()
        .cloned()
        .ok_or_else(|| AppError::Internal("Push notifications not configured".into()))?;

    let user_id = user.and_then(|u| u.0.user_id);
    let session_id = req.session_id;
    let title = req
        .title
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "Krusty".to_string());
    let body = req
        .body
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "Test notification from Krusty".to_string());

    let stats = push_service
        .notify_user(
            user_id.as_deref(),
            PushPayload {
                title,
                body,
                session_id: session_id.clone(),
                tag: Some(
                    session_id
                        .map(|id| format!("session-{id}"))
                        .unwrap_or_else(|| "push-test".to_string()),
                ),
            },
            PushEventType::Test,
        )
        .await;

    Ok(Json(PushTestResponse {
        accepted: stats.attempted > 0,
        attempted: stats.attempted,
        sent: stats.sent,
        stale_removed: stats.stale_removed,
        failed: stats.failed,
    }))
}

#[derive(Deserialize)]
struct SubscribeRequest {
    endpoint: String,
    p256dh: String,
    auth: String,
}

#[derive(Serialize)]
struct SubscribeResponse {
    id: String,
}

fn normalize_endpoint(endpoint: &str) -> Result<String, AppError> {
    normalize_push_endpoint(endpoint).map_err(|err| AppError::BadRequest(err.to_string()))
}

fn normalize_auth_secret(auth: &str) -> Result<String, AppError> {
    let auth = auth.trim();
    if auth.is_empty() {
        return Err(AppError::BadRequest(
            "Push auth secret is required".to_string(),
        ));
    }

    let decoded = Base64UrlUnpadded::decode_vec(auth).map_err(|_| {
        AppError::BadRequest("Push auth secret must be base64url encoded".to_string())
    })?;

    if decoded.len() != 16 {
        return Err(AppError::BadRequest(
            "Push auth secret must decode to 16 bytes".to_string(),
        ));
    }

    Ok(auth.to_string())
}

fn normalize_p256dh_key(p256dh: &str) -> Result<String, AppError> {
    let p256dh = p256dh.trim();
    if p256dh.is_empty() {
        return Err(AppError::BadRequest(
            "Push p256dh key is required".to_string(),
        ));
    }

    let decoded = Base64UrlUnpadded::decode_vec(p256dh).map_err(|_| {
        AppError::BadRequest("Push p256dh key must be base64url encoded".to_string())
    })?;

    PublicKey::from_sec1_bytes(&decoded).map_err(|_| {
        AppError::BadRequest("Push p256dh key must be a valid P-256 public key".to_string())
    })?;

    Ok(p256dh.to_string())
}

async fn subscribe(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<SubscribeRequest>,
) -> Result<Json<SubscribeResponse>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let endpoint = normalize_endpoint(&req.endpoint)?;
    let p256dh = normalize_p256dh_key(&req.p256dh)?;
    let auth = normalize_auth_secret(&req.auth)?;

    let db = Database::new(&state.db_path)?;
    let store = PushSubscriptionStore::new(&db);
    let id = store.upsert(user_id.as_deref(), &endpoint, &p256dh, &auth)?;
    Ok(Json(SubscribeResponse { id }))
}

#[derive(Deserialize)]
struct UnsubscribeRequest {
    endpoint: String,
}

async fn unsubscribe(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<UnsubscribeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let endpoint = normalize_endpoint(&req.endpoint)?;

    let db = Database::new(&state.db_path)?;
    let store = PushSubscriptionStore::new(&db);
    let removed = store.remove_by_endpoint_for_user(user_id.as_deref(), &endpoint)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

#[cfg(test)]
mod tests {
    use super::{normalize_auth_secret, normalize_endpoint, normalize_p256dh_key};
    use base64ct::{Base64UrlUnpadded, Encoding};
    use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};

    fn valid_p256dh_key() -> String {
        let keypair = ES256KeyPair::generate();
        let public_key_bytes = keypair.public_key().public_key().to_bytes_uncompressed();
        Base64UrlUnpadded::encode_string(&public_key_bytes)
    }

    #[test]
    fn subscribe_validation_rejects_non_https_endpoints() {
        let error = normalize_endpoint("http://fcm.googleapis.com/subscription")
            .expect_err("http endpoint should be rejected");
        assert!(matches!(error, super::AppError::BadRequest(_)));
    }

    #[test]
    fn subscribe_validation_rejects_loopback_and_untrusted_hosts() {
        for endpoint in [
            "https://127.0.0.1/internal",
            "https://localhost/internal",
            "https://192.168.0.10/internal",
            "https://push.example.test/subscription",
        ] {
            let error = normalize_endpoint(endpoint)
                .expect_err("unsupported push endpoint host should be rejected");
            assert!(matches!(error, super::AppError::BadRequest(_)));
        }
    }

    #[test]
    fn subscribe_validation_rejects_invalid_auth_secret() {
        let error = normalize_auth_secret("not-base64")
            .expect_err("invalid auth secret should be rejected");
        assert!(matches!(error, super::AppError::BadRequest(_)));
    }

    #[test]
    fn subscribe_validation_accepts_valid_push_keys() {
        let auth = Base64UrlUnpadded::encode_string(b"abcdefghijklmnop");
        assert!(normalize_endpoint("https://fcm.googleapis.com/fcm/send/test").is_ok());
        assert!(normalize_auth_secret(&auth).is_ok());
        assert!(normalize_p256dh_key(&valid_p256dh_key()).is_ok());
    }
}
