//! Web Push notification service
//!
//! Handles VAPID key management and sending push notifications
//! via the Web Push protocol (RFC 8030).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::http;
use base64ct::{Base64UrlUnpadded, Encoding};
use serde::Serialize;
use tokio::time::sleep;
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};
use web_push_native::p256::PublicKey;
use web_push_native::{Auth, WebPushBuilder};

use krusty_core::storage::{
    Database, PushDeliveryAttemptInput, PushDeliveryAttemptStore, PushSubscription,
    PushSubscriptionStore,
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
#[derive(Debug, Clone, Serialize)]
pub struct PushPayload {
    pub title: String,
    pub body: String,
    pub session_id: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum PushEventType {
    Completion,
    AwaitingInput,
    MakoUpdate,
    Error,
    Test,
}

impl PushEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completion => "completion",
            Self::AwaitingInput => "awaiting_input",
            Self::MakoUpdate => "mako_update",
            Self::Error => "error",
            Self::Test => "test",
        }
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

impl PushService {
    /// Load or generate a VAPID keypair and create the service.
    pub fn init(vapid_key_path: &std::path::Path, db_path: Arc<PathBuf>) -> Result<Self> {
        if let Some(parent) = vapid_key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let keypair = if vapid_key_path.exists() {
            let pem =
                std::fs::read_to_string(vapid_key_path).context("Failed to read VAPID key file")?;
            ES256KeyPair::from_pem(&pem).context("Failed to parse VAPID PEM")?
        } else {
            let kp = ES256KeyPair::generate();
            let pem = kp.to_pem().context("Failed to serialize VAPID key")?;
            std::fs::write(vapid_key_path, &pem).context("Failed to write VAPID key file")?;
            tracing::info!(
                "Generated new VAPID keypair at {}",
                vapid_key_path.display()
            );
            kp
        };

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

        if subscriptions.is_empty() {
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
