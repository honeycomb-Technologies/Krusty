//! Apple Push Notification service (APNs)
//!
//! Sends push notifications to iOS devices via the APNs HTTP/2 API.
//! Uses JWT (ES256) token-based authentication with a .p8 private key.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use futures::StreamExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use mitsuro_core::storage::{
    hash_request_bytes, ApnsDevice, ApnsDeviceStore, Database, LiveActivityToken,
    LiveActivityTokenStore, NotificationIntentStore, PushDeliveryAttemptInput,
    PushDeliveryAttemptStore,
};

const APNS_PRODUCTION_URL: &str = "https://api.push.apple.com";
const APNS_SANDBOX_URL: &str = "https://api.sandbox.push.apple.com";
pub(crate) const DEFAULT_APNS_BUNDLE_ID: &str = "io.mitsuro.mobile";

/// JWT tokens are valid for up to 60 minutes. We refresh at 50 min.
const JWT_REFRESH_INTERVAL: Duration = Duration::from_secs(50 * 60);

const MAX_APNS_ATTEMPTS: usize = 3;
const APNS_RETRY_BASE_DELAY_MS: u64 = 1_000;
const MAX_STALE_FAILURES: i64 = 10;
const MAX_APNS_ERROR_BODY_BYTES: usize = 4 * 1024;
const APNS_ERROR_BODY_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_APNS_PAYLOAD_BYTES: usize = 4 * 1024;
const MAX_APNS_TITLE_CHARS: usize = 160;
const MAX_APNS_BODY_CHARS: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApnsPayload {
    pub title: String,
    pub body: String,
    pub session_id: Option<String>,
    pub category: Option<String>,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy)]
pub enum ApnsEventType {
    ToolApproval,
    Completion,
    AwaitingInput,
    HiveUpdate,
    Error,
    Test,
}

impl ApnsEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolApproval => "tool_approval",
            Self::Completion => "completion",
            Self::AwaitingInput => "awaiting_input",
            Self::HiveUpdate => "hive_update",
            Self::Error => "error",
            Self::Test => "test",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "tool_approval" => Self::ToolApproval,
            "completion" => Self::Completion,
            "awaiting_input" => Self::AwaitingInput,
            "hive_update" => Self::HiveUpdate,
            value if value == crate::legacy_identity::HIVE_PUSH_EVENT => Self::HiveUpdate,
            "error" => Self::Error,
            "test" => Self::Test,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ApnsStats {
    pub attempted: usize,
    pub sent: usize,
    pub failed: usize,
    pub stale_removed: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    iss: String,
    iat: u64,
}

struct CachedToken {
    token: String,
    created_at: Instant,
}

#[derive(Debug)]
struct ApnsAccepted {
    apns_id: Option<String>,
}

#[derive(Debug)]
struct ApnsSendError {
    status: Option<StatusCode>,
    reason: Option<&'static str>,
    summary: String,
}

impl ApnsSendError {
    fn transport(error: impl std::fmt::Display) -> Self {
        Self {
            status: None,
            reason: None,
            summary: format!("APNs request failed ({error})"),
        }
    }

    fn retryable(&self) -> bool {
        self.status.is_none()
            || self.status == Some(StatusCode::TOO_MANY_REQUESTS)
            || self.status.is_some_and(|status| status.is_server_error())
            || matches!(
                self.reason,
                Some("ExpiredProviderToken" | "TooManyRequests")
            )
    }

    fn invalidates_device(&self) -> bool {
        matches!(
            self.reason,
            Some("BadDeviceToken" | "DeviceTokenNotForTopic" | "ExpiredToken" | "Unregistered")
        )
    }
}

/// APNs configuration loaded from environment or config file.
#[derive(Clone)]
pub struct ApnsConfig {
    pub key_id: String,
    pub team_id: String,
    pub bundle_id: String,
    pub sandbox: bool,
    encoding_key: EncodingKey,
}

pub struct ApnsService {
    config: ApnsConfig,
    client: Client,
    db_path: Arc<PathBuf>,
    cached_token: RwLock<Option<CachedToken>>,
}

impl ApnsService {
    /// Initialize the APNs service from a .p8 key file.
    pub fn init(
        key_path: &PathBuf,
        key_id: &str,
        team_id: &str,
        bundle_id: &str,
        sandbox: bool,
        db_path: Arc<PathBuf>,
    ) -> Result<Self> {
        let key_data = std::fs::read(key_path).context("Failed to read APNs .p8 key file")?;

        let encoding_key = EncodingKey::from_ec_pem(&key_data)
            .context("Failed to parse APNs .p8 key as EC private key")?;

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            // APNs is HTTP/2-only. Do not allow reqwest to fall back to HTTP/1.1
            // when ALPN negotiation is unavailable or inconclusive.
            .http2_prior_knowledge()
            .build()
            .context("Failed to build HTTP client for APNs")?;

        info!(
            key_id,
            team_id, bundle_id, sandbox, "APNs service initialized"
        );

        Ok(Self {
            config: ApnsConfig {
                key_id: key_id.to_string(),
                team_id: team_id.to_string(),
                bundle_id: bundle_id.to_string(),
                sandbox,
                encoding_key,
            },
            client,
            db_path,
            cached_token: RwLock::new(None),
        })
    }

