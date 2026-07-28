//! Bounded, content-free performance diagnostics from internal mobile builds.

use std::collections::BTreeMap;

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::storage::{
    Database, MobileDiagnosticEvent, MobileDiagnosticEventInput, MobileDiagnosticNativePayload,
    MobileDiagnosticNativePayloadInput, MobileDiagnosticReport, MobileDiagnosticRun,
    MobileDiagnosticRunInput, MobileDiagnosticStore,
};

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

const MAX_BATCH_BYTES: usize = 512 * 1024;
const MAX_EVENTS_PER_BATCH: usize = 256;
const MAX_NATIVE_PAYLOADS_PER_BATCH: usize = 16;
const MAX_NATIVE_PAYLOAD_BYTES: usize = 4 * 1024;
const MAX_NATIVE_SOURCE_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_NATIVE_DIAGNOSTIC_COUNT: u64 = 1_000;
const RETENTION_DAYS: i64 = 14;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/batches", post(ingest_batch))
        .route("/runs", get(list_runs))
        .route("/runs/:id/report", get(get_report))
        .route("/runs/:id/events", get(get_events))
        .route("/runs/:id/native-payloads", get(get_native_payloads))
        .layer(DefaultBodyLimit::max(MAX_BATCH_BYTES))
}

#[derive(Debug, Deserialize, Serialize)]
struct MobileDiagnosticBatchRequest {
    run: MobileDiagnosticRunPayload,
    #[serde(default)]
    events: Vec<MobileDiagnosticEventPayload>,
    #[serde(default)]
    native_payloads: Vec<MobileDiagnosticNativePayloadPayload>,
    #[serde(default)]
    completed: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct MobileDiagnosticRunPayload {
    id: String,
    installation_id: String,
    app_version: String,
    build_number: String,
    platform: String,
    os_version: String,
    device_class: String,
    capture_level: String,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    #[serde(default)]
    dropped_event_count: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct MobileDiagnosticEventPayload {
    sequence: u64,
    occurred_at_ms: i64,
    monotonic_ms: f64,
    category: String,
    name: String,
    duration_ms: Option<f64>,
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default)]
    attributes: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct MobileDiagnosticNativePayloadPayload {
    payload_id: String,
    kind: String,
    received_at_ms: i64,
    payload_json: String,
}

fn default_severity() -> String {
    "info".to_string()
}

#[derive(Debug, Serialize)]
struct MobileDiagnosticBatchResponse {
    run_id: String,
    accepted_events: usize,
    accepted_native_payloads: usize,
    dropped_attributes: usize,
}

#[derive(Debug, Deserialize)]
struct ListRunsQuery {
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ListRunsResponse {
    runs: Vec<MobileDiagnosticRun>,
}

#[derive(Debug, Deserialize)]
struct ListEventsQuery {
    after_sequence: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ListEventsResponse {
    events: Vec<MobileDiagnosticEvent>,
    next_after_sequence: Option<u64>,
    has_more: bool,
}

async fn ingest_batch(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(payload): Json<MobileDiagnosticBatchRequest>,
) -> Result<Json<MobileDiagnosticBatchResponse>, AppError> {
    validate_run(&payload.run)?;
    if payload.events.len() > MAX_EVENTS_PER_BATCH {
        return Err(AppError::BadRequest(format!(
            "diagnostic batch exceeds {MAX_EVENTS_PER_BATCH} events"
        )));
    }
    if payload.native_payloads.len() > MAX_NATIVE_PAYLOADS_PER_BATCH {
        return Err(AppError::BadRequest(format!(
            "diagnostic batch exceeds {MAX_NATIVE_PAYLOADS_PER_BATCH} native payloads"
        )));
    }

    let mut sanitized = Vec::with_capacity(payload.events.len());
    let mut dropped_attributes = 0usize;
    for event in &payload.events {
        validate_event(event)?;
        let (attributes, dropped) = sanitize_attributes(&event.attributes);
        dropped_attributes += dropped;
        let attributes_json = serde_json::to_string(&attributes)?;
        sanitized.push((event, attributes_json));
    }
    let event_inputs = sanitized
        .iter()
        .map(|(event, attributes_json)| MobileDiagnosticEventInput {
            sequence: event.sequence,
            occurred_at_ms: event.occurred_at_ms,
            monotonic_ms: event.monotonic_ms,
            category: &event.category,
            name: &event.name,
            duration_ms: event.duration_ms,
            severity: &event.severity,
            attributes_json,
        })
        .collect::<Vec<_>>();

    let mut sanitized_native = Vec::with_capacity(payload.native_payloads.len());
    for native in &payload.native_payloads {
        validate_native_payload(native)?;
        let parsed: serde_json::Value = serde_json::from_str(&native.payload_json)?;
        if !parsed.is_object() {
            return Err(AppError::BadRequest(
                "MetricKit payload must be a JSON object".to_string(),
            ));
        }
        let sanitized_payload = sanitize_native_summary(&parsed)?;
        let sanitized_json = serde_json::to_string(&sanitized_payload)?;
        if sanitized_json.len() > MAX_NATIVE_PAYLOAD_BYTES {
            return Err(AppError::BadRequest(
                "MetricKit payload exceeds the sanitized size limit".to_string(),
            ));
        }
        sanitized_native.push((native, sanitized_json));
    }
    let native_inputs = sanitized_native
        .iter()
        .map(
            |(payload, payload_json)| MobileDiagnosticNativePayloadInput {
                payload_id: &payload.payload_id,
                kind: &payload.kind,
                received_at_ms: payload.received_at_ms,
                payload_json,
            },
        )
        .collect::<Vec<_>>();

    let user_id = user.0.user_id.as_deref();
    let mut store = MobileDiagnosticStore::new(Database::new(&state.db_path)?);
    let (accepted_events, accepted_native_payloads) = store.ingest_batch(
        MobileDiagnosticRunInput {
            id: &payload.run.id,
            user_id,
            installation_id: &payload.run.installation_id,
            app_version: &payload.run.app_version,
            build_number: &payload.run.build_number,
            platform: &payload.run.platform,
            os_version: &payload.run.os_version,
            device_class: &payload.run.device_class,
            capture_level: &payload.run.capture_level,
            started_at_ms: payload.run.started_at_ms,
            ended_at_ms: payload.run.ended_at_ms,
            completed: payload.completed,
            dropped_event_count: payload.run.dropped_event_count,
        },
        &event_inputs,
        &native_inputs,
    )?;
    let _ = store.prune_older_than_days(RETENTION_DAYS);

    Ok(Json(MobileDiagnosticBatchResponse {
        run_id: payload.run.id,
        accepted_events,
        accepted_native_payloads,
        dropped_attributes,
    }))
}

async fn list_runs(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<ListRunsQuery>,
) -> Result<Json<ListRunsResponse>, AppError> {
    let store = MobileDiagnosticStore::new(Database::new(&state.db_path)?);
    let runs = store.list_runs_for_user(user.0.user_id.as_deref(), query.limit.unwrap_or(20))?;
    Ok(Json(ListRunsResponse { runs }))
}

async fn get_report(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<MobileDiagnosticReport>, AppError> {
    validate_token("run id", &id, 128)?;
    let store = MobileDiagnosticStore::new(Database::new(&state.db_path)?);
    let report = store
        .report_for_user(&id, user.0.user_id.as_deref())?
        .ok_or_else(|| AppError::NotFound("Diagnostic run not found".to_string()))?;
    Ok(Json(report))
}

async fn get_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ListEventsQuery>,
) -> Result<Json<ListEventsResponse>, AppError> {
    validate_token("run id", &id, 128)?;
    let limit = query.limit.unwrap_or(250).clamp(1, 500);
    let store = MobileDiagnosticStore::new(Database::new(&state.db_path)?);
    let mut events = store
        .events_for_user(
            &id,
            user.0.user_id.as_deref(),
            query.after_sequence.unwrap_or(0),
            limit + 1,
        )?
        .ok_or_else(|| AppError::NotFound("Diagnostic run not found".to_string()))?;
    let has_more = events.len() > limit;
    if has_more {
        events.truncate(limit);
    }
    let next_after_sequence = events.last().map(|event| event.sequence);
    Ok(Json(ListEventsResponse {
        events,
        next_after_sequence,
        has_more,
    }))
}

async fn get_native_payloads(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<MobileDiagnosticNativePayload>>, AppError> {
    validate_token("run id", &id, 128)?;
    let store = MobileDiagnosticStore::new(Database::new(&state.db_path)?);
    let payloads = store
        .native_payloads_for_user(&id, user.0.user_id.as_deref())?
        .ok_or_else(|| AppError::NotFound("Diagnostic run not found".to_string()))?;
    Ok(Json(payloads))
}

fn validate_run(run: &MobileDiagnosticRunPayload) -> Result<(), AppError> {
    validate_token("run id", &run.id, 128)?;
    validate_token("installation id", &run.installation_id, 128)?;
    validate_label("app version", &run.app_version, 32)?;
    validate_label("build number", &run.build_number, 32)?;
    if !matches!(run.platform.as_str(), "ios" | "android" | "web") {
        return Err(AppError::BadRequest(
            "invalid diagnostic platform".to_string(),
        ));
    }
    validate_label("OS version", &run.os_version, 64)?;
    validate_label("device class", &run.device_class, 64)?;
    if !matches!(run.capture_level.as_str(), "baseline" | "stress") {
        return Err(AppError::BadRequest("invalid capture level".to_string()));
    }
    if run.started_at_ms <= 0 || run.ended_at_ms.is_some_and(|end| end < run.started_at_ms) {
        return Err(AppError::BadRequest(
            "invalid diagnostic run timestamps".to_string(),
        ));
    }
    Ok(())
}

fn validate_native_payload(payload: &MobileDiagnosticNativePayloadPayload) -> Result<(), AppError> {
    validate_token("native payload id", &payload.payload_id, 128)?;
    if !matches!(payload.kind.as_str(), "metric" | "diagnostic") {
        return Err(AppError::BadRequest(
            "invalid MetricKit payload kind".to_string(),
        ));
    }
    if payload.received_at_ms <= 0 || payload.payload_json.len() > MAX_NATIVE_PAYLOAD_BYTES {
        return Err(AppError::BadRequest(
            "invalid MetricKit payload bounds".to_string(),
        ));
    }
    Ok(())
}

fn validate_event(event: &MobileDiagnosticEventPayload) -> Result<(), AppError> {
    if !matches!(
        event.category.as_str(),
        "app"
            | "navigation"
            | "session"
            | "network"
            | "stream"
            | "runtime"
            | "resource"
            | "webview"
            | "live_activity"
            | "widget"
            | "error"
            | "memory"
    ) {
        return Err(AppError::BadRequest(
            "invalid diagnostic event category".to_string(),
        ));
    }
    validate_token("event name", &event.name, 64)?;
    if !matches!(
        event.severity.as_str(),
        "debug" | "info" | "warning" | "error" | "fatal"
    ) {
        return Err(AppError::BadRequest(
            "invalid diagnostic severity".to_string(),
        ));
    }
    if !event.monotonic_ms.is_finite() || event.monotonic_ms < 0.0 {
        return Err(AppError::BadRequest(
            "invalid monotonic timestamp".to_string(),
        ));
    }
    if event
        .duration_ms
        .is_some_and(|duration| !duration.is_finite() || !(0.0..=600_000.0).contains(&duration))
    {
        return Err(AppError::BadRequest(
            "invalid diagnostic duration".to_string(),
        ));
    }
    Ok(())
}

fn validate_token(field: &str, value: &str, max: usize) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::BadRequest(format!("invalid {field}")));
    }
    Ok(())
}

fn validate_label(field: &str, value: &str, max: usize) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > max
        || value.chars().any(char::is_control)
        || value.contains("://")
        || value.contains('?')
        || value.contains('\\')
    {
        return Err(AppError::BadRequest(format!("invalid {field}")));
    }
    Ok(())
}

