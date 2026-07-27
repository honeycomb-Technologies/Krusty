//! Web Push notification service
//!
//! Handles VAPID key management and sending push notifications
//! via the Web Push protocol (RFC 8030).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::http;
use base64ct::{Base64UrlUnpadded, Encoding};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};
use web_push_native::p256::PublicKey;
use web_push_native::{Auth, WebPushBuilder};

use krusty_core::storage::{
    Database, ExpoPushDevice, ExpoPushDeviceStore, NotificationIntentStore,
    PushDeliveryAttemptInput, PushDeliveryAttemptStore, PushSubscription, PushSubscriptionStore,
};

const MAX_PUSH_ATTEMPTS: usize = 3;
const PUSH_RETRY_BASE_DELAY_MS: u64 = 300;

const ALLOWED_PUSH_ENDPOINT_HOSTS: &[&str] = &[
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
    "webpush.push.apple.com",
];
const ALLOWED_PUSH_ENDPOINT_SUFFIXES: &[&str] = &[
    ".push.services.mozilla.com",
    ".notify.windows.com",
    ".push.apple.com",
];

/// Validate and normalize a browser Web Push endpoint before it is persisted or used.
///
/// Push delivery is an outbound server request, so endpoints are restricted to HTTPS
/// origins operated by known browser push providers. This prevents attacker-supplied
/// subscriptions from turning push test/diagnostic routes into SSRF probes.
pub fn normalize_push_endpoint(endpoint: &str) -> Result<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        anyhow::bail!("Push subscription endpoint is required");
    }

    let uri: http::Uri = endpoint
        .parse()
        .context("Push subscription endpoint must be a valid URL")?;

    if uri.scheme_str() != Some("https") || uri.host().is_none() {
        anyhow::bail!("Push subscription endpoint must be an https URL");
    }

    let host = uri
        .host()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !is_allowed_push_endpoint_host(&host) {
        anyhow::bail!("Push subscription endpoint host is not a supported push service");
    }

    Ok(endpoint.to_string())
}

fn is_allowed_push_endpoint_host(host: &str) -> bool {
    ALLOWED_PUSH_ENDPOINT_HOSTS.contains(&host)
        || ALLOWED_PUSH_ENDPOINT_SUFFIXES
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

/// Payload sent inside a push notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushPayload {
    pub title: String,
    pub body: String,
    pub session_id: Option<String>,
    pub tag: Option<String>,
    pub category: Option<String>,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy)]
pub enum PushEventType {
    ToolApproval,
    Completion,
    AwaitingInput,
    MakoUpdate,
    Error,
    Test,
}

impl PushEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolApproval => "tool_approval",
            Self::Completion => "completion",
            Self::AwaitingInput => "awaiting_input",
            Self::MakoUpdate => "mako_update",
            Self::Error => "error",
            Self::Test => "test",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "tool_approval" => Self::ToolApproval,
            "completion" => Self::Completion,
            "awaiting_input" => Self::AwaitingInput,
            "mako_update" => Self::MakoUpdate,
            "error" => Self::Error,
            "test" => Self::Test,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PushNotifyStats {
    pub attempted: usize,
    pub sent: usize,
    pub stale_removed: usize,
    pub failed: usize,
}

pub struct PushService {
    keypair: ES256KeyPair,
    public_key_base64url: String,
    contact: String,
    db_path: Arc<PathBuf>,
    http_client: reqwest::Client,
}

enum DeliveryOutcome {
    Success {
        status: u16,
        latency_ms: u64,
    },
    Stale {
        status: u16,
        latency_ms: u64,
    },
    Failure {
        status: Option<u16>,
        reason: String,
        latency_ms: Option<u64>,
    },
}

struct VapidInitLock(File);