    /// Try to initialize from environment variables. Returns None if not configured.
    pub fn from_env(db_path: Arc<PathBuf>) -> Option<Self> {
        let key_path = mitsuro_core::identity::env_var("MITSURO_APNS_KEY_PATH").ok()?;
        let key_id = mitsuro_core::identity::env_var("MITSURO_APNS_KEY_ID").ok()?;
        let team_id = mitsuro_core::identity::env_var("MITSURO_APNS_TEAM_ID").ok()?;
        let bundle_id = mitsuro_core::identity::env_var("MITSURO_APNS_BUNDLE_ID")
            .unwrap_or_else(|_| DEFAULT_APNS_BUNDLE_ID.into());
        let sandbox = mitsuro_core::identity::env_var("MITSURO_APNS_SANDBOX")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

        match Self::init(
            &PathBuf::from(key_path),
            &key_id,
            &team_id,
            &bundle_id,
            sandbox,
            db_path,
        ) {
            Ok(svc) => Some(svc),
            Err(e) => {
                warn!("APNs initialization failed: {e:#}");
                None
            }
        }
    }

    /// Get or refresh the JWT bearer token.
    async fn get_token(&self) -> Result<String> {
        // Check cache
        {
            let cached = self.cached_token.read().await;
            if let Some(ref ct) = *cached {
                if ct.created_at.elapsed() < JWT_REFRESH_INTERVAL {
                    return Ok(ct.token.clone());
                }
            }
        }

        // Generate new token
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = JwtClaims {
            iss: self.config.team_id.clone(),
            iat: now,
        };

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());

        let token = encode(&header, &claims, &self.config.encoding_key)
            .context("Failed to sign APNs JWT")?;

        // Cache it
        {
            let mut cached = self.cached_token.write().await;
            *cached = Some(CachedToken {
                token: token.clone(),
                created_at: Instant::now(),
            });
        }