fn sanitize_attributes(
    attributes: &BTreeMap<String, serde_json::Value>,
) -> (BTreeMap<String, serde_json::Value>, usize) {
    let mut clean = BTreeMap::new();
    let mut dropped = 0usize;
    for (key, value) in attributes {
        if !allowed_attribute_key(key) || !safe_attribute_value(key, value) {
            dropped += 1;
            continue;
        }
        clean.insert(key.clone(), value.clone());
    }
    (clean, dropped)
}

fn allowed_attribute_key(key: &str) -> bool {
    matches!(
        key,
        "active_count"
            | "app_state"
            | "cancelled"
            | "code"
            | "count"
            | "dropped"
            | "from"
            | "http_status"
            | "level"
            | "method"
            | "metric"
            | "mode"
            | "outcome"
            | "pending_count"
            | "phase"
            | "queue_depth"
            | "request_hash"
            | "resource"
            | "retry_count"
            | "route"
            | "session_hash"
            | "source"
            | "state"
            | "surface"
            | "to"
            | "value"
            | "webview_kind"
    )
}

fn safe_attribute_value(key: &str, value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
        serde_json::Value::String(text) => {
            text.len() <= 96
                && !text.chars().any(char::is_control)
                && !text.contains("://")
                && !text.contains('?')
                && !text.to_ascii_lowercase().contains("bearer ")
                && (key == "route" || !text.contains('/'))
        }
        _ => false,
    }
}

