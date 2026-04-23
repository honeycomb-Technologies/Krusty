//! Apple Push Notification service (APNs)
//!
//! Sends push notifications to iOS devices via the APNs HTTP/2 API.
//! Uses JWT (ES256) token-based authentication with a .p8 private key.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use krusty_core::storage::{ApnsDevice, ApnsDeviceStore, Database};

const APNS_PRODUCTION_URL: &str = "https://api.push.apple.com";
const APNS_SANDBOX_URL: &str = "https://api.sandbox.push.apple.com";
pub(crate) const DEFAULT_APNS_BUNDLE_ID: &str = "io.krusty.mobile";

/// JWT tokens are valid for up to 60 minutes. We refresh at 50 min.
const JWT_REFRESH_INTERVAL: Duration = Duration::from_secs(50 * 60);

const MAX_APNS_ATTEMPTS: usize = 3;
const APNS_RETRY_BASE_DELAY_MS: u64 = 300;
const MAX_STALE_FAILURES: i64 = 10;

#[derive(Debug, Clone, Serialize)]
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
    MakoUpdate,
    Error,
    Test,
}

impl ApnsEventType {
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
        let key_path = std::env::var("KRUSTY_APNS_KEY_PATH").ok()?;
        let key_id = std::env::var("KRUSTY_APNS_KEY_ID").ok()?;
        let team_id = std::env::var("KRUSTY_APNS_TEAM_ID").ok()?;
        let bundle_id = std::env::var("KRUSTY_APNS_BUNDLE_ID")
            .unwrap_or_else(|_| DEFAULT_APNS_BUNDLE_ID.into());
        let sandbox = std::env::var("KRUSTY_APNS_SANDBOX")
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

    fn base_url(&self) -> &str {
        if self.config.sandbox {
            APNS_SANDBOX_URL
        } else {
            APNS_PRODUCTION_URL
        }
    }

    /// Send a push notification to a single device.
    async fn send_to_device(
        &self,
        device_token: &str,
        payload: &ApnsPayload,
        event_type: ApnsEventType,
    ) -> Result<()> {
        let token = self.get_token().await?;
        let url = format!("{}/3/device/{}", self.base_url(), device_token);

        // Build the APNs JSON payload
        let apns_payload = build_apns_json(payload, &self.config.bundle_id, event_type);

        let topic = apns_topic(&self.config.bundle_id, event_type);
        let push_type = apns_push_type(event_type);

        let resp = self
            .client
            .post(&url)
            .header("authorization", format!("bearer {token}"))
            .header("apns-topic", &topic)
            .header("apns-push-type", push_type)
            .header("apns-priority", "10")
            .header("apns-expiration", "0")
            .json(&apns_payload)
            .send()
            .await
            .context("APNs request failed")?;

        let status = resp.status();
        if status.is_success() {
            debug!(device_token, "APNs delivery succeeded");
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(anyhow::anyhow!("APNs returned {status}: {body}"))
        }
    }

    fn load_devices_for_user(&self, user_id: Option<&str>) -> Result<Vec<ApnsDevice>> {
        let db =
            Database::new(&self.db_path).context("Failed to open DB for APNs device lookup")?;
        let store = ApnsDeviceStore::new(&db);
        store.get_for_user(user_id)
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
            attempted: devices.len(),
            sent: 0,
            failed: 0,
            stale_removed: 0,
        };

        for device in &devices {
            let mut delivered = false;

            for attempt in 0..MAX_APNS_ATTEMPTS {
                match self
                    .send_to_device(&device.device_token, &payload, event_type)
                    .await
                {
                    Ok(()) => {
                        let _ = self.mark_device_success(&device.device_token);
                        stats.sent += 1;
                        delivered = true;
                        break;
                    }
                    Err(e) => {
                        if attempt < MAX_APNS_ATTEMPTS - 1 {
                            let delay = APNS_RETRY_BASE_DELAY_MS * 2u64.pow(attempt as u32);
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        } else {
                            warn!(
                                device_token = device.device_token,
                                error = %e,
                                "APNs delivery failed after {MAX_APNS_ATTEMPTS} attempts"
                            );
                            let _ = self.mark_device_failure(&device.device_token, &e.to_string());
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
            stats.stale_removed = removed;
        }

        stats
    }
}

fn apns_topic(bundle_id: &str, event_type: ApnsEventType) -> String {
    match event_type {
        ApnsEventType::ToolApproval | ApnsEventType::Completion => {
            format!("{}.push-type.liveactivity", bundle_id)
        }
        _ => bundle_id.to_string(),
    }
}

fn apns_push_type(event_type: ApnsEventType) -> &'static str {
    match event_type {
        ApnsEventType::ToolApproval | ApnsEventType::Completion => "liveactivity",
        _ => "alert",
    }
}

/// Build the APNs JSON payload from our internal payload struct.
fn build_apns_json(
    payload: &ApnsPayload,
    _bundle_id: &str,
    _event_type: ApnsEventType,
) -> serde_json::Value {
    let mut aps = serde_json::json!({
        "alert": {
            "title": payload.title,
            "body": payload.body,
        },
        "sound": "default",
        "mutable-content": 1,
    });

    if let Some(ref cat) = payload.category {
        aps["category"] = serde_json::Value::String(cat.clone());
    }

    let mut root = serde_json::json!({ "aps": aps });

    if let Some(ref sid) = payload.session_id {
        root["session_id"] = serde_json::Value::String(sid.clone());
    }

    if let Some(ref data) = payload.data {
        root["data"] = data.clone();
    }

    root
}

#[cfg(test)]
mod tests {
    use super::{apns_push_type, apns_topic, ApnsEventType, DEFAULT_APNS_BUNDLE_ID};

    #[test]
    fn live_activity_events_use_liveactivity_contract() {
        assert_eq!(
            apns_topic(DEFAULT_APNS_BUNDLE_ID, ApnsEventType::ToolApproval),
            format!("{}.push-type.liveactivity", DEFAULT_APNS_BUNDLE_ID)
        );
        assert_eq!(apns_push_type(ApnsEventType::ToolApproval), "liveactivity");
        assert_eq!(
            apns_topic(DEFAULT_APNS_BUNDLE_ID, ApnsEventType::Completion),
            format!("{}.push-type.liveactivity", DEFAULT_APNS_BUNDLE_ID)
        );
        assert_eq!(apns_push_type(ApnsEventType::Completion), "liveactivity");
    }

    #[test]
    fn standard_events_use_alert_contract() {
        assert_eq!(
            apns_topic(DEFAULT_APNS_BUNDLE_ID, ApnsEventType::AwaitingInput),
            DEFAULT_APNS_BUNDLE_ID
        );
        assert_eq!(apns_push_type(ApnsEventType::AwaitingInput), "alert");
    }
}