        Ok(token)
    }

    async fn invalidate_cached_token(&self) {
        *self.cached_token.write().await = None;
    }

    fn base_url(&self) -> &str {
        if self.config.sandbox {
            APNS_SANDBOX_URL
        } else {
            APNS_PRODUCTION_URL
        }
    }

    pub fn accepts_registration(&self, bundle_id: &str, environment: &str) -> bool {
        supported_apns_bundle_id(&self.config.bundle_id, bundle_id)
            && environment
                == if self.config.sandbox {
                    "sandbox"
                } else {
                    "production"
                }
    }

    /// Send a push notification to a single device.
    async fn send_to_device(
        &self,
        device_token: &str,
        bundle_id: &str,
        payload: &ApnsPayload,
        event_type: ApnsEventType,
        play_sound: bool,
    ) -> std::result::Result<ApnsAccepted, ApnsSendError> {
        let token = self.get_token().await.map_err(ApnsSendError::transport)?;
        let url = format!("{}/3/device/{}", self.base_url(), device_token);

        // Build the APNs JSON payload
        let apns_payload = build_apns_json(payload, bundle_id, event_type, play_sound);
        let encoded_payload =
            serde_json::to_vec(&apns_payload).map_err(ApnsSendError::transport)?;
        if encoded_payload.len() > MAX_APNS_PAYLOAD_BYTES {
            return Err(ApnsSendError {
                status: Some(StatusCode::PAYLOAD_TOO_LARGE),
                reason: Some("PayloadTooLarge"),
                summary: format!(
                    "APNs payload exceeds {} bytes after normalization",
                    MAX_APNS_PAYLOAD_BYTES
                ),
            });
        }

        let topic = apns_topic(bundle_id, event_type);
        let push_type = apns_push_type(event_type);

        let mut request = self
            .client
            .post(&url)
            .header("authorization", format!("bearer {token}"))
            .header("apns-topic", &topic)
            .header("apns-push-type", push_type)
            .header("apns-priority", apns_priority(event_type))
            .header("apns-expiration", apns_expiration(event_type).to_string());
        if let Some(collapse_id) = apns_collapse_id(payload, event_type) {
            request = request.header("apns-collapse-id", collapse_id);
        }
        let resp = request
            .body(encoded_payload)
            .header("content-type", "application/json")
            .send()
            .await
            .map_err(ApnsSendError::transport)?;

        let status = resp.status();
        if status.is_success() {
            let apns_id = resp
                .headers()
                .get("apns-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            debug!(
                device_token_hash = %device_token_fingerprint(device_token),
                apns_id = ?apns_id,
                "APNs delivery succeeded"
            );
            Ok(ApnsAccepted { apns_id })
        } else {
            Err(parse_apns_failure(status, resp).await)
        }
    }

    fn load_devices_for_user(&self, user_id: Option<&str>) -> Result<Vec<ApnsDevice>> {
        let db =
            Database::new(&self.db_path).context("Failed to open DB for APNs device lookup")?;
        let store = ApnsDeviceStore::new(&db);
        store.get_for_user(user_id)
    }

    fn load_live_activities(
        &self,
        user_id: Option<&str>,
        session_id: &str,
    ) -> Result<Vec<LiveActivityToken>> {
        let db = Database::new(&self.db_path)
            .context("Failed to open DB for Live Activity token lookup")?;
        LiveActivityTokenStore::new(&db).active_for_session(user_id, session_id)
    }

    fn end_live_activity_token(&self, push_token: &str) {
        if let Ok(db) = Database::new(&self.db_path) {
            let _ = LiveActivityTokenStore::new(&db).end_token(push_token);
        }
    }

    fn persist_live_activity_state(
        &self,
        activity: &LiveActivityToken,
        content_state: &serde_json::Value,
    ) {
        if let Ok(db) = Database::new(&self.db_path) {
            let _ = LiveActivityTokenStore::new(&db).update_state_for_user(
                activity.user_id.as_deref(),
                &activity.session_id,
                &activity.push_token,
                content_state,
            );
        }
    }

    fn mark_device_success(&self, device_token: &str) -> Result<()> {
        let db =
            Database::new(&self.db_path).context("Failed to open DB for APNs success update")?;
        let store = ApnsDeviceStore::new(&db);
        store.mark_success(device_token)
    }

    fn mark_device_failure(&self, device_token: &str, reason: &str) -> Result<()> {
        let db =
            Database::new(&self.db_path).context("Failed to open DB for APNs failure update")?;
        let store = ApnsDeviceStore::new(&db);
        store.mark_failure(device_token, reason)
    }

    fn remove_stale_devices(&self, max_failures: i64) -> Result<usize> {
        let db =
            Database::new(&self.db_path).context("Failed to open DB for APNs stale cleanup")?;
        let store = ApnsDeviceStore::new(&db);
        store.remove_stale(max_failures)
    }

    fn remove_device(&self, device: &ApnsDevice) -> Result<bool> {
        let db =
            Database::new(&self.db_path).context("Failed to open DB for APNs token removal")?;
        let store = ApnsDeviceStore::new(&db);
        store.remove_by_token_for_user(device.user_id.as_deref(), &device.device_token)
    }

    fn record_attempt(
        &self,
        user_id: Option<&str>,
        payload: &ApnsPayload,
        device_token: &str,
        event_type: ApnsEventType,
        outcome: &str,
        status: Option<StatusCode>,
        error_message: Option<&str>,
        latency: Duration,
    ) {
        let Ok(db) = Database::new(&self.db_path) else {
            return;
        };
        let endpoint = format!(
            "{}/3/device/sha256:{}",
            self.base_url(),
            device_token_fingerprint(device_token)
        );
        let _ = PushDeliveryAttemptStore::new(&db).record_attempt(PushDeliveryAttemptInput {
            user_id,
            session_id: payload.session_id.as_deref(),
            endpoint: &endpoint,
            event_type: event_type.as_str(),
            outcome,
            http_status: status.map(|value| value.as_u16()),
            error_message,
            latency_ms: Some(latency.as_millis() as u64),
        });
    }

    /// Send to all devices for a user, with retries.
    pub async fn notify_user(
        &self,
        user_id: Option<&str>,
        payload: ApnsPayload,
        event_type: ApnsEventType,
    ) -> ApnsStats {
        let devices = match self.load_devices_for_user(user_id) {
            Ok(devices) => devices,
            Err(e) => {
                error!("Failed to load APNs devices: {e:#}");
                return ApnsStats {
                    attempted: 0,
                    sent: 0,
                    failed: 0,
                    stale_removed: 0,
                };
            }
        };

        let mut stats = ApnsStats {
            attempted: 0,
            sent: 0,
            failed: 0,
            stale_removed: 0,
        };

        for device in &devices {
            if !self.accepts_registration(&device.bundle_id, &device.environment)
                || !device_allows_event(&device.notification_level, event_type)
            {
                continue;
            }
            stats.attempted += 1;
            let mut delivered = false;

            for attempt in 0..MAX_APNS_ATTEMPTS {
                let started_at = Instant::now();
                match self
                    .send_to_device(
                        &device.device_token,
                        &device.bundle_id,
                        &payload,
                        event_type,
                        device_plays_sound(&device.notification_level, event_type),
                    )
                    .await
                {
                    Ok(accepted) => {
                        self.record_attempt(
                            user_id,
                            &payload,
                            &device.device_token,
                            event_type,
                            "success",
                            Some(StatusCode::OK),
                            None,
                            started_at.elapsed(),
                        );
                        let _ = self.mark_device_success(&device.device_token);
                        stats.sent += 1;
                        delivered = true;
                        debug!(
                            event_type = event_type.as_str(),
                            apns_id = ?accepted.apns_id,
                            "APNs request accepted"
                        );
                        break;
                    }
                    Err(error) => {
                        if error.reason == Some("ExpiredProviderToken") {
                            self.invalidate_cached_token().await;
                        }
                        self.record_attempt(
                            user_id,
                            &payload,
                            &device.device_token,
                            event_type,
                            "failure",
                            error.status,
                            Some(&error.summary),
                            started_at.elapsed(),
                        );
                        if error.invalidates_device() {
                            if self.remove_device(device).unwrap_or(false) {
                                stats.stale_removed += 1;
                            }
                            warn!(
                                device_token_hash = %device_token_fingerprint(&device.device_token),
                                error = %error.summary,
                                "Removed permanently invalid APNs device"
                            );
                            break;
                        }

                        if error.retryable() && attempt < MAX_APNS_ATTEMPTS - 1 {
                            let delay = APNS_RETRY_BASE_DELAY_MS * 2u64.pow(attempt as u32);
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        } else {
                            warn!(
                                device_token_hash = %device_token_fingerprint(&device.device_token),
                                error = %error.summary,
                                attempts = attempt + 1,
                                "APNs delivery failed"
                            );
                            if error.status.is_none()
                                || error.status == Some(StatusCode::TOO_MANY_REQUESTS)
                                || error.status.is_some_and(|status| status.is_server_error())
                            {
                                let _ =
                                    self.mark_device_failure(&device.device_token, &error.summary);
                            }
                            break;
                        }
                    }
                }
            }

            if !delivered {
                stats.failed += 1;
            }
        }

        // Clean up stale devices
        if let Ok(removed) = self.remove_stale_devices(MAX_STALE_FAILURES) {
            stats.stale_removed += removed;
        }

        stats
    }

    pub async fn notify_user_durable(
        &self,
        user_id: Option<&str>,
        payload: ApnsPayload,
        event_type: ApnsEventType,
    ) -> ApnsStats {
        let intent_id = Database::new(&self.db_path).ok().and_then(|db| {
            let payload_json = serde_json::to_value(&payload).ok()?;
            NotificationIntentStore::new(&db)
                .enqueue(
                    "apns",
                    user_id,
                    payload.session_id.as_deref(),
                    event_type.as_str(),
                    &payload_json,
                    apns_ttl_seconds(event_type) as i64,
                )
                .ok()
        });
        if let Some(id) = intent_id.as_deref() {
            self.mark_intent_dispatching(id);
        }
        let stats = self.notify_user(user_id, payload, event_type).await;
        if let Some(id) = intent_id.as_deref() {
            self.finish_intent(id, &stats);
        }
        stats
    }

    pub async fn recover_notification_intents(&self) {
        let intents = Database::new(&self.db_path)
            .and_then(|db| NotificationIntentStore::new(&db).recoverable("apns", 100))
            .unwrap_or_default();
        for intent in intents {
            let Some(event_name) = intent.event_type.strip_prefix("apns:") else {
                continue;
            };
            let Some(event_type) = ApnsEventType::from_str(event_name) else {
                self.mark_intent_cancelled(&intent.id, "unknown APNs event type");
                continue;
            };
            let Ok(payload) = serde_json::from_value::<ApnsPayload>(intent.payload) else {
                self.mark_intent_cancelled(&intent.id, "invalid APNs intent payload");
                continue;
            };
            self.mark_intent_dispatching(&intent.id);
            let stats = self
                .notify_user(intent.user_id.as_deref(), payload, event_type)
                .await;
            self.finish_intent(&intent.id, &stats);
        }
    }

    fn mark_intent_dispatching(&self, id: &str) {
        if let Ok(db) = Database::new(&self.db_path) {
            let _ = NotificationIntentStore::new(&db).mark_dispatching(id);
        }
    }

    fn mark_intent_cancelled(&self, id: &str, reason: &str) {
        if let Ok(db) = Database::new(&self.db_path) {
            let _ = NotificationIntentStore::new(&db).mark_cancelled(id, reason);
        }
    }

    fn finish_intent(&self, id: &str, stats: &ApnsStats) {
        if let Ok(db) = Database::new(&self.db_path) {
            let store = NotificationIntentStore::new(&db);
            if stats.sent > 0 && stats.failed == 0 {
                let _ = store.mark_accepted(id);
            } else if stats.attempted == 0 {
                let _ = store.mark_cancelled(id, "no eligible APNs devices");
            } else {
                let _ = store.mark_failed(
                    id,
                    &format!(
                        "APNs delivery failed: attempted={}, sent={}, failed={}",
                        stats.attempted, stats.sent, stats.failed
                    ),
                );
            }
        }
    }

    pub async fn notify_live_activities(
        &self,
        user_id: Option<&str>,
        payload: &ApnsPayload,
        event_type: ApnsEventType,
    ) -> ApnsStats {
        let Some(session_id) = payload.session_id.as_deref() else {
            return empty_apns_stats();
        };
        let activities = match self.load_live_activities(user_id, session_id) {
            Ok(activities) => activities,
            Err(error) => {
                warn!(
                    session_id,
                    error = %error,
                    "Failed to load Live Activity tokens"
                );
                return empty_apns_stats();
            }
        };
        let (content_state, should_end) =
            live_activity_content_state(payload, event_type, &activities);
        let mut stats = empty_apns_stats();

        for activity in activities {
            if !self.accepts_registration(&activity.bundle_id, &activity.environment) {
                continue;
            }
            stats.attempted += 1;
            let mut delivered = false;
            for attempt in 0..MAX_APNS_ATTEMPTS {
                let started_at = Instant::now();
                let result = self
                    .send_live_activity_update(&activity, &content_state, should_end)
                    .await;
                match result {
                    Ok(accepted) => {
                        self.record_attempt(
                            user_id,
                            payload,
                            &activity.push_token,
                            event_type,
                            if should_end {
                                "live_activity_end"
                            } else {
                                "live_activity_update"
                            },
                            Some(StatusCode::OK),
                            None,
                            started_at.elapsed(),
                        );
                        if should_end {
                            self.end_live_activity_token(&activity.push_token);
                        } else {
                            self.persist_live_activity_state(&activity, &content_state);
                        }
                        debug!(
                            session_id,
                            apns_id = ?accepted.apns_id,
                            should_end,
                            "Live Activity request accepted"
                        );
                        stats.sent += 1;
                        delivered = true;
                        break;
                    }
                    Err(error) => {
                        if error.reason == Some("ExpiredProviderToken") {
                            self.invalidate_cached_token().await;
                        }
                        self.record_attempt(
                            user_id,
                            payload,
                            &activity.push_token,
                            event_type,
                            "live_activity_failure",
                            error.status,
                            Some(&error.summary),
                            started_at.elapsed(),
                        );
                        if error.invalidates_device() {
                            self.end_live_activity_token(&activity.push_token);
                            stats.stale_removed += 1;
                            break;
                        }
                        if error.retryable() && attempt < MAX_APNS_ATTEMPTS - 1 {
                            let delay = APNS_RETRY_BASE_DELAY_MS * 2u64.pow(attempt as u32);
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                            continue;
                        }
                        warn!(
                            session_id,
                            error = %error.summary,
                            attempts = attempt + 1,
                            "Live Activity delivery failed"
                        );
                        break;
                    }
                }
            }
            if !delivered {
                stats.failed += 1;
            }
        }

        stats
    }

    async fn send_live_activity_update(
        &self,
        activity: &LiveActivityToken,
        content_state: &serde_json::Value,
        should_end: bool,
    ) -> std::result::Result<ApnsAccepted, ApnsSendError> {
        let token = self.get_token().await.map_err(ApnsSendError::transport)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut aps = serde_json::json!({
            "timestamp": timestamp,
            "event": if should_end { "end" } else { "update" },
            "content-state": content_state,
        });
        if should_end {
            aps["dismissal-date"] = serde_json::Value::from(timestamp + 60);
        } else {
            aps["stale-date"] = serde_json::Value::from(timestamp + 5 * 60);
        }
        let body = serde_json::to_vec(&serde_json::json!({ "aps": aps }))
            .map_err(ApnsSendError::transport)?;
        if body.len() > MAX_APNS_PAYLOAD_BYTES {
            return Err(ApnsSendError {
                status: Some(StatusCode::PAYLOAD_TOO_LARGE),
                reason: Some("PayloadTooLarge"),
                summary: "Live Activity payload exceeds APNs size limit".into(),
            });
        }
        let url = format!("{}/3/device/{}", self.base_url(), activity.push_token);
        let response = self
            .client
            .post(url)
            .header("authorization", format!("bearer {token}"))
            .header(
                "apns-topic",
                format!("{}.push-type.liveactivity", activity.bundle_id),
            )
            .header("apns-push-type", "liveactivity")
            .header("apns-priority", "10")
            .header("apns-expiration", (timestamp + 15 * 60).to_string())
            .body(body)
            .header("content-type", "application/json")
            .send()
            .await
            .map_err(ApnsSendError::transport)?;
        let status = response.status();
        if status.is_success() {
            Ok(ApnsAccepted {
                apns_id: response
                    .headers()
                    .get("apns-id")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            })
        } else {
            Err(parse_apns_failure(status, response).await)
        }
    }
}

