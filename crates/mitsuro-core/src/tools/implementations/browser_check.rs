//! Governed localhost browser acceptance checks.
//!
//! This specialist tool deliberately stays narrower than a general browser:
//! it launches an installed Chromium-family browser in a temporary profile,
//! visits only loopback HTTP(S) URLs, performs bounded interactions, and fails
//! on page/console errors or unmet assertions. Generated web products can
//! therefore distinguish build/HTTP availability from runtime acceptance.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use url::Url;
use uuid::Uuid;

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use super::bash::background_endpoint_hints;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SETTLE_MS: u64 = 10_000;
const MAX_ACTIONS: usize = 32;
const MAX_VIEWPORTS: usize = 4;
const MAX_FILL_CHARS: usize = 4_096;

pub struct BrowserCheckTool;

#[derive(Debug, Clone, Deserialize)]
struct BrowserCheckParams {
    url: String,
    #[serde(default)]
    process_id: Option<String>,
    #[serde(default = "default_viewports")]
    viewports: Vec<Viewport>,
    #[serde(default)]
    actions: Vec<BrowserAction>,
    #[serde(default = "default_settle_ms")]
    settle_ms: u64,
    #[serde(default)]
    require_service_worker: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Viewport {
    #[serde(default)]
    name: Option<String>,
    width: u32,
    height: u32,
    #[serde(default = "default_device_scale_factor")]
    device_scale_factor: f64,
    #[serde(default)]
    mobile: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum BrowserAction {
    Click {
        selector: String,
    },
    Fill {
        selector: String,
        value: String,
    },
    Reload,
    Key {
        key: String,
    },
    Wait {
        ms: u64,
    },
    Assert {
        expression: String,
        message: String,
    },
    Visible {
        selector: String,
    },
    Inspect {
        #[serde(default)]
        selector: Option<String>,
    },
    AssertText {
        selector: String,
        /// Canonical expected text. `equals`, `contains`, and `expect` remain
        /// compatibility readers for calls produced against the older schema.
        #[serde(default)]
        text: Option<String>,
        #[serde(default, rename = "match")]
        match_mode: Option<TextMatchMode>,
        #[serde(default)]
        equals: Option<String>,
        #[serde(default)]
        contains: Option<String>,
        #[serde(default)]
        expect: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TextMatchMode {
    #[serde(alias = "value", alias = "exact")]
    Equals,
    Contains,
}

fn default_viewports() -> Vec<Viewport> {
    vec![
        Viewport {
            name: Some("phone".to_string()),
            width: 390,
            height: 844,
            device_scale_factor: 2.0,
            mobile: true,
        },
        Viewport {
            name: Some("desktop".to_string()),
            width: 1280,
            height: 720,
            device_scale_factor: 1.0,
            mobile: false,
        },
    ]
}

const fn default_settle_ms() -> u64 {
    500
}

const fn default_device_scale_factor() -> f64 {
    1.0
}

#[async_trait]
impl Tool for BrowserCheckTool {
    fn name(&self) -> &str {
        "browser_check"
    }

    fn description(&self) -> &str {
        "Run bounded runtime acceptance against a loopback web app in an installed Chromium browser. Can inspect bounded DOM landmarks before interaction, then check phone/desktop viewports, declared clicks/form fills/keys/structured text assertions, console errors, page exceptions, and optional service-worker readiness. Use this before claiming an interactive web product is complete; build and HTTP smoke checks are not substitutes."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            "Start the app on an explicit, unused 127.0.0.1 port first and pass the tracked process_id returned by the background launch. When you did not author or inspect the current DOM, begin with an inspect action and use the returned ids, data-testid values, roles, labels, and text instead of guessing selectors. Enter form values with {\"action\":\"fill\",\"selector\":\"#title\",\"value\":\"Database latency\"}; fill supports text inputs, textareas, and selects and emits input/change events. Use {\"action\":\"reload\"} before asserting persisted state. Prefer the canonical structured assertion {\"action\":\"assert_text\",\"selector\":\"#status\",\"text\":\"paused\",\"match\":\"equals\"}; use match=contains for a substring. It compares both rendered and semantic DOM text, so CSS-only capitalization does not cause a false miss. Reserve arbitrary JavaScript assert for state that cannot be expressed structurally. Supply interactions that exercise the primary user journey. Keys accept named values such as ArrowLeft, Space, and Escape, DOM codes such as KeyP and Digit7, or one printable character. A successful result is canonical runtime evidence only for the interaction modalities listed in its report; do not claim keyboard testing from a click-only run. If the browser is unavailable, report that acceptance was not run rather than installing another browser runtime.",
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Loopback http/https URL, such as http://127.0.0.1:4173/"
                },
                "process_id": {
                    "type": "string",
                    "description": "Tracked preview process returned by the background launch. When supplied, its session, project, active state, and endpoint must match this check."
                },
                "viewports": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_VIEWPORTS,
                    "description": "Defaults to phone 390x844 and desktop 1280x720",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "width": {"type": "integer", "minimum": 240, "maximum": 3840},
                            "height": {"type": "integer", "minimum": 240, "maximum": 2160},
                            "device_scale_factor": {"type": "number", "minimum": 0.5, "maximum": 4},
                            "mobile": {"type": "boolean"}
                        },
                        "required": ["width", "height"],
                        "additionalProperties": false
                    }
                },
                "actions": {
                    "type": "array",
                    "maxItems": MAX_ACTIONS,
                    "description": "Repeatable primary-journey steps run independently in every viewport",
                    "items": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {"action": {"const": "click"}, "selector": {"type": "string"}},
                                "required": ["action", "selector"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "action": {"const": "fill"},
                                    "selector": {"type": "string"},
                                    "value": {"type": "string", "maxLength": MAX_FILL_CHARS}
                                },
                                "required": ["action", "selector", "value"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {"action": {"const": "reload"}},
                                "required": ["action"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {"action": {"const": "key"}, "key": {"type": "string"}},
                                "required": ["action", "key"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {"action": {"const": "wait"}, "ms": {"type": "integer", "minimum": 0, "maximum": MAX_SETTLE_MS}},
                                "required": ["action", "ms"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "action": {"const": "assert"},
                                    "expression": {"type": "string", "description": "JavaScript expression that must evaluate truthy"},
                                    "message": {"type": "string"}
                                },
                                "required": ["action", "expression", "message"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {"action": {"const": "visible"}, "selector": {"type": "string"}},
                                "required": ["action", "selector"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "action": {"const": "inspect"},
                                    "selector": {"type": "string", "description": "Optional CSS selector. Omit to discover bounded interactive/text landmarks across the page."}
                                },
                                "required": ["action"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "action": {"const": "assert_text"},
                                    "selector": {"type": "string"},
                                    "text": {"type": "string", "description": "Expected visible or semantic DOM text"},
                                    "match": {"type": "string", "enum": ["equals", "contains"], "default": "equals"}
                                },
                                "required": ["action", "selector", "text"],
                                "additionalProperties": false
                            }
                        ]
                    }
                },
                "settle_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_SETTLE_MS,
                    "description": "Quiet time after navigation and each interaction (default 500ms)"
                },
                "require_service_worker": {
                    "type": "boolean",
                    "description": "Require navigator.serviceWorker.ready before acceptance passes"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<BrowserCheckParams>(params) {
            Ok(params) => params,
            Err(error) => return error,
        };
        if let Err(error) = validate_params(&params) {
            return ToolResult::invalid_parameters(error);
        }
        let bound_process_id = match validate_process_binding(&params, ctx).await {
            Ok(process_id) => process_id,
            Err(error) => return error,
        };

        match run_browser_check(params).await {
            Ok(mut report) => {
                if let (Some(process_id), Some(report)) = (bound_process_id, report.as_object_mut())
                {
                    report.insert("process_id".to_string(), Value::String(process_id));
                    report.insert("process_bound".to_string(), Value::Bool(true));
                }
                ToolResult::success_data(report)
            }
            Err(error) => ToolResult::error_with_details(
                "browser_acceptance_failed",
                error.to_string(),
                None,
                Some(json!({
                    "acceptance_tier": "browser_runtime",
                    "failure_class": classify_browser_failure(&error),
                })),
            ),
        }
    }
}

async fn validate_process_binding(
    params: &BrowserCheckParams,
    ctx: &ToolContext,
) -> std::result::Result<Option<String>, ToolResult> {
    let Some(registry) = ctx.process_registry.as_ref() else {
        return Ok(None);
    };
    let url = Url::parse(&params.url).map_err(ToolResult::invalid_parameters)?;
    let Some(port) = url.port_or_known_default() else {
        return Err(ToolResult::invalid_parameters(
            "browser_check URL does not identify a port",
        ));
    };
    let processes = match ctx.effective_process_owner_id() {
        Some(user_id) => registry.list_for_user(user_id).await,
        None => registry.list().await,
    };
    let requested = params
        .process_id
        .as_deref()
        .filter(|process_id| !process_id.trim().is_empty());
    let mut endpoint_owners = processes
        .iter()
        .filter(|process| {
            process.is_active()
                && background_endpoint_hints(&process.command)
                    .iter()
                    .any(|endpoint| endpoint_port(endpoint) == Some(port))
        })
        .collect::<Vec<_>>();

    let owner = if let Some(process_id) = requested {
        let Some(process) = processes.iter().find(|process| process.id == process_id) else {
            return Err(ToolResult::error_with_details(
                "browser_process_not_found",
                format!("Tracked preview process {process_id} was not found"),
                Some(json!({
                    "process_id": process_id,
                    "next_action": "Start or reuse this project's tracked preview and pass the returned process_id."
                })),
                None,
            ));
        };
        if !process.is_active() {
            return Err(ToolResult::error_with_details(
                "browser_process_not_active",
                format!(
                    "Tracked preview process {process_id} is {}",
                    process.display_status()
                ),
                Some(json!({"process_id": process_id})),
                None,
            ));
        }
        if !endpoint_owners.iter().any(|owner| owner.id == process.id) {
            return Err(ToolResult::error_with_details(
                "browser_process_endpoint_mismatch",
                format!("Tracked preview process {process_id} does not own loopback port {port}"),
                Some(json!({
                    "process_id": process_id,
                    "url": params.url,
                    "next_action": "Use the endpoint advertised by this exact tracked process."
                })),
                None,
            ));
        }
        process
    } else {
        if endpoint_owners.len() > 1 {
            return Err(ToolResult::error_with_details(
                "browser_endpoint_ambiguous",
                format!("Multiple tracked processes claim loopback port {port}"),
                Some(json!({
                    "url": params.url,
                    "process_ids": endpoint_owners
                        .iter()
                        .map(|process| process.id.as_str())
                        .collect::<Vec<_>>(),
                    "next_action": "Pass the process_id returned by this project's background launch."
                })),
                None,
            ));
        }
        let Some(process) = endpoint_owners.pop() else {
            // Manually managed loopback services remain valid for local and
            // test use. Tracked previews are owner-checked whenever present.
            return Ok(None);
        };
        process
    };

    // A delegated task has an execution-scoped process owner and deliberately
    // omits the parent session ID so process completion cannot enqueue a
    // parent process wake. `list_for_user` above already restricted the lookup
    // to that unforgeable task owner, so requiring a session match here would
    // reject the task's own preview. Ordinary parent/user-owned previews still
    // require the original session boundary.
    let session_matches = ctx.process_owner_id.is_some()
        || match (ctx.session_id.as_deref(), owner.session_id.as_deref()) {
            (Some(current), Some(process)) => current == process,
            (None, None) => true,
            _ => false,
        };
    let working_dir_matches =
        normalized_path(&ctx.working_dir) == normalized_path(&owner._working_dir);
    if !session_matches || !working_dir_matches {
        return Err(ToolResult::error_with_details(
            "browser_endpoint_owned_by_foreign_process",
            format!("Loopback port {port} belongs to another tracked preview"),
            Some(json!({
                "url": params.url,
                "process_id": owner.id,
                "process_session_id": owner.session_id,
                "process_working_dir": owner._working_dir,
                "current_session_id": ctx.session_id,
                "current_working_dir": ctx.working_dir,
                "next_action": "Start this project's preview on an unused loopback port and pass its returned process_id."
            })),
            None,
        ));
    }

    Ok(Some(owner.id.clone()))
}

fn endpoint_port(endpoint: &str) -> Option<u16> {
    endpoint.rsplit_once(':')?.1.parse().ok()
}

fn normalized_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn validate_params(params: &BrowserCheckParams) -> Result<()> {
    validate_loopback_url(&params.url)?;
    if params.viewports.is_empty() || params.viewports.len() > MAX_VIEWPORTS {
        bail!("viewports must contain between 1 and {MAX_VIEWPORTS} entries");
    }
    if params.actions.len() > MAX_ACTIONS {
        bail!("actions exceeds the {MAX_ACTIONS}-step limit");
    }
    if params.settle_ms > MAX_SETTLE_MS {
        bail!("settle_ms exceeds {MAX_SETTLE_MS}");
    }
    for viewport in &params.viewports {
        if !(240..=3840).contains(&viewport.width)
            || !(240..=2160).contains(&viewport.height)
            || !(0.5..=4.0).contains(&viewport.device_scale_factor)
        {
            bail!("viewport dimensions or device scale factor are out of bounds");
        }
    }
    for action in &params.actions {
        match action {
            BrowserAction::Click { selector }
            | BrowserAction::Fill { selector, .. }
            | BrowserAction::Visible { selector }
            | BrowserAction::AssertText { selector, .. }
                if selector.trim().is_empty() =>
            {
                bail!("browser selectors cannot be empty")
            }
            BrowserAction::Key { key } => {
                key_event_descriptor(key).context("invalid browser key")?;
            }
            BrowserAction::Fill { value, .. } if value.chars().count() > MAX_FILL_CHARS => {
                bail!("browser fill value exceeds {MAX_FILL_CHARS} characters")
            }
            BrowserAction::Wait { ms } if *ms > MAX_SETTLE_MS => {
                bail!("action wait exceeds {MAX_SETTLE_MS}ms")
            }
            BrowserAction::Assert {
                expression,
                message,
            } if expression.trim().is_empty() || message.trim().is_empty() => {
                bail!("browser assertions require an expression and failure message")
            }
            BrowserAction::Inspect {
                selector: Some(selector),
            } if selector.trim().is_empty() => {
                bail!("browser inspect selector cannot be empty")
            }
            BrowserAction::AssertText {
                text,
                match_mode,
                equals,
                contains,
                expect,
                ..
            } => {
                resolve_text_expectation(
                    text.as_deref(),
                    *match_mode,
                    equals.as_deref(),
                    contains.as_deref(),
                    expect.as_deref(),
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve_text_expectation<'a>(
    text: Option<&'a str>,
    match_mode: Option<TextMatchMode>,
    equals: Option<&'a str>,
    contains: Option<&'a str>,
    expect: Option<&'a str>,
) -> Result<(&'a str, TextMatchMode)> {
    let supplied = [text, equals, contains, expect]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if supplied.len() != 1 || supplied[0].trim().is_empty() {
        bail!(
            "assert_text requires canonical text plus optional match=equals|contains (legacy equals, contains, or expect are also accepted)"
        );
    }
    if text.is_none() && match_mode.is_some() {
        bail!("assert_text match is only valid with canonical text");
    }
    let mode = if contains.is_some() {
        TextMatchMode::Contains
    } else if text.is_some() {
        match_mode.unwrap_or(TextMatchMode::Equals)
    } else {
        TextMatchMode::Equals
    };
    Ok((supplied[0], mode))
}

fn text_assertion_matches(
    rendered: Option<&str>,
    semantic: Option<&str>,
    expected: &str,
    mode: TextMatchMode,
) -> bool {
    let matches = |actual: Option<&str>| match mode {
        TextMatchMode::Equals => actual == Some(expected),
        TextMatchMode::Contains => actual.is_some_and(|actual| actual.contains(expected)),
    };
    matches(rendered) || matches(semantic)
}

fn classify_browser_failure(error: &anyhow::Error) -> &'static str {
    let message = error.to_string();
    if message.contains("browser assertion failed")
        || message.contains("browser text assertion failed")
        || message.contains("selector is not visible")
        || message.contains("selector not found")
        || message.contains("service worker was not ready")
    {
        "acceptance_miss"
    } else if message.contains("browser runtime errors")
        || message.contains("page exception")
        || message.contains("browser target crashed")
    {
        "page_runtime_error"
    } else {
        "browser_runtime_error"
    }
}

fn validate_loopback_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).context("browser_check requires a valid URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("browser_check only supports http and https URLs");
    }
    let host = url.host_str().unwrap_or_default();
    if !matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]") {
        bail!("browser_check is restricted to loopback URLs");
    }
    Ok(url)
}