impl VapidInitLock {
    fn acquire(key_path: &Path) -> Result<Self> {
        let lock_path = sidecar_path(key_path, ".lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&lock_path)
            .with_context(|| format!("Failed to open VAPID init lock {}", lock_path.display()))?;
        set_private_file_permissions(&lock_path)?;
        FileExt::lock_exclusive(&file).context("Failed to lock VAPID key initialization")?;
        Ok(Self(file))
    }
}

impl Drop for VapidInitLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn load_or_create_vapid_keypair(key_path: &Path) -> Result<ES256KeyPair> {
    let parent = key_path
        .parent()
        .context("VAPID key path must have a parent directory")?;
    fs::create_dir_all(parent)?;
    let _init_lock = VapidInitLock::acquire(key_path)?;

    if key_path.exists() {
        set_private_file_permissions(key_path)?;
        let pem = fs::read_to_string(key_path).context("Failed to read VAPID key file")?;
        return ES256KeyPair::from_pem(&pem).context("Failed to parse VAPID PEM");
    }

    let keypair = ES256KeyPair::generate();
    let pem = keypair.to_pem().context("Failed to serialize VAPID key")?;
    let temp_path = sidecar_path(key_path, &format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let write_result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp_path)
            .with_context(|| format!("Failed to create VAPID temp file {}", temp_path.display()))?;
        file.write_all(pem.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, key_path).with_context(|| {
            format!(
                "Failed to atomically install VAPID key {}",
                key_path.display()
            )
        })?;
        set_private_file_permissions(key_path)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result?;

    tracing::info!(
        "Generated new private VAPID keypair at {}",
        key_path.display()
    );
    Ok(keypair)
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn set_private_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("Failed to set private permissions on {}", path.display()))?;
    }
    Ok(())
}

impl PushService {
    /// Load or generate a VAPID keypair and create the service.
    pub fn init(vapid_key_path: &std::path::Path, db_path: Arc<PathBuf>) -> Result<Self> {
        let keypair = load_or_create_vapid_keypair(vapid_key_path)?;

        let public_key_bytes = keypair.public_key().public_key().to_bytes_uncompressed();
        let public_key_base64url = Base64UrlUnpadded::encode_string(&public_key_bytes);

        let contact = std::env::var("KRUSTY_PUSH_CONTACT")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "mailto:krusty@localhost".to_string());