fn empty_apns_stats() -> ApnsStats {
    ApnsStats {
        attempted: 0,
        sent: 0,
        failed: 0,
        stale_removed: 0,
    }
}

fn live_activity_content_state(
    payload: &ApnsPayload,
    event_type: ApnsEventType,
    activities: &[LiveActivityToken],
) -> (serde_json::Value, bool) {
    let mut content_state = activities
        .first()
        .map(|activity| activity.content_state.clone())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let data = payload.data.as_ref();
    let kind = data
        .and_then(|value| value.get("kind"))
        .and_then(serde_json::Value::as_str);
    let should_end = matches!(event_type, ApnsEventType::Completion | ApnsEventType::Error)
        || matches!(kind, Some("completion" | "error"));
    let needs_input = matches!(
        event_type,
        ApnsEventType::ToolApproval | ApnsEventType::AwaitingInput | ApnsEventType::Error
    ) || matches!(kind, Some("awaiting_input" | "error"));

    if let Some(object) = content_state.as_object_mut() {
        object.insert(
            "status".into(),
            serde_json::Value::String(
                if should_end && !needs_input {
                    "completed"
                } else if needs_input {
                    "needs_input"
                } else {
                    "working"
                }
                .into(),
            ),
        );
        if let Some(started_at_ms) = activities.first().map(|activity| activity.started_at_ms) {
            let elapsed = chrono::Utc::now()
                .timestamp_millis()
                .saturating_sub(started_at_ms)
                / 1000;
            object.insert(
                "elapsedSeconds".into(),
                serde_json::Value::from(elapsed.max(0)),
            );
        }
        for key in [
            "toolApprovalId",
            "toolApprovalName",
            "toolApprovalSessionId",
        ] {
            object.remove(key);
        }
        if matches!(event_type, ApnsEventType::ToolApproval) {
            for (source, target) in [
                ("requestId", "toolApprovalId"),
                ("toolName", "toolApprovalName"),
                ("sessionId", "toolApprovalSessionId"),
            ] {
                if let Some(value) = data.and_then(|value| value.get(source)).cloned() {
                    object.insert(target.into(), value);
                }
            }
        }
    }

    (content_state, should_end)
}