fn sanitize_native_summary(value: &serde_json::Value) -> Result<serde_json::Value, AppError> {
    let source = value.as_object().ok_or_else(|| {
        AppError::BadRequest("MetricKit summary must be a JSON object".to_string())
    })?;
    if source
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err(AppError::BadRequest(
            "unsupported MetricKit summary schema".to_string(),
        ));
    }

    let mut clean = serde_json::Map::new();
    clean.insert("schema_version".to_string(), serde_json::json!(1));
    insert_bounded_native_integer(
        source,
        &mut clean,
        "source_payload_bytes",
        MAX_NATIVE_SOURCE_PAYLOAD_BYTES,
    )?;
    for key in [
        "has_application_launch_metrics",
        "has_application_responsiveness_metrics",
        "has_memory_metrics",
        "has_cpu_metrics",
        "has_disk_io_metrics",
        "has_display_metrics",
        "has_network_transfer_metrics",
        "has_application_exit_metrics",
        "has_cellular_condition_metrics",
        "has_location_activity_metrics",
        "has_animation_metrics",
    ] {
        let value = source
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                AppError::BadRequest(format!("invalid MetricKit summary field: {key}"))
            })?;
        clean.insert(key.to_string(), serde_json::Value::Bool(value));
    }
    for key in [
        "crash_diagnostic_count",
        "hang_diagnostic_count",
        "cpu_exception_diagnostic_count",
        "disk_write_exception_diagnostic_count",
    ] {
        insert_bounded_native_integer(source, &mut clean, key, MAX_NATIVE_DIAGNOSTIC_COUNT)?;
    }
    Ok(serde_json::Value::Object(clean))
}

