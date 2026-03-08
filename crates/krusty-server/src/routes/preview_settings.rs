//! Preview / port-forwarding settings endpoints.

use std::ops::RangeInclusive;

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::storage::{Database, Preferences};

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

const PREVIEW_SETTINGS_KEY: &str = "preview_settings_v1";
const DEFAULT_BLOCKED_PORTS: [u16; 3] = [22, 2375, 2376];
const AUTO_REFRESH_RANGE_SECS: RangeInclusive<u16> = 2..=60;
const PROBE_TIMEOUT_RANGE_MS: RangeInclusive<u16> = 300..=1500;
const DEFAULT_PROBE_TIMEOUT_MS: u16 = 800;

/// Preview/port-forwarding settings stored in user preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PreviewSettings {
    pub enabled: bool,
    pub auto_refresh_secs: u16,
    pub show_only_http_like: bool,
    pub probe_timeout_ms: u16,
    pub allow_force_open_non_http: bool,
    pub pinned_ports: Vec<u16>,
    pub hidden_ports: Vec<u16>,
    pub blocked_ports: Vec<u16>,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_refresh_secs: 5,
            show_only_http_like: false,
            probe_timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
            allow_force_open_non_http: true,
            pinned_ports: vec![],
            hidden_ports: vec![],
            blocked_ports: DEFAULT_BLOCKED_PORTS.to_vec(),
        }
    }
}

impl PreviewSettings {
    pub fn normalize(&mut self) {
        normalize_ports(&mut self.pinned_ports);
        normalize_ports(&mut self.hidden_ports);
        normalize_ports(&mut self.blocked_ports);

        if !AUTO_REFRESH_RANGE_SECS.contains(&self.auto_refresh_secs) {
            self.auto_refresh_secs = Self::default().auto_refresh_secs;
        }
        if !PROBE_TIMEOUT_RANGE_MS.contains(&self.probe_timeout_ms) {
            self.probe_timeout_ms = DEFAULT_PROBE_TIMEOUT_MS;
        }
    }

    pub fn is_blocked(&self, port: u16) -> bool {
        self.blocked_ports.binary_search(&port).is_ok()
    }
}

#[derive(Debug, Deserialize)]
struct PreviewSettingsPatch {
    enabled: Option<bool>,
    auto_refresh_secs: Option<u16>,
    show_only_http_like: Option<bool>,
    probe_timeout_ms: Option<u16>,
    allow_force_open_non_http: Option<bool>,
    blocked_ports: Option<Vec<u16>>,
    pinned_ports: Option<Vec<u16>>,
    hidden_ports: Option<Vec<u16>>,
}

#[derive(Debug, Deserialize)]
struct PortMutationRequest {
    port: u16,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(get_preview_settings).patch(update_preview_settings),
        )
        .route("/pins", post(add_pinned_port))
        .route("/pins/:port", delete(remove_pinned_port))
        .route("/hidden", post(add_hidden_port))
        .route("/hidden/:port", delete(remove_hidden_port))
}