async fn parse_apns_failure(status: StatusCode, response: Response) -> ApnsSendError {
    match tokio::time::timeout(APNS_ERROR_BODY_TIMEOUT, read_capped_apns_body(response)).await {
        Ok(Ok(body)) => ApnsSendError {
            status: Some(status),
            reason: parsed_apns_reason(&body.bytes),
            summary: summarize_apns_failure_body(status, &body.bytes, body.truncated),
        },
        Ok(Err(())) => ApnsSendError {
            status: Some(status),
            reason: None,
            summary: format!(
                "APNs rejected request (status={}, reason=unknown, body_state=read_failed)",
                status.as_u16()
            ),
        },
        Err(_) => ApnsSendError {
            status: Some(status),
            reason: None,
            summary: format!(
                "APNs rejected request (status={}, reason=unknown, body_state=timeout)",
                status.as_u16()
            ),
        },
    }
}

struct CappedApnsBody {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_capped_apns_body(response: Response) -> std::result::Result<CappedApnsBody, ()> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::with_capacity(MAX_APNS_ERROR_BODY_BYTES.min(512));
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        let remaining = MAX_APNS_ERROR_BODY_BYTES.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() == MAX_APNS_ERROR_BODY_BYTES {
            // Read at most one subsequent frame to distinguish an exact-size
            // response from a truncated one. The outer timeout bounds a peer
            // that stalls instead of ending the body.
            if let Some(next) = stream.next().await {
                let next = next.map_err(|_| ())?;
                truncated = !next.is_empty();
            }
            break;
        }
    }
    Ok(CappedApnsBody { bytes, truncated })
}