async fn run_browser_check(params: BrowserCheckParams) -> Result<Value> {
    let binary = discover_browser_binary()?;
    let profile = std::env::temp_dir()
        .join("mitsuro-browser-check")
        .join(Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&profile)
        .await
        .with_context(|| format!("create browser profile {}", profile.display()))?;
    let mut browser = BrowserProcess::launch(&binary, &profile).await?;
    let result = run_with_browser(&mut browser, &params).await;
    browser.stop().await;
    if let Err(error) = tokio::fs::remove_dir_all(&profile).await {
        tracing::debug!(path = %profile.display(), %error, "Could not remove browser-check profile");
    }
    result
}

async fn run_with_browser(
    browser: &mut BrowserProcess,
    params: &BrowserCheckParams,
) -> Result<Value> {
    let mut cdp = CdpClient::connect(browser.debug_port).await?;
    cdp.call("Page.enable", json!({})).await?;
    cdp.call("Runtime.enable", json!({})).await?;
    cdp.call("Log.enable", json!({})).await?;
    cdp.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({
            "source": r#"(() => {
                const errors = [];
                Object.defineProperty(globalThis, "__mitsuroAcceptanceErrors", { value: errors });
                const originalError = console.error.bind(console);
                console.error = (...args) => {
                    errors.push(args.map((value) => String(value)).join(" "));
                    originalError(...args);
                };
                addEventListener("error", (event) => errors.push(String(event.error?.stack || event.message || "page error")));
                addEventListener("unhandledrejection", (event) => errors.push(String(event.reason?.stack || event.reason || "unhandled rejection")));
            })();"#
        }),
    )
    .await?;

    let mut viewport_reports = Vec::new();
    let mut report_warnings = Vec::new();
    for (index, viewport) in params.viewports.iter().enumerate() {
        cdp.errors.clear();
        cdp.resource_errors.clear();
        cdp.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": viewport.width,
                "height": viewport.height,
                "deviceScaleFactor": viewport.device_scale_factor,
                "mobile": viewport.mobile,
            }),
        )
        .await?;
        cdp.call("Page.navigate", json!({"url": params.url}))
            .await?;
        cdp.wait_for_document(&params.url).await?;
        cdp.drain_for(Duration::from_millis(params.settle_ms))
            .await?;

        if params.require_service_worker {
            let service_worker_ready = cdp
                .evaluate(
                    "Promise.race([navigator.serviceWorker.ready.then(() => true), new Promise(resolve => setTimeout(() => resolve(false), 5000))])",
                    true,
                )
                .await?;
            if service_worker_ready != Value::Bool(true) {
                bail!(
                    "service worker was not ready in viewport {}",
                    viewport_label(viewport, index)
                );
            }
        }

        let mut completed_actions = 0usize;
        let mut observations = Vec::new();
        for (action_index, action) in params.actions.iter().enumerate() {
            if let Some(observation) = cdp.perform_action(action).await? {
                observations.push(json!({
                    "action_index": action_index,
                    "observation": observation,
                }));
            }
            completed_actions += 1;
            cdp.drain_for(Duration::from_millis(params.settle_ms))
                .await?;
        }
        cdp.drain_for(Duration::from_millis(250)).await?;
        if let Value::Array(errors) = cdp
            .evaluate("globalThis.__mitsuroAcceptanceErrors || []", false)
            .await?
        {
            cdp.errors.extend(errors.into_iter().map(|error| {
                error
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| error.to_string())
            }));
        }
        let declared_icons = cdp
            .evaluate(
                r#"Array.from(document.querySelectorAll('link[rel~="icon"]')).map(link => link.href)"#,
                false,
            )
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        let mut viewport_warnings = Vec::new();
        for resource in std::mem::take(&mut cdp.resource_errors) {
            if is_implicit_favicon_404(&resource, &declared_icons) {
                viewport_warnings.push(format!(
                    "ignored implicit browser favicon request: {}",
                    resource.url
                ));
            } else {
                cdp.errors.push(format!(
                    "resource error [{}] {}: {}",
                    resource.source, resource.url, resource.text
                ));
            }
        }
        cdp.errors.sort();
        cdp.errors.dedup();
        viewport_warnings.sort();
        viewport_warnings.dedup();
        if !cdp.errors.is_empty() {
            bail!(
                "browser runtime errors in {}: {}",
                viewport_label(viewport, index),
                cdp.errors.join(" | ")
            );
        }

        viewport_reports.push(json!({
            "name": viewport_label(viewport, index),
            "width": viewport.width,
            "height": viewport.height,
            "mobile": viewport.mobile,
            "actions_completed": completed_actions,
            "console_errors": 0,
            "page_exceptions": 0,
            "service_worker_ready": params.require_service_worker.then_some(true),
            "observations": observations,
            "warnings": viewport_warnings.clone(),
        }));
        report_warnings.extend(viewport_warnings);
    }

    report_warnings.sort();
    report_warnings.dedup();

    Ok(json!({
        "url": params.url,
        "browser_binary": binary_display_name(&browser.binary),
        "acceptance_tier": "browser_runtime",
        "viewports": viewport_reports,
        "viewports_passed": params.viewports.len(),
        "actions_per_viewport": params.actions.len(),
        "click_actions_per_viewport": params.actions.iter().filter(|action| matches!(action, BrowserAction::Click { .. })).count(),
        "fill_actions_per_viewport": params.actions.iter().filter(|action| matches!(action, BrowserAction::Fill { .. })).count(),
        "reload_actions_per_viewport": params.actions.iter().filter(|action| matches!(action, BrowserAction::Reload)).count(),
        "key_actions_per_viewport": params.actions.iter().filter(|action| matches!(action, BrowserAction::Key { .. })).count(),
        "inspect_actions_per_viewport": params.actions.iter().filter(|action| matches!(action, BrowserAction::Inspect { .. })).count(),
        "structured_assertions_per_viewport": params.actions.iter().filter(|action| matches!(action, BrowserAction::AssertText { .. })).count(),
        "console_errors": 0,
        "page_exceptions": 0,
        "service_worker_required": params.require_service_worker,
        "warnings": report_warnings,
    }))
}