async fn get_preview_settings(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<PreviewSettings>, AppError> {
    let settings = load_preview_settings(&state, user.as_ref())?;
    Ok(Json(settings))
}

async fn update_preview_settings(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(patch): Json<PreviewSettingsPatch>,
) -> Result<Json<PreviewSettings>, AppError> {
    let mut settings = load_preview_settings(&state, user.as_ref())?;

    if let Some(enabled) = patch.enabled {
        settings.enabled = enabled;
    }

    if let Some(auto_refresh_secs) = patch.auto_refresh_secs {
        if !AUTO_REFRESH_RANGE_SECS.contains(&auto_refresh_secs) {
            return Err(AppError::BadRequest(format!(
                "auto_refresh_secs must be within {}-{} seconds",
                AUTO_REFRESH_RANGE_SECS.start(),
                AUTO_REFRESH_RANGE_SECS.end()
            )));
        }
        settings.auto_refresh_secs = auto_refresh_secs;
    }

    if let Some(show_only_http_like) = patch.show_only_http_like {
        settings.show_only_http_like = show_only_http_like;
    }

    if let Some(probe_timeout_ms) = patch.probe_timeout_ms {
        if !PROBE_TIMEOUT_RANGE_MS.contains(&probe_timeout_ms) {
            return Err(AppError::BadRequest(format!(
                "probe_timeout_ms must be within {}-{} milliseconds",
                PROBE_TIMEOUT_RANGE_MS.start(),
                PROBE_TIMEOUT_RANGE_MS.end()
            )));
        }
        settings.probe_timeout_ms = probe_timeout_ms;
    }

    if let Some(allow_force_open_non_http) = patch.allow_force_open_non_http {
        settings.allow_force_open_non_http = allow_force_open_non_http;
    }

    if let Some(blocked_ports) = patch.blocked_ports {
        settings.blocked_ports = blocked_ports;
    }

    if let Some(pinned_ports) = patch.pinned_ports {
        settings.pinned_ports = pinned_ports;
    }

    if let Some(hidden_ports) = patch.hidden_ports {
        settings.hidden_ports = hidden_ports;
    }

    settings.normalize();
    save_preview_settings(&state, user.as_ref(), &settings)?;
    Ok(Json(settings))
}

async fn add_pinned_port(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<PortMutationRequest>,
) -> Result<Json<PreviewSettings>, AppError> {
    if req.port == state.server_port {
        return Err(AppError::BadRequest(
            "Cannot pin the Krusty server port".to_string(),
        ));
    }

    let mut settings = load_preview_settings(&state, user.as_ref())?;
    settings.pinned_ports.push(req.port);
    settings.hidden_ports.retain(|p| *p != req.port);
    settings.normalize();
    save_preview_settings(&state, user.as_ref(), &settings)?;
    Ok(Json(settings))
}

async fn remove_pinned_port(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(port): Path<u16>,
) -> Result<Json<PreviewSettings>, AppError> {
    let mut settings = load_preview_settings(&state, user.as_ref())?;
    settings.pinned_ports.retain(|p| *p != port);
    settings.normalize();
    save_preview_settings(&state, user.as_ref(), &settings)?;
    Ok(Json(settings))
}

async fn add_hidden_port(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<PortMutationRequest>,
) -> Result<Json<PreviewSettings>, AppError> {
    let mut settings = load_preview_settings(&state, user.as_ref())?;
    settings.hidden_ports.push(req.port);
    settings.normalize();
    save_preview_settings(&state, user.as_ref(), &settings)?;
    Ok(Json(settings))
}

async fn remove_hidden_port(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(port): Path<u16>,
) -> Result<Json<PreviewSettings>, AppError> {
    let mut settings = load_preview_settings(&state, user.as_ref())?;
    settings.hidden_ports.retain(|p| *p != port);
    settings.normalize();
    save_preview_settings(&state, user.as_ref(), &settings)?;
    Ok(Json(settings))
}

pub(crate) fn load_preview_settings(
    state: &AppState,
    user: Option<&CurrentUser>,
) -> Result<PreviewSettings, AppError> {
    let prefs = preferences_for_user(state, user)?;
    let raw = prefs.get(PREVIEW_SETTINGS_KEY);
    let mut settings = match raw {
        Some(raw) => match serde_json::from_str::<PreviewSettings>(&raw) {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(
                    "Failed to parse preview settings JSON, using defaults: {}",
                    err
                );
                PreviewSettings::default()
            }
        },
        None => PreviewSettings::default(),
    };
    settings.normalize();
    Ok(settings)
}

pub(crate) fn save_preview_settings(
    state: &AppState,
    user: Option<&CurrentUser>,
    settings: &PreviewSettings,
) -> Result<(), AppError> {
    let prefs = preferences_for_user(state, user)?;
    let raw = serde_json::to_string(settings)?;
    prefs.set(PREVIEW_SETTINGS_KEY, &raw)?;
    Ok(())
}

fn preferences_for_user(
    state: &AppState,
    user: Option<&CurrentUser>,
) -> Result<Preferences, AppError> {
    let db = Database::new(&state.db_path)?;
    let user_id = user.and_then(|u| u.0.user_id.as_deref());
    let prefs = match user_id {
        Some(user_id) => Preferences::for_user(db, user_id),
        None => Preferences::new(db),
    };
    Ok(prefs)
}

fn normalize_ports(ports: &mut Vec<u16>) {
    ports.retain(|p| *p != 0);
    ports.sort_unstable();
    ports.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_settings_normalize_deduplicates_ports() {
        let mut settings = PreviewSettings {
            pinned_ports: vec![5173, 5173, 0, 3000],
            hidden_ports: vec![3000, 3000],
            blocked_ports: vec![22, 22, 2376],
            auto_refresh_secs: 999,
            probe_timeout_ms: 9999,
            ..PreviewSettings::default()
        };

        settings.normalize();

        assert_eq!(settings.pinned_ports, vec![3000, 5173]);
        assert_eq!(settings.hidden_ports, vec![3000]);
        assert_eq!(settings.blocked_ports, vec![22, 2376]);
        assert_eq!(settings.auto_refresh_secs, 5);
        assert_eq!(settings.probe_timeout_ms, DEFAULT_PROBE_TIMEOUT_MS);
    }

    #[test]
    fn preview_settings_defaults_include_preview_probe_controls() {
        let settings = PreviewSettings::default();
        assert_eq!(settings.probe_timeout_ms, DEFAULT_PROBE_TIMEOUT_MS);
        assert!(settings.allow_force_open_non_http);
    }
}