fn summarize_apns_failure_body(status: StatusCode, body: &[u8], truncated: bool) -> String {
    let known_reason = parsed_apns_reason(body);
    if let Some(reason) = known_reason {
        return format!(
            "APNs rejected request (status={}, reason={reason})",
            status.as_u16()
        );
    }

    format!(
        "APNs rejected request (status={}, reason=unknown, body_sha256={}, body_truncated={truncated})",
        status.as_u16(),
        hash_request_bytes(body),
    )
}

fn parsed_apns_reason(body: &[u8]) -> Option<&'static str> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|payload| {
            payload
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .and_then(allowlisted_apns_reason)
        })
}

fn allowlisted_apns_reason(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "BadCollapseId" => "BadCollapseId",
        "BadDeviceToken" => "BadDeviceToken",
        "BadExpirationDate" => "BadExpirationDate",
        "BadMessageId" => "BadMessageId",
        "BadPriority" => "BadPriority",
        "BadTopic" => "BadTopic",
        "DeviceTokenNotForTopic" => "DeviceTokenNotForTopic",
        "DuplicateHeaders" => "DuplicateHeaders",
        "IdleTimeout" => "IdleTimeout",
        "InvalidPushType" => "InvalidPushType",
        "MissingDeviceToken" => "MissingDeviceToken",
        "MissingTopic" => "MissingTopic",
        "PayloadEmpty" => "PayloadEmpty",
        "TopicDisallowed" => "TopicDisallowed",
        "BadCertificate" => "BadCertificate",
        "BadCertificateEnvironment" => "BadCertificateEnvironment",
        "ExpiredProviderToken" => "ExpiredProviderToken",
        "ExpiredToken" => "ExpiredToken",
        "Forbidden" => "Forbidden",
        "InvalidProviderToken" => "InvalidProviderToken",
        "MissingProviderToken" => "MissingProviderToken",
        "BadPath" => "BadPath",
        "MethodNotAllowed" => "MethodNotAllowed",
        "Unregistered" => "Unregistered",
        "PayloadTooLarge" => "PayloadTooLarge",
        "TooManyProviderTokenUpdates" => "TooManyProviderTokenUpdates",
        "TooManyRequests" => "TooManyRequests",
        "InternalServerError" => "InternalServerError",
        "ServiceUnavailable" => "ServiceUnavailable",
        "Shutdown" => "Shutdown",
        _ => return None,
    })
}

fn device_token_fingerprint(device_token: &str) -> String {
    hash_request_bytes(device_token.as_bytes())[..12].to_string()
}

fn supported_apns_bundle_id(configured_bundle_id: &str, candidate: &str) -> bool {
    candidate == configured_bundle_id
        || candidate == DEFAULT_APNS_BUNDLE_ID
        || candidate == crate::legacy_identity::APNS_BUNDLE_ID
}

fn apns_topic(bundle_id: &str, event_type: ApnsEventType) -> String {
    let _ = event_type;
    bundle_id.to_string()
}

fn apns_push_type(event_type: ApnsEventType) -> &'static str {
    let _ = event_type;
    "alert"
}

fn apns_priority(event_type: ApnsEventType) -> &'static str {
    match event_type {
        ApnsEventType::ToolApproval | ApnsEventType::AwaitingInput | ApnsEventType::Error => "10",
        ApnsEventType::Completion | ApnsEventType::HiveUpdate | ApnsEventType::Test => "5",
    }
}

fn apns_expiration(event_type: ApnsEventType) -> u64 {
    let ttl = Duration::from_secs(apns_ttl_seconds(event_type));
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .saturating_add(ttl)
        .as_secs()
}

fn apns_ttl_seconds(event_type: ApnsEventType) -> u64 {
    match event_type {
        ApnsEventType::ToolApproval => 15 * 60,
        ApnsEventType::AwaitingInput => 60 * 60,
        ApnsEventType::Completion | ApnsEventType::Error => 6 * 60 * 60,
        ApnsEventType::HiveUpdate => 24 * 60 * 60,
        ApnsEventType::Test => 5 * 60,
    }
}