fn viewport_label(viewport: &Viewport, index: usize) -> String {
    viewport
        .name
        .clone()
        .unwrap_or_else(|| format!("viewport-{}", index + 1))
}

fn discover_browser_binary() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("MITSURO_BROWSER_BINARY") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!("MITSURO_BROWSER_BINARY does not point to a file");
    }

    for candidate in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "msedge",
    ] {
        if let Ok(path) = which::which(candidate) {
            return Ok(path);
        }
    }

    for candidate in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }

    bail!("no installed Chromium-family browser was found; runtime acceptance was not run")
}

fn binary_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("chromium")
        .to_string()
}

struct BrowserProcess {
    child: Child,
    binary: PathBuf,
    debug_port: u16,
}

impl BrowserProcess {
    async fn launch(binary: &Path, profile: &Path) -> Result<Self> {
        let mut child = Command::new(binary)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--disable-extensions",
                "--no-first-run",
                "--no-default-browser-check",
                "--remote-debugging-port=0",
                "--remote-allow-origins=*",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("launch browser {}", binary.display()))?;

        let active_port = profile.join("DevToolsActivePort");
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let debug_port = loop {
            if let Some(status) = child.try_wait().context("poll browser startup")? {
                bail!("browser exited during startup with {status}");
            }
            if let Ok(contents) = tokio::fs::read_to_string(&active_port).await {
                if let Some(port) = contents
                    .lines()
                    .next()
                    .and_then(|line| line.trim().parse::<u16>().ok())
                {
                    break port;
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill().await;
                bail!("browser did not expose its debugging endpoint within 10 seconds");
            }
            sleep(Duration::from_millis(50)).await;
        };

        Ok(Self {
            child,
            binary: binary.to_path_buf(),
            debug_port,
        })
    }