fn insert_bounded_native_integer(
    source: &serde_json::Map<String, serde_json::Value>,
    destination: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    maximum: u64,
) -> Result<(), AppError> {
    let value = source
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value <= maximum)
        .ok_or_else(|| AppError::BadRequest(format!("invalid MetricKit summary field: {key}")))?;
    destination.insert(key.to_string(), serde_json::json!(value));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitization_drops_content_bearing_and_unknown_fields() {
        let attributes = BTreeMap::from([
            ("mode".to_string(), serde_json::json!("code")),
            ("route".to_string(), serde_json::json!("/(tabs)/settings")),
            ("message".to_string(), serde_json::json!("private prompt")),
            (
                "source".to_string(),
                serde_json::json!("https://secret.example/token?x=1"),
            ),
        ]);
        let (clean, dropped) = sanitize_attributes(&attributes);
        assert_eq!(dropped, 2);
        assert_eq!(clean.len(), 2);
        assert_eq!(clean["mode"], "code");
    }

    #[test]
    fn event_contract_rejects_unbounded_duration_and_freeform_name() {
        let event = MobileDiagnosticEventPayload {
            sequence: 1,
            occurred_at_ms: 1,
            monotonic_ms: 1.0,
            category: "runtime".to_string(),
            name: "long task containing spaces".to_string(),
            duration_ms: Some(700_000.0),
            severity: "info".to_string(),
            attributes: BTreeMap::new(),
        };
        assert!(validate_event(&event).is_err());
    }

    #[test]
    fn native_summary_allowlist_drops_every_unknown_field() {
        let payload = serde_json::json!({
            "schema_version": 1,
            "source_payload_bytes": 4096,
            "has_application_launch_metrics": true,
            "has_application_responsiveness_metrics": false,
            "has_memory_metrics": true,
            "has_cpu_metrics": true,
            "has_disk_io_metrics": false,
            "has_display_metrics": true,
            "has_network_transfer_metrics": false,
            "has_application_exit_metrics": true,
            "has_cellular_condition_metrics": false,
            "has_location_activity_metrics": false,
            "has_animation_metrics": false,
            "crash_diagnostic_count": 1,
            "hang_diagnostic_count": 2,
            "cpu_exception_diagnostic_count": 0,
            "disk_write_exception_diagnostic_count": 0,
            "prompt": "private content",
            "callStackTree": { "binaryName": "Krusty", "sampleCount": 12 },
            "exceptionReason": "private crash reason"
        });
        let clean = sanitize_native_summary(&payload).expect("sanitized");
        assert_eq!(clean["crash_diagnostic_count"], 1);
        assert!(clean.get("prompt").is_none());
        assert!(clean.get("callStackTree").is_none());
        assert!(clean.get("exceptionReason").is_none());
    }

    #[test]
    fn native_summary_rejects_missing_or_invalid_allowlisted_values() {
        let raw_metric_kit = serde_json::json!({
            "callStackTree": { "binaryName": "Krusty" },
            "exceptionReason": "private crash reason"
        });
        assert!(sanitize_native_summary(&raw_metric_kit).is_err());

        let invalid = serde_json::json!({
            "schema_version": 1,
            "source_payload_bytes": MAX_NATIVE_SOURCE_PAYLOAD_BYTES + 1,
            "has_application_launch_metrics": false,
            "has_application_responsiveness_metrics": false,
            "has_memory_metrics": false,
            "has_cpu_metrics": false,
            "has_disk_io_metrics": false,
            "has_display_metrics": false,
            "has_network_transfer_metrics": false,
            "has_application_exit_metrics": false,
            "has_cellular_condition_metrics": false,
            "has_location_activity_metrics": false,
            "has_animation_metrics": false,
            "crash_diagnostic_count": 0,
            "hang_diagnostic_count": 0,
            "cpu_exception_diagnostic_count": 0,
            "disk_write_exception_diagnostic_count": 0
        });
        assert!(sanitize_native_summary(&invalid).is_err());
    }
}