        Ok(Self {
            keypair,
            public_key_base64url,
            contact,
            db_path,
            http_client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .build()
                .context("Failed to create push HTTP client")?,
        })
    }

    /// The VAPID public key encoded as base64url (no padding).
    /// Clients need this to create a PushSubscription.
    pub fn vapid_public_key_base64url(&self) -> &str {
        &self.public_key_base64url
    }

    async fn send_once(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        payload: &PushPayload,
    ) -> Result<reqwest::Response> {
        let endpoint =
            normalize_push_endpoint(endpoint).context("Invalid push subscription endpoint")?;
        let endpoint_uri: http::Uri = endpoint
            .parse()
            .context("Invalid push subscription endpoint")?;

        let ua_public_bytes =
            Base64UrlUnpadded::decode_vec(p256dh).context("Invalid p256dh key")?;
        let ua_public =
            PublicKey::from_sec1_bytes(&ua_public_bytes).context("Invalid p256dh public key")?;

        let ua_auth_bytes = Base64UrlUnpadded::decode_vec(auth).context("Invalid auth secret")?;
        let ua_auth_arr: [u8; 16] = ua_auth_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Auth secret must be 16 bytes"))?;
        let ua_auth: Auth = ua_auth_arr.into();

        let body = serde_json::to_vec(payload)?;

        let http_request = WebPushBuilder::new(endpoint_uri, ua_public, ua_auth)
            .with_vapid(&self.keypair, &self.contact)
            .build(body)
            .context("Failed to build push request")?;

        let (parts, body_bytes) = http_request.into_parts();
        let url = parts.uri.to_string();

        let mut req_builder = self.http_client.request(
            reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
                .unwrap_or(reqwest::Method::POST),
            &url,
        );
        for (name, value) in &parts.headers {
            if let Ok(v) = value.to_str() {
                req_builder = req_builder.header(name.as_str(), v);
            }
        }

        req_builder
            .body(body_bytes)
            .send()
            .await
            .context("Failed to send push notification")
    }

    async fn send_with_retry(
        &self,
        subscription: &PushSubscription,
        payload: &PushPayload,
    ) -> DeliveryOutcome {
        let start = Instant::now();

        for attempt in 1..=MAX_PUSH_ATTEMPTS {
            match self
                .send_once(
                    &subscription.endpoint,
                    &subscription.p256dh,
                    &subscription.auth,
                    payload,
                )
                .await
            {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let latency_ms = elapsed_ms(start);

                    if (200..300).contains(&status) {
                        return DeliveryOutcome::Success { status, latency_ms };
                    }

                    if status == 403 || status == 404 || status == 410 {
                        return DeliveryOutcome::Stale { status, latency_ms };
                    }

                    let reason = format!("Push failed with status {}", status);

                    if is_transient_status(status) && attempt < MAX_PUSH_ATTEMPTS {
                        tracing::warn!(
                            endpoint = %subscription.endpoint,
                            status,
                            attempt,
                            "Transient push failure, retrying"
                        );
                        sleep(backoff_delay(attempt)).await;
                        continue;
                    }

                    return DeliveryOutcome::Failure {
                        status: Some(status),
                        reason,
                        latency_ms: Some(latency_ms),
                    };
                }
                Err(err) => {
                    let reason = err.to_string();
                    let latency_ms = Some(elapsed_ms(start));

                    if attempt < MAX_PUSH_ATTEMPTS {
                        tracing::warn!(
                            endpoint = %subscription.endpoint,
                            attempt,
                            error = %reason,
                            "Push send error, retrying"
                        );
                        sleep(backoff_delay(attempt)).await;
                        continue;
                    }

                    return DeliveryOutcome::Failure {
                        status: None,
                        reason,
                        latency_ms,
                    };
                }
            }
        }

        DeliveryOutcome::Failure {
            status: None,
            reason: "Exhausted push retry attempts".to_string(),
            latency_ms: None,
        }
    }

    async fn send_expo_once(
        &self,
        device: &ExpoPushDevice,
        payload: &PushPayload,
        event_type: PushEventType,
    ) -> Result<reqwest::Response> {
        let ttl = match event_type {
            PushEventType::ToolApproval => 15 * 60,
            PushEventType::AwaitingInput => 60 * 60,
            PushEventType::Completion | PushEventType::Error => 6 * 60 * 60,
            PushEventType::MakoUpdate => 24 * 60 * 60,
            PushEventType::Test => 5 * 60,
        };
        let important = matches!(
            event_type,
            PushEventType::ToolApproval | PushEventType::AwaitingInput | PushEventType::Error
        );
        let play_sound = device.notification_level == "all"
            || device.notification_level == "important" && important;
        let mut message = serde_json::json!({
            "to": device.expo_push_token,
            "title": truncate_push_text(&payload.title, 160),
            "body": truncate_push_text(&payload.body, 2_000),
            "data": payload.data.clone().unwrap_or_else(|| serde_json::json!({})),
            "ttl": ttl,
            "priority": if important { "high" } else { "default" },
            "sound": if play_sound { serde_json::Value::String("default".into()) } else { serde_json::Value::Null },
            "channelId": "default",
        });
        if let Some(category) = &payload.category {
            message["categoryId"] = serde_json::Value::String(category.clone());
        }
        if let Some(collapse_id) = payload.tag.as_ref().or(payload.session_id.as_ref()) {
            message["collapseId"] = serde_json::Value::String(truncate_push_text(collapse_id, 64));
        }
        let mut request = self
            .http_client
            .post("https://exp.host/--/api/v2/push/send")
            .json(&message);
        if let Ok(access_token) = std::env::var("KRUSTY_EXPO_PUSH_ACCESS_TOKEN") {
            if !access_token.trim().is_empty() {
                request = request.bearer_auth(access_token);
            }
        }
        request
            .send()
            .await
            .context("Failed to send Expo push notification")
    }

    /// Send a notification to all subscriptions for a user.
    /// In single-tenant mode (user_id = None), sends to all subscriptions.
    pub async fn notify_user(
        &self,
        user_id: Option<&str>,
        payload: PushPayload,
        event_type: PushEventType,
    ) -> PushNotifyStats {
        let db = match Database::new(&self.db_path) {
            Ok(db) => db,
            Err(e) => {
                tracing::error!("Failed to open DB for push: {}", e);
                return PushNotifyStats::default();
            }
        };
        let subscriptions = {
            let store = PushSubscriptionStore::new(&db);
            match user_id {
                Some(uid) => store.get_for_user(uid).unwrap_or_default(),
                None => store.get_all().unwrap_or_default(),
            }
        };
        let expo_devices = ExpoPushDeviceStore::new(&db)
            .get_for_user(user_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|device| expo_device_allows_event(&device.notification_level, event_type))
            .collect::<Vec<_>>();

        if subscriptions.is_empty() && expo_devices.is_empty() {
            tracing::info!(
                user_id = user_id.unwrap_or("<single-tenant>"),
                event_type = event_type.as_str(),
                "No push subscriptions found"
            );
            return PushNotifyStats::default();
        }

        tracing::info!(
            user_id = user_id.unwrap_or("<single-tenant>"),
            event_type = event_type.as_str(),
            count = subscriptions.len(),
            "Sending push notifications"
        );

        let mut stats = PushNotifyStats::default();

        for sub in subscriptions {
            stats.attempted += 1;

            let outcome = self.send_with_retry(&sub, &payload).await;
            match &outcome {
                DeliveryOutcome::Success { .. } => stats.sent += 1,
                DeliveryOutcome::Stale { .. } => stats.stale_removed += 1,
                DeliveryOutcome::Failure { .. } => stats.failed += 1,
            }

            record_delivery_outcome(&db, &sub, &payload, event_type, outcome);
        }

        for device in expo_devices {
            stats.attempted += 1;
            let started_at = Instant::now();
            let response = self.send_expo_once(&device, &payload, event_type).await;
            match response {
                Ok(response) => {
                    let status = response.status();
                    let response_body = response.json::<serde_json::Value>().await.ok();
                    let ticket_status = response_body
                        .as_ref()
                        .and_then(|value| value.get("data"))
                        .and_then(|value| value.get("status"))
                        .and_then(serde_json::Value::as_str);
                    let error_code = response_body
                        .as_ref()
                        .and_then(|value| value.get("data"))
                        .and_then(|value| value.get("details"))
                        .and_then(|value| value.get("error"))
                        .and_then(serde_json::Value::as_str);
                    if status.is_success() && ticket_status == Some("ok") {
                        stats.sent += 1;
                        let _ = ExpoPushDeviceStore::new(&db).mark_success(&device.expo_push_token);
                        record_expo_attempt(
                            &db,
                            &device,
                            &payload,
                            event_type,
                            "success",
                            Some(status.as_u16()),
                            None,
                            started_at.elapsed(),
                        );
                    } else if error_code == Some("DeviceNotRegistered") {
                        stats.stale_removed += 1;
                        let _ = ExpoPushDeviceStore::new(&db)
                            .remove_for_user(device.user_id.as_deref(), &device.expo_push_token);
                        record_expo_attempt(
                            &db,
                            &device,
                            &payload,
                            event_type,
                            "stale",
                            Some(status.as_u16()),
                            Some("DeviceNotRegistered"),
                            started_at.elapsed(),
                        );
                    } else {
                        stats.failed += 1;
                        let reason = error_code.unwrap_or("ExpoPushRejected");
                        let _ = ExpoPushDeviceStore::new(&db)
                            .mark_failure(&device.expo_push_token, reason);
                        record_expo_attempt(
                            &db,
                            &device,
                            &payload,
                            event_type,
                            "failure",
                            Some(status.as_u16()),
                            Some(reason),
                            started_at.elapsed(),
                        );
                    }
                }
                Err(error) => {
                    stats.failed += 1;
                    let reason = error.to_string();
                    let _ = ExpoPushDeviceStore::new(&db)
                        .mark_failure(&device.expo_push_token, &reason);
                    record_expo_attempt(
                        &db,
                        &device,
                        &payload,
                        event_type,
                        "failure",
                        None,
                        Some(&reason),
                        started_at.elapsed(),
                    );
                }
            }
        }

        tracing::info!(
            user_id = user_id.unwrap_or("<single-tenant>"),
            event_type = event_type.as_str(),
            attempted = stats.attempted,
            sent = stats.sent,
            stale_removed = stats.stale_removed,
            failed = stats.failed,
            "Push notifications finished"
        );

        stats
    }

    pub async fn notify_user_durable(
        &self,
        user_id: Option<&str>,
        payload: PushPayload,
        event_type: PushEventType,
    ) -> PushNotifyStats {
        let intent_id = Database::new(&self.db_path).ok().and_then(|db| {
            let payload_json = serde_json::to_value(&payload).ok()?;
            NotificationIntentStore::new(&db)
                .enqueue(
                    "push",
                    user_id,
                    payload.session_id.as_deref(),
                    event_type.as_str(),
                    &payload_json,
                    push_ttl_seconds(event_type) as i64,
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
            .and_then(|db| NotificationIntentStore::new(&db).recoverable("push", 100))
            .unwrap_or_default();
        for intent in intents {
            let Some(event_name) = intent.event_type.strip_prefix("push:") else {
                continue;
            };
            let Some(event_type) = PushEventType::from_str(event_name) else {
                self.mark_intent_cancelled(&intent.id, "unknown push event type");
                continue;
            };
            let Ok(payload) = serde_json::from_value::<PushPayload>(intent.payload) else {
                self.mark_intent_cancelled(&intent.id, "invalid push intent payload");
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

    fn finish_intent(&self, id: &str, stats: &PushNotifyStats) {
        if let Ok(db) = Database::new(&self.db_path) {
            let store = NotificationIntentStore::new(&db);
            if stats.sent > 0 && stats.failed == 0 {
                let _ = store.mark_accepted(id);
            } else if stats.attempted == 0 {
                let _ = store.mark_cancelled(id, "no eligible push subscriptions");
            } else {
                let _ = store.mark_failed(
                    id,
                    &format!(
                        "Push delivery failed: attempted={}, sent={}, failed={}",
                        stats.attempted, stats.sent, stats.failed
                    ),
                );
            }
        }
    }
}

fn push_ttl_seconds(event_type: PushEventType) -> u64 {
    match event_type {
        PushEventType::ToolApproval => 15 * 60,
        PushEventType::AwaitingInput => 60 * 60,
        PushEventType::Completion | PushEventType::Error => 6 * 60 * 60,
        PushEventType::MakoUpdate => 24 * 60 * 60,
        PushEventType::Test => 5 * 60,
    }
}

fn expo_device_allows_event(level: &str, event_type: PushEventType) -> bool {
    match level {
        "all" => true,
        "important" => matches!(
            event_type,
            PushEventType::ToolApproval
                | PushEventType::AwaitingInput
                | PushEventType::Error
                | PushEventType::Test
        ),
        "silent" => matches!(event_type, PushEventType::Test),
        _ => false,
    }
}

fn truncate_push_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut output = value
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        output.push('…');
        output
    }
}

fn record_expo_attempt(
    db: &Database,
    device: &ExpoPushDevice,
    payload: &PushPayload,
    event_type: PushEventType,
    outcome: &str,
    status: Option<u16>,
    error_message: Option<&str>,
    latency: Duration,
) {
    let endpoint = format!(
        "https://exp.host/--/api/v2/push/send/sha256:{}",
        &krusty_core::storage::hash_request_bytes(device.expo_push_token.as_bytes())[..12],
    );
    let _ = PushDeliveryAttemptStore::new(db).record_attempt(PushDeliveryAttemptInput {
        user_id: device.user_id.as_deref(),
        session_id: payload.session_id.as_deref(),
        endpoint: &endpoint,
        event_type: event_type.as_str(),
        outcome,
        http_status: status,
        error_message,
        latency_ms: Some(latency.as_millis() as u64),
    });
}

fn record_delivery_outcome(
    db: &Database,
    sub: &PushSubscription,
    payload: &PushPayload,
    event_type: PushEventType,
    outcome: DeliveryOutcome,
) {
    let store = PushSubscriptionStore::new(db);
    let attempt_store = PushDeliveryAttemptStore::new(db);

    match outcome {
        DeliveryOutcome::Success { status, latency_ms } => {
            let _ = store.mark_success(&sub.endpoint);
            let _ = attempt_store.record_attempt(PushDeliveryAttemptInput {
                user_id: sub.user_id.as_deref(),
                session_id: payload.session_id.as_deref(),
                endpoint: &sub.endpoint,
                event_type: event_type.as_str(),
                outcome: "success",
                http_status: Some(status),
                error_message: None,
                latency_ms: Some(latency_ms),
            });
            tracing::debug!(endpoint = %sub.endpoint, status, "Push sent");
        }
        DeliveryOutcome::Stale { status, latency_ms } => {
            tracing::info!(
                endpoint = %sub.endpoint,
                status,
                "Push subscription stale, removing"
            );
            let _ = store.remove_by_endpoint(&sub.endpoint);
            let _ = attempt_store.record_attempt(PushDeliveryAttemptInput {
                user_id: sub.user_id.as_deref(),
                session_id: payload.session_id.as_deref(),
                endpoint: &sub.endpoint,
                event_type: event_type.as_str(),
                outcome: "stale",
                http_status: Some(status),
                error_message: Some("subscription stale or rejected"),
                latency_ms: Some(latency_ms),
            });
        }
        DeliveryOutcome::Failure {
            status,
            reason,
            latency_ms,
        } => {
            let _ = store.mark_failure(&sub.endpoint, &reason);
            let _ = attempt_store.record_attempt(PushDeliveryAttemptInput {
                user_id: sub.user_id.as_deref(),
                session_id: payload.session_id.as_deref(),
                endpoint: &sub.endpoint,
                event_type: event_type.as_str(),
                outcome: "failure",
                http_status: status,
                error_message: Some(&reason),
                latency_ms,
            });
            tracing::warn!(endpoint = %sub.endpoint, status, "Push failed: {}", reason);
        }
    }
}

fn is_transient_status(status: u16) -> bool {
    status == 429 || status >= 500
}

fn backoff_delay(attempt: usize) -> Duration {
    let exponent = (attempt.saturating_sub(1)).min(10) as u32;
    let multiplier = 1u64 << exponent;
    Duration::from_millis(PUSH_RETRY_BASE_DELAY_MS.saturating_mul(multiplier))
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use tempfile::TempDir;

    use super::PushService;

    #[test]
    fn concurrent_vapid_initializers_converge_on_one_private_key() {
        let temp = TempDir::new().expect("temporary push root should exist");
        let key_path = Arc::new(temp.path().join("vapid.pem"));
        let db_path = Arc::new(temp.path().join("krusty.db"));
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let key_path = Arc::clone(&key_path);
                let db_path = Arc::clone(&db_path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    PushService::init(key_path.as_path(), db_path)
                        .expect("concurrent push initialization should succeed")
                        .vapid_public_key_base64url()
                        .to_string()
                })
            })
            .collect::<Vec<_>>();
        let public_keys = handles
            .into_iter()
            .map(|handle| handle.join().expect("initializer thread should not panic"))
            .collect::<Vec<_>>();

        assert!(public_keys
            .iter()
            .all(|public_key| public_key == &public_keys[0]));
        assert!(key_path.exists());
        assert!(super::sidecar_path(key_path.as_path(), ".lock").exists());
        assert!(std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp")));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key_path.as_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