    async fn stop(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct CdpClient {
    socket: CdpSocket,
    next_id: u64,
    errors: Vec<String>,
    resource_errors: Vec<BrowserResourceError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserResourceError {
    source: String,
    url: String,
    text: String,
}

fn is_implicit_favicon_404(resource: &BrowserResourceError, declared_icons: &[String]) -> bool {
    resource.source == "network"
        && resource.text.contains("404")
        && Url::parse(&resource.url)
            .ok()
            .is_some_and(|url| url.path() == "/favicon.ico")
        && declared_icons.is_empty()
}

impl CdpClient {
    async fn connect(port: u16) -> Result<Self> {
        let endpoint = format!("http://127.0.0.1:{port}/json/list");
        let targets = timeout(COMMAND_TIMEOUT, reqwest::get(endpoint))
            .await
            .context("timed out reading browser targets")??
            .error_for_status()?
            .json::<Vec<Value>>()
            .await?;
        let websocket = targets
            .iter()
            .find(|target| target.get("type").and_then(Value::as_str) == Some("page"))
            .and_then(|target| target.get("webSocketDebuggerUrl"))
            .and_then(Value::as_str)
            .context("browser did not expose a page debugging target")?;
        let (socket, _) = connect_async(websocket).await?;
        Ok(Self {
            socket,
            next_id: 1,
            errors: Vec::new(),
            resource_errors: Vec::new(),
        })
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.socket
            .send(Message::Text(
                json!({"id": id, "method": method, "params": params}).to_string(),
            ))
            .await?;

        timeout(COMMAND_TIMEOUT, async {
            loop {
                let message = self
                    .socket
                    .next()
                    .await
                    .context("browser debugging socket closed")??;
                let Some(value) = message_json(message)? else {
                    continue;
                };
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    if let Some(error) = value.get("error") {
                        bail!("browser command {method} failed: {error}");
                    }
                    return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                }
                self.record_event(&value);
            }
        })
        .await
        .with_context(|| format!("browser command {method} timed out"))?
    }

    async fn wait_for_document(&mut self, expected_url: &str) -> Result<()> {
        let expected_url = serde_json::to_string(expected_url)?;
        let deadline = Instant::now() + LOAD_TIMEOUT;
        loop {
            let last_state = self
                .evaluate(
                    &format!(
                        "({{ready: document.readyState === 'complete' && location.href === {expected_url}, readyState: document.readyState, href: location.href, title: document.title}})"
                    ),
                    false,
                )
                .await?;
            if last_state.get("ready").and_then(Value::as_bool) == Some(true) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "page did not finish loading the requested URL within 15 seconds; last state: {last_state}"
                );
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    async fn drain_for(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(());
            };
            match timeout(
                remaining.min(Duration::from_millis(100)),
                self.socket.next(),
            )
            .await
            {
                Ok(Some(message)) => {
                    if let Some(value) = message_json(message?)? {
                        self.record_event(&value);
                    }
                }
                Ok(None) => bail!("browser debugging socket closed"),
                Err(_) if Instant::now() >= deadline => return Ok(()),
                Err(_) => {}
            }
        }
    }

    async fn evaluate(&mut self, expression: &str, await_promise: bool) -> Result<Value> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": await_promise,
                    "returnByValue": true,
                    "userGesture": true,
                }),
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            bail!(
                "browser evaluation failed: {}",
                exception_preview(exception)
            );
        }
        Ok(result
            .get("result")
            .and_then(|remote| remote.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn perform_action(&mut self, action: &BrowserAction) -> Result<Option<Value>> {
        let observation = match action {
            BrowserAction::Click { selector } => {
                let selector = serde_json::to_string(selector)?;
                self.evaluate(
                    &format!(
                        "(() => {{ const element = document.querySelector({selector}); if (!element) throw new Error('selector not found: ' + {selector}); element.click(); return true; }})()"
                    ),
                    false,
                )
                .await?;
                None
            }
            BrowserAction::Fill { selector, value } => {
                let selector = serde_json::to_string(selector)?;
                let value = serde_json::to_string(value)?;
                self.evaluate(
                    &format!(
                        r#"(() => {{
                            const element = document.querySelector({selector});
                            if (!element) throw new Error('selector not found: ' + {selector});
                            const value = {value};
                            element.focus();
                            if (element instanceof HTMLInputElement) {{
                                if (['checkbox', 'radio', 'file'].includes(element.type)) {{
                                    throw new Error('fill does not support input type ' + element.type);
                                }}
                                const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
                                if (!setter) throw new Error('input value setter is unavailable');
                                setter.call(element, value);
                            }} else if (element instanceof HTMLTextAreaElement) {{
                                const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
                                if (!setter) throw new Error('textarea value setter is unavailable');
                                setter.call(element, value);
                            }} else if (element instanceof HTMLSelectElement) {{
                                if (!Array.from(element.options).some(option => option.value === value)) {{
                                    throw new Error('select has no option with value: ' + value);
                                }}
                                const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set;
                                if (!setter) throw new Error('select value setter is unavailable');
                                setter.call(element, value);
                            }} else {{
                                throw new Error('fill requires an input, textarea, or select element');
                            }}
                            element.dispatchEvent(new Event('input', {{bubbles: true, composed: true}}));
                            element.dispatchEvent(new Event('change', {{bubbles: true, composed: true}}));
                            return true;
                        }})()"#
                    ),
                    false,
                )
                .await?;
                None
            }
            BrowserAction::Reload => {
                let current_url = self
                    .evaluate("location.href", false)
                    .await?
                    .as_str()
                    .context("browser page has no current URL")?
                    .to_string();
                self.call("Page.reload", json!({"ignoreCache": false}))
                    .await?;
                self.wait_for_document(&current_url).await?;
                None
            }
            BrowserAction::Key { key } => {
                let key = key_event_descriptor(key)?;
                let mut key_down = json!({
                    "type": "keyDown",
                    "key": &key.key,
                    "code": &key.code,
                    "windowsVirtualKeyCode": key.virtual_key_code,
                    "nativeVirtualKeyCode": key.virtual_key_code,
                });
                if let Some(text) = key.text {
                    key_down["text"] = Value::String(text.clone());
                    key_down["unmodifiedText"] = Value::String(text);
                }
                self.call("Input.dispatchKeyEvent", key_down).await?;
                self.call(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": "keyUp",
                        "key": &key.key,
                        "code": &key.code,
                        "windowsVirtualKeyCode": key.virtual_key_code,
                        "nativeVirtualKeyCode": key.virtual_key_code,
                    }),
                )
                .await?;
                None
            }
            BrowserAction::Wait { ms } => {
                sleep(Duration::from_millis(*ms)).await;
                None
            }
            BrowserAction::Assert {
                expression,
                message,
            } => {
                if self.evaluate(expression, true).await? != Value::Bool(true) {
                    bail!(
                        "browser assertion failed: {message}; expression evaluated to false: {expression}"
                    );
                }
                None
            }
            BrowserAction::Visible { selector } => {
                let selector = serde_json::to_string(selector)?;
                let visible = self
                    .evaluate(
                        &format!(
                            "(() => {{ const element = document.querySelector({selector}); if (!element) return false; const style = getComputedStyle(element); const rect = element.getBoundingClientRect(); return style.visibility !== 'hidden' && style.display !== 'none' && rect.width > 0 && rect.height > 0; }})()"
                        ),
                        false,
                    )
                    .await?;
                if visible != Value::Bool(true) {
                    bail!("selector is not visible: {}", selector.trim_matches('"'));
                }
                None
            }
            BrowserAction::Inspect { selector } => {
                let selector = serde_json::to_string(selector)?;
                let report = self
                    .evaluate(
                        &format!(
                            r#"(() => {{
                                const selector = {selector};
                                const candidates = selector
                                    ? Array.from(document.querySelectorAll(selector))
                                    : Array.from(document.querySelectorAll('[id], [data-testid], button, a, input, select, textarea, [role], h1, h2, h3, [aria-label]'));
                                const describe = element => {{
                                    const rect = element.getBoundingClientRect();
                                    const style = getComputedStyle(element);
                                    return {{
                                        tag: element.tagName.toLowerCase(),
                                        id: element.id || null,
                                        testid: element.getAttribute('data-testid'),
                                        role: element.getAttribute('role'),
                                        label: element.getAttribute('aria-label'),
                                        text: (element.innerText || element.textContent || '').trim().slice(0, 240),
                                        visible: style.visibility !== 'hidden' && style.display !== 'none' && rect.width > 0 && rect.height > 0
                                    }};
                                }};
                                return {{
                                    selector,
                                    title: document.title,
                                    url: location.href,
                                    body_text: (document.body?.innerText || '').trim().slice(0, 1200),
                                    match_count: candidates.length,
                                    elements: candidates.slice(0, 48).map(describe)
                                }};
                            }})()"#
                        ),
                        false,
                    )
                    .await?;
                Some(report)
            }
            BrowserAction::AssertText {
                selector,
                text,
                match_mode,
                equals,
                contains,
                expect,
            } => {
                let (expected, mode) = resolve_text_expectation(
                    text.as_deref(),
                    *match_mode,
                    equals.as_deref(),
                    contains.as_deref(),
                    expect.as_deref(),
                )?;
                let encoded_selector = serde_json::to_string(selector)?;
                let observed = self
                    .evaluate(
                        &format!(
                            "(() => {{ const element = document.querySelector({encoded_selector}); return {{found: Boolean(element), rendered_text: element ? (element.innerText || '').trim() : null, semantic_text: element ? (element.textContent || '').trim() : null}}; }})()"
                        ),
                        false,
                    )
                    .await?;
                let rendered = observed.get("rendered_text").and_then(Value::as_str);
                let semantic = observed.get("semantic_text").and_then(Value::as_str);
                let passed = text_assertion_matches(rendered, semantic, expected, mode);
                if !passed {
                    bail!(
                        "browser text assertion failed: selector={}, expected={}, actual={}",
                        serde_json::to_string(selector)?,
                        serde_json::to_string(&json!({"text": expected, "match": mode}))?,
                        serde_json::to_string(&observed)?,
                    );
                }
                Some(json!({
                    "selector": selector,
                    "expected": {"text": expected, "match": mode},
                    "actual": {"rendered_text": rendered, "semantic_text": semantic},
                }))
            }
        };
        Ok(observation)
    }

    fn record_event(&mut self, event: &Value) {
        match event.get("method").and_then(Value::as_str) {
            Some("Runtime.exceptionThrown") => {
                let details = event
                    .pointer("/params/exceptionDetails")
                    .map(exception_preview)
                    .unwrap_or_else(|| "uncaught page exception".to_string());
                self.errors.push(details);
            }
            Some("Runtime.consoleAPICalled") => {
                let kind = event.pointer("/params/type").and_then(Value::as_str);
                if matches!(kind, Some("error" | "assert")) {
                    let args = event
                        .pointer("/params/args")
                        .and_then(Value::as_array)
                        .map(|args| {
                            args.iter()
                                .filter_map(|arg| {
                                    arg.get("value").map(Value::to_string).or_else(|| {
                                        arg.get("description")
                                            .and_then(Value::as_str)
                                            .map(str::to_string)
                                    })
                                })
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    self.errors.push(format!("console {kind:?}: {args}"));
                }
            }
            Some("Log.entryAdded") => {
                let level = event.pointer("/params/entry/level").and_then(Value::as_str);
                if matches!(level, Some("error")) {
                    let text = event
                        .pointer("/params/entry/text")
                        .and_then(Value::as_str)
                        .unwrap_or("browser log error");
                    let source = event
                        .pointer("/params/entry/source")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let url = event
                        .pointer("/params/entry/url")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if source == "network" || !url.is_empty() {
                        self.resource_errors.push(BrowserResourceError {
                            source: source.to_string(),
                            url: url.to_string(),
                            text: text.to_string(),
                        });
                    } else {
                        self.errors.push(text.to_string());
                    }
                }
            }
            Some("Inspector.targetCrashed") => {
                self.errors.push("browser target crashed".to_string())
            }
            _ => {}
        }
    }
}

struct KeyEventDescriptor {
    key: String,
    code: String,
    virtual_key_code: u32,
    text: Option<String>,
}

fn key_event_descriptor(raw: &str) -> Result<KeyEventDescriptor> {
    let named = match raw {
        "ArrowLeft" => Some(("ArrowLeft", "ArrowLeft", 37)),
        "ArrowUp" => Some(("ArrowUp", "ArrowUp", 38)),
        "ArrowRight" => Some(("ArrowRight", "ArrowRight", 39)),
        "ArrowDown" => Some(("ArrowDown", "ArrowDown", 40)),
        "Space" | "Spacebar" => Some((" ", "Space", 32)),
        "Enter" => Some(("Enter", "Enter", 13)),
        "Escape" | "Esc" => Some(("Escape", "Escape", 27)),
        "Tab" => Some(("Tab", "Tab", 9)),
        "Backspace" => Some(("Backspace", "Backspace", 8)),
        "Delete" => Some(("Delete", "Delete", 46)),
        "Home" => Some(("Home", "Home", 36)),
        "End" => Some(("End", "End", 35)),
        "PageUp" => Some(("PageUp", "PageUp", 33)),
        "PageDown" => Some(("PageDown", "PageDown", 34)),
        _ => None,
    };
    if let Some((key, code, virtual_key_code)) = named {
        let text = (code == "Space").then(|| " ".to_string());
        return Ok(KeyEventDescriptor {
            key: key.to_string(),
            code: code.to_string(),
            virtual_key_code,
            text,
        });
    }

    if let Some(letter) = raw
        .strip_prefix("Key")
        .filter(|value| value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic())
    {
        let upper = letter.as_bytes()[0].to_ascii_uppercase() as char;
        let lower = upper.to_ascii_lowercase();
        return Ok(KeyEventDescriptor {
            key: lower.to_string(),
            code: format!("Key{upper}"),
            virtual_key_code: u32::from(upper),
            text: Some(lower.to_string()),
        });
    }
    if let Some(digit) = raw
        .strip_prefix("Digit")
        .filter(|value| value.len() == 1 && value.as_bytes()[0].is_ascii_digit())
    {
        let digit = digit.as_bytes()[0] as char;
        return Ok(KeyEventDescriptor {
            key: digit.to_string(),
            code: format!("Digit{digit}"),
            virtual_key_code: u32::from(digit),
            text: Some(digit.to_string()),
        });
    }

    let mut characters = raw.chars();
    let Some(character) = characters.next() else {
        bail!("browser keys cannot be empty");
    };
    if characters.next().is_some() || character.is_control() {
        bail!(
            "unsupported browser key '{raw}'; use ArrowLeft/Up/Right/Down, Space, Enter, Escape, Tab, Backspace, Delete, Home, End, PageUp, PageDown, KeyA..KeyZ, Digit0..Digit9, or one printable character"
        );
    }
    let upper = character.to_ascii_uppercase();
    let code = if character.is_ascii_alphabetic() {
        format!("Key{upper}")
    } else if character.is_ascii_digit() {
        format!("Digit{character}")
    } else {
        String::new()
    };
    Ok(KeyEventDescriptor {
        key: character.to_string(),
        code,
        virtual_key_code: u32::from(upper),
        text: Some(character.to_string()),
    })
}

fn message_json(message: Message) -> Result<Option<Value>> {
    match message {
        Message::Text(text) => Ok(Some(serde_json::from_str(text.as_ref())?)),
        Message::Binary(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Message::Close(_) => bail!("browser debugging socket closed"),
        _ => Ok(None),
    }
}

fn exception_preview(details: &Value) -> String {
    details
        .pointer("/exception/description")
        .and_then(Value::as_str)
        .or_else(|| details.get("text").and_then(Value::as_str))
        .unwrap_or("browser evaluation exception")
        .chars()
        .take(800)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use crate::process::ProcessRegistry;

    use super::*;

    static BROWSER_TEST_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
        once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

    #[test]
    fn loopback_url_policy_rejects_remote_hosts_and_non_http_schemes() {
        assert!(validate_loopback_url("http://127.0.0.1:4173/").is_ok());
        assert!(validate_loopback_url("https://localhost:8787/app").is_ok());
        assert!(validate_loopback_url("http://[::1]:3000/").is_ok());
        assert!(validate_loopback_url("https://example.com/").is_err());
        assert!(validate_loopback_url("file:///tmp/index.html").is_err());
    }

    #[test]
    fn default_contract_checks_phone_and_desktop() {
        let viewports = default_viewports();
        assert_eq!(viewports.len(), 2);
        assert!(viewports[0].mobile);
        assert!(!viewports[1].mobile);
    }

    #[test]
    fn schema_exposes_runtime_actions_without_a_browser_binary_override() {
        let schema = BrowserCheckTool.parameters_schema();
        assert_eq!(BrowserCheckTool.name(), "browser_check");
        assert!(schema["properties"].get("actions").is_some());
        assert!(schema["properties"].get("process_id").is_some());
        assert!(schema["properties"].get("browser_binary").is_none());
        let actions = schema["properties"]["actions"]["items"]["oneOf"]
            .as_array()
            .expect("action variants");
        assert!(actions.iter().any(|action| {
            action["properties"]["action"]["const"] == Value::String("inspect".to_string())
        }));
        let fill = actions
            .iter()
            .find(|action| {
                action["properties"]["action"]["const"] == Value::String("fill".to_string())
            })
            .expect("fill action");
        assert_eq!(fill["required"], json!(["action", "selector", "value"]));
        assert_eq!(fill["properties"]["value"]["maxLength"], MAX_FILL_CHARS);
        let reload = actions
            .iter()
            .find(|action| {
                action["properties"]["action"]["const"] == Value::String("reload".to_string())
            })
            .expect("reload action");
        assert_eq!(reload["required"], json!(["action"]));
        let assert_text = actions
            .iter()
            .find(|action| {
                action["properties"]["action"]["const"] == Value::String("assert_text".to_string())
            })
            .expect("assert_text action");
        assert_eq!(
            assert_text["required"],
            json!(["action", "selector", "text"])
        );
        assert_eq!(
            assert_text["properties"]["match"]["enum"],
            json!(["equals", "contains"])
        );
        assert!(assert_text["properties"].get("equals").is_none());
        assert!(assert_text["properties"].get("expect").is_none());
    }

    #[test]
    fn text_assertions_accept_canonical_and_legacy_forms_without_ambiguity() {
        assert_eq!(
            resolve_text_expectation(
                Some("paused"),
                Some(TextMatchMode::Contains),
                None,
                None,
                None,
            )
            .unwrap(),
            ("paused", TextMatchMode::Contains)
        );
        assert_eq!(
            resolve_text_expectation(None, None, Some("ready"), None, None).unwrap(),
            ("ready", TextMatchMode::Equals)
        );
        assert_eq!(
            resolve_text_expectation(None, None, None, Some("score"), None).unwrap(),
            ("score", TextMatchMode::Contains)
        );
        assert_eq!(
            resolve_text_expectation(None, None, None, None, Some("legacy")).unwrap(),
            ("legacy", TextMatchMode::Equals)
        );
        assert!(resolve_text_expectation(Some("one"), None, Some("two"), None, None,).is_err());
        assert_eq!(
            serde_json::from_value::<TextMatchMode>(json!("value")).unwrap(),
            TextMatchMode::Equals
        );
        assert_eq!(
            serde_json::from_value::<TextMatchMode>(json!("exact")).unwrap(),
            TextMatchMode::Equals
        );
    }

    #[test]
    fn text_assertions_check_rendered_and_semantic_dom_text() {
        assert!(text_assertion_matches(
            Some("GAME PAUSED"),
            Some("Game paused"),
            "Game paused",
            TextMatchMode::Equals,
        ));
        assert!(text_assertion_matches(
            Some("SCORE: 100"),
            None,
            "100",
            TextMatchMode::Contains,
        ));
        assert!(!text_assertion_matches(
            Some("RUNNING"),
            Some("Running"),
            "Paused",
            TextMatchMode::Equals,
        ));
    }

    #[test]
    fn favicon_warning_is_only_for_implicit_undeclared_request() {
        let resource = BrowserResourceError {
            source: "network".to_string(),
            url: "http://127.0.0.1:4173/favicon.ico".to_string(),
            text: "Failed to load resource: the server responded with a status of 404".to_string(),
        };
        assert!(is_implicit_favicon_404(&resource, &[]));
        assert!(!is_implicit_favicon_404(
            &resource,
            &["http://127.0.0.1:4173/favicon.ico".to_string()],
        ));
        let script = BrowserResourceError {
            url: "http://127.0.0.1:4173/app.js".to_string(),
            ..resource
        };
        assert!(!is_implicit_favicon_404(&script, &[]));
    }

    #[test]
    fn browser_failure_classification_separates_acceptance_from_runtime() {
        assert_eq!(
            classify_browser_failure(&anyhow::anyhow!(
                "browser text assertion failed: expected x actual y"
            )),
            "acceptance_miss"
        );
        assert_eq!(
            classify_browser_failure(&anyhow::anyhow!("browser runtime errors in phone: boom")),
            "page_runtime_error"
        );
        assert_eq!(
            classify_browser_failure(&anyhow::anyhow!("browser did not expose endpoint")),
            "browser_runtime_error"
        );
    }

    fn binding_params(port: u16, process_id: Option<String>) -> BrowserCheckParams {
        BrowserCheckParams {
            url: format!("http://127.0.0.1:{port}/"),
            process_id,
            viewports: default_viewports(),
            actions: Vec::new(),
            settle_ms: 10,
            require_service_worker: false,
        }
    }

    #[tokio::test]
    async fn browser_binding_rejects_foreign_session_preview_before_navigation() {
        let current = tempfile::TempDir::new().unwrap();
        let foreign = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ProcessRegistry::new());
        let process_id = registry
            .spawn_for_user(
                "test-user",
                "python3 -c 'import time; time.sleep(60)' --host 127.0.0.1 --port 5940".to_string(),
                foreign.path().to_path_buf(),
                Some("foreign preview".to_string()),
                Some("foreign-session".to_string()),
            )
            .await
            .unwrap();
        let mut ctx =
            ToolContext::with_process_registry(current.path().to_path_buf(), Arc::clone(&registry))
                .with_session_metadata("current-session".to_string(), current.path().join("db"));
        ctx.user_id = Some("test-user".to_string());

        let error = validate_process_binding(&binding_params(5940, None), &ctx)
            .await
            .expect_err("foreign preview must be rejected");
        let envelope: Value = serde_json::from_str(&error.output).unwrap();
        assert_eq!(
            envelope["error"]["code"],
            "browser_endpoint_owned_by_foreign_process"
        );
        registry
            .kill_for_user("test-user", &process_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn browser_binding_accepts_exact_session_project_and_process() {
        let project = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ProcessRegistry::new());
        let process_id = registry
            .spawn_for_user(
                "test-user",
                "python3 -c 'import time; time.sleep(60)' --host 127.0.0.1 --port 5941".to_string(),
                project.path().to_path_buf(),
                Some("owned preview".to_string()),
                Some("current-session".to_string()),
            )
            .await
            .unwrap();
        let mut ctx =
            ToolContext::with_process_registry(project.path().to_path_buf(), Arc::clone(&registry))
                .with_session_metadata("current-session".to_string(), project.path().join("db"));
        ctx.user_id = Some("test-user".to_string());

        let binding =
            validate_process_binding(&binding_params(5941, Some(process_id.clone())), &ctx)
                .await
                .expect("owned preview must bind");
        assert_eq!(binding.as_deref(), Some(process_id.as_str()));
        registry
            .kill_for_user("test-user", &process_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn browser_binding_accepts_exact_delegated_owner_without_process_session() {
        let project = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ProcessRegistry::new());
        let delegated_owner = "test-user:hive:task-owner";
        let process_id = registry
            .spawn_for_user(
                delegated_owner,
                "python3 -c 'import time; time.sleep(60)' --host 127.0.0.1 --port 5942".to_string(),
                project.path().to_path_buf(),
                Some("delegated preview".to_string()),
                None,
            )
            .await
            .unwrap();
        let mut ctx =
            ToolContext::with_process_registry(project.path().to_path_buf(), Arc::clone(&registry))
                .with_session_metadata("parent-session".to_string(), project.path().join("db"))
                .with_process_owner_id(delegated_owner.to_string());
        ctx.user_id = Some("test-user".to_string());

        let binding =
            validate_process_binding(&binding_params(5942, Some(process_id.clone())), &ctx)
                .await
                .expect("the exact delegated owner must bind its sessionless preview");
        assert_eq!(binding.as_deref(), Some(process_id.as_str()));
        registry
            .kill_for_user(delegated_owner, &process_id)
            .await
            .unwrap();
    }

    #[test]
    fn cdp_key_descriptors_omit_text_for_navigation_and_normalize_space() {
        let arrow = key_event_descriptor("ArrowLeft").expect("arrow key");
        assert_eq!(arrow.key, "ArrowLeft");
        assert_eq!(arrow.code, "ArrowLeft");
        assert_eq!(arrow.virtual_key_code, 37);
        assert_eq!(arrow.text, None);

        let space = key_event_descriptor("Space").expect("space key");
        assert_eq!(space.key, " ");
        assert_eq!(space.code, "Space");
        assert_eq!(space.text.as_deref(), Some(" "));

        let letter = key_event_descriptor("p").expect("printable key");
        assert_eq!(letter.code, "KeyP");
        assert_eq!(letter.text.as_deref(), Some("p"));

        let dom_letter = key_event_descriptor("KeyP").expect("DOM letter code");
        assert_eq!(dom_letter.key, "p");
        assert_eq!(dom_letter.code, "KeyP");
        assert_eq!(dom_letter.text.as_deref(), Some("p"));

        let dom_digit = key_event_descriptor("Digit7").expect("DOM digit code");
        assert_eq!(dom_digit.key, "7");
        assert_eq!(dom_digit.code, "Digit7");
        assert_eq!(dom_digit.text.as_deref(), Some("7"));

        assert!(key_event_descriptor("DefinitelyNotAKey").is_err());
    }

    #[tokio::test]
    async fn installed_browser_smoke_is_opt_in() {
        if std::env::var("MITSURO_RUN_BROWSER_ACCEPTANCE")
            .ok()
            .as_deref()
            != Some("1")
        {
            return;
        }
        let _guard = BROWSER_TEST_LOCK.lock().await;
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let server_done = Arc::clone(&done);
        let server_thread = std::thread::spawn(move || {
            while !server_done.load(Ordering::Relaxed) {
                let Some(request) = server
                    .recv_timeout(Duration::from_millis(100))
                    .expect("browser request")
                else {
                    continue;
                };
                let content_type =
                    tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                        .unwrap();
                request
                    .respond(
                        tiny_http::Response::from_string(
                            "<input id=name oninput=\"this.dataset.input='yes'\" onchange=\"this.dataset.change='yes'\"><button id=go onclick=\"this.dataset.hit='yes'\">Go</button><script>sessionStorage.loads=String(Number(sessionStorage.loads||0)+1);addEventListener('keydown',event=>document.body.dataset.key=event.key)</script>",
                        )
                        .with_header(content_type),
                    )
                    .unwrap();
            }
        });
        let report = run_browser_check(BrowserCheckParams {
            url: format!("http://{address}/"),
            process_id: None,
            viewports: vec![default_viewports().remove(0)],
            actions: vec![
                BrowserAction::Inspect { selector: None },
                BrowserAction::Click {
                    selector: "#go".to_string(),
                },
                BrowserAction::Fill {
                    selector: "#name".to_string(),
                    value: "Incident Alpha".to_string(),
                },
                BrowserAction::AssertText {
                    selector: "#go".to_string(),
                    text: Some("Go".to_string()),
                    match_mode: Some(TextMatchMode::Equals),
                    expect: None,
                    equals: None,
                    contains: None,
                },
                BrowserAction::Key {
                    key: "ArrowLeft".to_string(),
                },
                BrowserAction::Assert {
                    expression: "document.querySelector('#go').dataset.hit === 'yes' && document.body.dataset.key === 'ArrowLeft' && document.querySelector('#name').value === 'Incident Alpha' && document.querySelector('#name').dataset.input === 'yes' && document.querySelector('#name').dataset.change === 'yes'".to_string(),
                    message: "button, form fill, or keyboard input did not change state".to_string(),
                },
                BrowserAction::Reload,
                BrowserAction::Assert {
                    expression: "Number(sessionStorage.loads) >= 2".to_string(),
                    message: "reload did not create a second document load".to_string(),
                },
            ],
            settle_ms: 10,
            require_service_worker: false,
        })
        .await
        .expect("browser acceptance");
        assert_eq!(report["viewports_passed"], 1);
        assert_eq!(report["key_actions_per_viewport"], 1);
        assert_eq!(report["fill_actions_per_viewport"], 1);
        assert_eq!(report["reload_actions_per_viewport"], 1);
        assert_eq!(report["inspect_actions_per_viewport"], 1);
        assert_eq!(report["structured_assertions_per_viewport"], 1);
        assert_eq!(
            report["viewports"][0]["observations"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        done.store(true, Ordering::Relaxed);
        server_thread.join().unwrap();
    }

    #[tokio::test]
    async fn installed_browser_rejects_console_errors_when_opted_in() {
        if std::env::var("MITSURO_RUN_BROWSER_ACCEPTANCE")
            .ok()
            .as_deref()
            != Some("1")
        {
            return;
        }
        let _guard = BROWSER_TEST_LOCK.lock().await;
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let server_thread = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .expect("browser request");
            let content_type =
                tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
            request
                .respond(
                    tiny_http::Response::from_string(
                        "<script>console.error('acceptance sentinel')</script>",
                    )
                    .with_header(content_type),
                )
                .unwrap();
        });

        let error = run_browser_check(BrowserCheckParams {
            url: format!("http://{address}/"),
            process_id: None,
            viewports: vec![default_viewports().remove(0)],
            actions: Vec::new(),
            settle_ms: 100,
            require_service_worker: false,
        })
        .await
        .expect_err("console error must fail acceptance");

        assert!(error.to_string().contains("acceptance sentinel"));
        server_thread.join().unwrap();
    }
}