fn apns_collapse_id(payload: &ApnsPayload, event_type: ApnsEventType) -> Option<String> {
    payload.session_id.as_ref().map(|session_id| {
        let request_id = payload
            .data
            .as_ref()
            .and_then(|data| data.get("requestId"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let identity = format!("{}:{session_id}:{request_id}", event_type.as_str());
        let digest = hash_request_bytes(identity.as_bytes());
        format!("mitsuro-{}", &digest[..48])
    })
}

fn device_allows_event(level: &str, event_type: ApnsEventType) -> bool {
    match level {
        "all" => true,
        "important" => matches!(
            event_type,
            ApnsEventType::ToolApproval
                | ApnsEventType::AwaitingInput
                | ApnsEventType::Error
                | ApnsEventType::Test
        ),
        "silent" => matches!(event_type, ApnsEventType::Test),
        _ => false,
    }
}

fn device_plays_sound(level: &str, event_type: ApnsEventType) -> bool {
    level == "all"
        || level == "important"
            && matches!(
                event_type,
                ApnsEventType::ToolApproval | ApnsEventType::AwaitingInput | ApnsEventType::Error
            )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut truncated = value
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        truncated.push('…');
        truncated
    }
}

/// Build the APNs JSON payload from our internal payload struct.
fn build_apns_json(
    payload: &ApnsPayload,
    _bundle_id: &str,
    event_type: ApnsEventType,
    play_sound: bool,
) -> serde_json::Value {
    let mut payload = payload.clone();
    let bridged_hive =
        crate::legacy_identity::bridge_hive_notification(&mut payload.category, &mut payload.data);
    let mut aps = serde_json::json!({
        "alert": {
            "title": truncate_chars(&payload.title, MAX_APNS_TITLE_CHARS),
            "body": truncate_chars(&payload.body, MAX_APNS_BODY_CHARS),
        },
        "interruption-level": match event_type {
            ApnsEventType::Completion | ApnsEventType::HiveUpdate => "passive",
            _ => "active",
        },
    });
    if play_sound {
        aps["sound"] = serde_json::Value::String("default".into());
    }

    if let Some(ref cat) = payload.category {
        aps["category"] = serde_json::Value::String(cat.clone());
    }

    let mut root = serde_json::json!({ "aps": aps });

    if let Some(ref sid) = payload.session_id {
        aps["thread-id"] = serde_json::Value::String(sid.clone());
        root["sessionId"] = serde_json::Value::String(sid.clone());
    }

    if let Some(ref data) = payload.data {
        if let Some(fields) = data.as_object() {
            for (key, value) in fields {
                root[key] = value.clone();
            }
        } else {
            root["payload"] = data.clone();
        }
    }

    root["eventType"] = serde_json::Value::String(
        if bridged_hive && matches!(event_type, ApnsEventType::HiveUpdate) {
            crate::legacy_identity::HIVE_PUSH_EVENT
        } else {
            event_type.as_str()
        }
        .to_string(),
    );
    root["aps"] = aps;

    root
}

#[cfg(test)]
mod tests {
    use super::{
        apns_push_type, apns_topic, build_apns_json, live_activity_content_state,
        summarize_apns_failure_body, supported_apns_bundle_id, ApnsEventType, ApnsPayload,
        DEFAULT_APNS_BUNDLE_ID,
    };
    use mitsuro_core::storage::{
        hash_request_bytes, ApnsDeviceRegistration, ApnsDeviceStore, Database, LiveActivityToken,
    };
    use reqwest::StatusCode;
    use tempfile::TempDir;

    #[test]
    fn deprecated_hive_event_is_read_but_canonical_value_is_emitted() {
        let parsed =
            ApnsEventType::from_str(crate::legacy_identity::HIVE_PUSH_EVENT).expect("event alias");
        assert_eq!(parsed.as_str(), "hive_update");
    }

    #[test]
    fn apns_accepts_canonical_and_deprecated_registered_bundle_topics() {
        assert!(supported_apns_bundle_id(
            DEFAULT_APNS_BUNDLE_ID,
            DEFAULT_APNS_BUNDLE_ID
        ));
        assert!(supported_apns_bundle_id(
            DEFAULT_APNS_BUNDLE_ID,
            crate::legacy_identity::APNS_BUNDLE_ID
        ));
        assert!(!supported_apns_bundle_id(
            DEFAULT_APNS_BUNDLE_ID,
            "example.invalid.bundle"
        ));
        assert_eq!(
            apns_topic(
                crate::legacy_identity::APNS_BUNDLE_ID,
                ApnsEventType::Completion
            ),
            crate::legacy_identity::APNS_BUNDLE_ID
        );
    }

    #[test]
    fn hive_apns_delivery_uses_deprecated_navigation_contract() {
        let payload = ApnsPayload {
            title: "Hive".into(),
            body: "Updated".into(),
            session_id: Some("session-1".into()),
            category: Some("HIVE_SESSION".into()),
            data: Some(serde_json::json!({
                "type": "hive_update",
                "focus": "hive",
            })),
        };
        let json = build_apns_json(
            &payload,
            DEFAULT_APNS_BUNDLE_ID,
            ApnsEventType::HiveUpdate,
            false,
        );

        assert_eq!(json["aps"]["category"], "MAKO_SESSION");
        assert_eq!(json["type"], "mako_update");
        assert_eq!(json["focus"], "mako");
        assert_eq!(json["eventType"], "mako_update");
        assert_eq!(payload.category.as_deref(), Some("HIVE_SESSION"));
    }

    #[test]
    fn device_notifications_always_use_alert_contract() {
        assert_eq!(
            apns_topic(DEFAULT_APNS_BUNDLE_ID, ApnsEventType::ToolApproval),
            DEFAULT_APNS_BUNDLE_ID
        );
        assert_eq!(apns_push_type(ApnsEventType::ToolApproval), "alert");
        assert_eq!(
            apns_topic(DEFAULT_APNS_BUNDLE_ID, ApnsEventType::Completion),
            DEFAULT_APNS_BUNDLE_ID
        );
        assert_eq!(apns_push_type(ApnsEventType::Completion), "alert");
    }

    #[test]
    fn standard_events_use_alert_contract() {
        assert_eq!(
            apns_topic(DEFAULT_APNS_BUNDLE_ID, ApnsEventType::AwaitingInput),
            DEFAULT_APNS_BUNDLE_ID
        );
        assert_eq!(apns_push_type(ApnsEventType::AwaitingInput), "alert");
    }

    #[test]
    fn live_activity_state_targets_the_exact_approval_then_ends_on_completion() {
        let activity = LiveActivityToken {
            id: "activity-1".into(),
            user_id: None,
            session_id: "session-1".into(),
            push_token: "a".repeat(64),
            bundle_id: DEFAULT_APNS_BUNDLE_ID.into(),
            environment: "production".into(),
            content_state: serde_json::json!({
                "chatTitle": "Lifecycle test",
                "status": "working",
                "startedAtMs": 1_700_000_000_000_i64,
                "elapsedSeconds": 0,
                "toolCount": 0,
                "filesAdded": 0,
                "filesRemoved": 0,
            }),
            started_at_ms: 1_700_000_000_000,
            active: true,
            created_at: "2026-07-26T00:00:00Z".into(),
            updated_at: "2026-07-26T00:00:00Z".into(),
            ended_at: None,
        };
        let approval = ApnsPayload {
            title: "Permission Required".into(),
            body: "Bash needs approval".into(),
            session_id: Some("session-1".into()),
            category: None,
            data: Some(serde_json::json!({
                "requestId": "request-1",
                "toolName": "Bash",
                "sessionId": "session-1",
            })),
        };
        let (approval_state, should_end) = live_activity_content_state(
            &approval,
            ApnsEventType::ToolApproval,
            std::slice::from_ref(&activity),
        );
        assert!(!should_end);
        assert_eq!(approval_state["status"], "needs_input");
        assert_eq!(approval_state["toolApprovalId"], "request-1");
        assert_eq!(approval_state["toolApprovalSessionId"], "session-1");

        let completion = ApnsPayload {
            title: "Complete".into(),
            body: "Response finished".into(),
            session_id: Some("session-1".into()),
            category: None,
            data: Some(serde_json::json!({ "kind": "completion" })),
        };
        let (completion_state, should_end) = live_activity_content_state(
            &completion,
            ApnsEventType::Completion,
            std::slice::from_ref(&activity),
        );
        assert!(should_end);
        assert_eq!(completion_state["status"], "completed");
        assert!(completion_state.get("toolApprovalId").is_none());
    }

    #[test]
    fn alert_payload_exposes_routing_data_at_the_apns_root() {
        let payload = ApnsPayload {
            title: "Session complete".into(),
            body: "Response finished".into(),
            session_id: Some("session-1".into()),
            category: Some("CHAT_SESSION".into()),
            data: Some(serde_json::json!({
                "type": "chat_update",
                "kind": "completion",
                "sessionId": "session-1",
                "focus": "chat",
            })),
        };

        let json = build_apns_json(
            &payload,
            DEFAULT_APNS_BUNDLE_ID,
            ApnsEventType::Completion,
            false,
        );

        assert_eq!(json["aps"]["alert"]["title"], "Session complete");
        assert_eq!(json["aps"]["thread-id"], "session-1");
        assert_eq!(json["aps"]["category"], "CHAT_SESSION");
        assert_eq!(json["sessionId"], "session-1");
        assert_eq!(json["type"], "chat_update");
        assert_eq!(json["kind"], "completion");
        assert_eq!(json["focus"], "chat");
        assert_eq!(json["eventType"], "completion");
        assert!(json.get("data").is_none());
    }

    #[test]
    fn apns_failure_body_is_bounded_to_allowlisted_reason_or_hash_before_persistence() {
        const SENTINEL: &str = "APNS_PRIVATE_FAILURE_SENTINEL";
        let unknown_body = serde_json::json!({
            "reason": SENTINEL,
            "debug": SENTINEL,
        })
        .to_string();
        let unknown =
            summarize_apns_failure_body(StatusCode::BAD_REQUEST, unknown_body.as_bytes(), true);
        assert!(!unknown.contains(SENTINEL));
        assert!(unknown.contains(&hash_request_bytes(unknown_body.as_bytes())));
        assert!(unknown.contains("body_truncated=true"));

        let known_body = serde_json::json!({
            "reason": "BadDeviceToken",
            "debug": SENTINEL,
        })
        .to_string();
        let known =
            summarize_apns_failure_body(StatusCode::BAD_REQUEST, known_body.as_bytes(), false);
        assert_eq!(
            known,
            "APNs rejected request (status=400, reason=BadDeviceToken)"
        );
        assert!(!known.contains(SENTINEL));

        let temp = TempDir::new().unwrap();
        let db = Database::new(&temp.path().join("apns-failure.db")).unwrap();
        let store = ApnsDeviceStore::new(&db);
        store
            .upsert(ApnsDeviceRegistration {
                user_id: None,
                device_token: "private-device-token",
                bundle_id: DEFAULT_APNS_BUNDLE_ID,
                notification_level: "important",
                environment: "production",
            })
            .unwrap();
        store
            .mark_failure("private-device-token", &unknown)
            .unwrap();
        let mut devices = store.get_for_user(None).unwrap();
        let persisted = devices.remove(0);
        let persisted_reason = persisted.last_failure_reason.unwrap();
        assert_eq!(persisted_reason, unknown);
        assert!(!persisted_reason.contains(SENTINEL));
    }
}
