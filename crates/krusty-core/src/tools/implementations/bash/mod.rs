//! Bash tool - Execute shell commands with real-time output streaming.

mod execution;
mod shell;
mod state_delta;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use crate::process::{ProcessInfo, ProcessRegistry, ProcessStatus};
use crate::tools::registry::Tool;
use crate::tools::truncation;
use crate::tools::{parse_params, ToolContext, ToolResult};

use execution::{execute_foreground, StreamContext};
use shell::{
    build_shell_command, configure_foreground_process_group, normalize_tracked_background_command,
    strip_ansi,
};

pub(super) const MAX_OUTPUT_LINES: usize = 2000;
pub(super) const MAX_OUTPUT_BYTES: usize = 50_000; // 50KB

// Bounded raw capture for foreground execution. Final model output is additionally
// truncated by MAX_OUTPUT_LINES/MAX_OUTPUT_BYTES after ANSI stripping.
pub(super) const RAW_CAPTURE_MAX_LINES: usize = 8_000;
pub(super) const RAW_CAPTURE_MAX_BYTES: usize = 2_000_000; // 2MB
                                                           // Reader tasks batch spool writes, but retain a generous bounded drain window
                                                           // so a busy or slow filesystem cannot silently discard the command's tail.
pub(super) const READER_JOIN_TIMEOUT_MS: u64 = 10_000;
pub(super) const TIMEOUT_KILL_GRACE_MS: u64 = 800;
const BACKGROUND_STARTUP_GRACE_MS: u64 = 250;

pub struct BashTool;

async fn background_start_result(
    registry: &ProcessRegistry,
    user_id: Option<&str>,
    process_id: String,
    endpoint_hints: Vec<String>,
    warnings: Vec<String>,
) -> ToolResult {
    tokio::time::sleep(Duration::from_millis(BACKGROUND_STARTUP_GRACE_MS)).await;
    let process = match user_id {
        Some(user_id) => registry.get_for_user(user_id, &process_id).await,
        None => registry.get(&process_id).await,
    };
    let Some(process) = process else {
        return ToolResult::error_with_details(
            "background_process_missing",
            "Background process was not registered after startup",
            Some(json!({"process_id": process_id})),
            None,
        );
    };

    match process.status {
        ProcessStatus::Running => ToolResult::success_data_with(
            json!({
                "message": "Process started in background",
                "process_id": process_id,
                "status": "running",
                "endpoint_hints": endpoint_hints,
                "next_action": "The process remains tracked after this turn. Use processes status/control when needed, and probe the advertised endpoint to verify service readiness."
            }),
            warnings,
            None,
            None,
        )
        .with_changed(true),
        ProcessStatus::Suspended => ToolResult::success_data_with(
            json!({
                "message": "Process started but is suspended",
                "process_id": process_id,
                "status": "suspended",
                "endpoint_hints": endpoint_hints
            }),
            warnings,
            None,
            None,
        )
        .with_changed(true),
        ProcessStatus::Completed {
            exit_code,
            duration_ms,
        } => ToolResult::success_data_with(
            json!({
                "message": "Background command completed during startup",
                "process_id": process_id,
                "status": "done",
                "exit_code": exit_code,
                "duration_ms": duration_ms
            }),
            warnings,
            None,
            None,
        ),
        ProcessStatus::Failed { error, duration_ms } => ToolResult::error_with_details(
            "background_start_failed",
            format!("Background process failed during startup: {error}"),
            Some(json!({
                "process_id": process_id,
                "status": "failed",
                "error": error,
                "duration_ms": duration_ms
            })),
            None,
        ),
        ProcessStatus::Killed { duration_ms } => ToolResult::error_with_details(
            "background_start_killed",
            "Background process was killed during startup",
            Some(json!({
                "process_id": process_id,
                "status": "killed",
                "duration_ms": duration_ms
            })),
            None,
        ),
    }
}

fn clean_shell_token(token: &str) -> &str {
    token.trim_matches(|character| matches!(character, '\'' | '"' | '`' | ',' | ';' | '(' | ')'))
}

fn parse_port(value: &str) -> Option<u16> {
    clean_shell_token(value)
        .trim_start_matches(':')
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
}

fn background_endpoint_hints(command: &str) -> Vec<String> {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    let mut host = None;
    let mut port = None;

    for (index, raw_token) in tokens.iter().enumerate() {
        let token = clean_shell_token(raw_token);
        if matches!(token, "--host" | "--bind") {
            host = tokens
                .get(index + 1)
                .map(|value| clean_shell_token(value).to_string());
            continue;
        }
        if let Some(value) = token
            .strip_prefix("--host=")
            .or_else(|| token.strip_prefix("--bind="))
        {
            host = Some(clean_shell_token(value).to_string());
            continue;
        }
        if matches!(token, "--port" | "-p") {
            port = tokens.get(index + 1).and_then(|value| parse_port(value));
            continue;
        }
        if let Some(value) = token
            .strip_prefix("--port=")
            .or_else(|| token.strip_prefix("-p="))
        {
            port = parse_port(value);
            continue;
        }
        if token == "http.server" {
            port = tokens.get(index + 1).and_then(|value| parse_port(value));
            continue;
        }
        if let Some((candidate_host, candidate_port)) = token.rsplit_once(':') {
            if matches!(candidate_host, "127.0.0.1" | "localhost" | "::1") {
                if let Some(candidate_port) = parse_port(candidate_port) {
                    host = Some(candidate_host.to_string());
                    port = Some(candidate_port);
                }
            }
        }
    }

    match port {
        Some(port) => vec![format!(
            "{}:{}",
            host.filter(|value| !value.is_empty())
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            port
        )],
        None => Vec::new(),
    }
}

fn normalized_working_dir(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn launch_signature(command: &str, working_dir: &std::path::Path) -> (PathBuf, String) {
    let mut effective_dir = normalized_working_dir(working_dir);
    let (mut effective_command, _, _) = normalize_tracked_background_command(command);

    if let Some((directory_segment, remainder)) = effective_command
        .split_once("&&")
        .map(|(directory, remainder)| (directory.to_string(), remainder.to_string()))
    {
        let directory_segment = directory_segment.trim();
        if let Some(directory) = directory_segment.strip_prefix("cd ") {
            let directory = clean_shell_token(directory.trim());
            let directory = PathBuf::from(directory);
            let resolved_directory = if directory.is_absolute() {
                directory
            } else {
                working_dir.join(directory)
            };
            effective_dir = normalized_working_dir(&resolved_directory);
            effective_command = normalize_tracked_background_command(remainder.trim()).0;
        }
    }

    (
        effective_dir,
        effective_command
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn same_background_launch(
    process: &ProcessInfo,
    command: &str,
    working_dir: &std::path::Path,
) -> bool {
    if !process.is_active() {
        return false;
    }

    let requested_endpoints = background_endpoint_hints(command);
    if !requested_endpoints.is_empty() {
        let existing_endpoints = background_endpoint_hints(&process.command);
        if !requested_endpoints
            .iter()
            .any(|endpoint| existing_endpoints.contains(endpoint))
        {
            return false;
        }
    }

    launch_signature(command, working_dir)
        == launch_signature(&process.command, &process._working_dir)
}

fn existing_background_result(
    process: ProcessInfo,
    endpoint_hints: Vec<String>,
    mut warnings: Vec<String>,
) -> ToolResult {
    warnings.push(
        "An equivalent owner-scoped background process is already active; reused its existing process_id instead of launching a duplicate."
            .to_string(),
    );
    ToolResult::success_data_with(
        json!({
            "message": "Equivalent background process already running",
            "process_id": process.id,
            "status": process.display_status(),
            "endpoint_hints": endpoint_hints,
            "reused_existing": true,
            "next_action": "Use processes status/control with this process_id and probe the endpoint before continuing."
        }),
        warnings,
        None,
        None,
    )
    .with_changed(false)
}

fn output_spool_path(ctx: &ToolContext) -> PathBuf {
    let session = ctx
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("local")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    // Durable command output is runtime state, not project source. Canonical
    // agent runs always provide a database path, so keep their recoverable
    // spools beside that database. Standalone/direct tool contexts without a
    // state database retain the legacy workspace-local fallback.
    ctx.db_path
        .as_ref()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.working_dir.join(".krusty"))
        .join("tool-output")
        .join(session)
        .join(format!("tool_{}.log", uuid::Uuid::new_v4()))
}

#[derive(Deserialize)]
struct Params {
    command: String,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    run_in_background: Option<bool>,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute shell commands for git, builds, package managers, compilers, servers, and system utilities. \
         Do not use find/ls/cat/head/tail/grep for filesystem discovery or reading: use List, Glob, Read, and Grep directly because Bash file operations are rejected. \
         Set run_in_background:true for servers/watchers."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use bash for git, builds, package managers, compilers, servers/watchers, and system utilities. Use dedicated tools for routine file read/search/edit operations.

Chain dependent commands with `&&`; run independent commands as separate parallel tool calls. Use `run_in_background:true` for servers/watchers instead of a trailing `&`. Preview servers must bind explicitly to `127.0.0.1` or `localhost`; do not expose a wildcard listener. Run the server as its own command rather than combining it with file discovery.

Background results return a durable `process_id` for lifecycle control after the turn. Use `processes` status/control for that process and probe its endpoint for service readiness; do not launch a duplicate while the tracked process is still running. If startup failed, act on the captured stderr or select an unused high port instead of repeating the command unchanged.

For user-requested private tailnet exposure, keep the application bound to loopback and run a tailnet-only `tailscale serve` proxy as a separate command. Do not use public Tailscale Funnel unless the user explicitly requests public exposure.

Default timeout is 30 seconds; set `timeout` explicitly for long-running commands (max 600000ms). Include a concise `description` for logging/progress.

Relative paths resolve from the tool working directory; chain `cd dir && command` when a command must run from a specific directory. Avoid interactive commands; avoid `sudo` unless explicitly requested.

If a validation/preflight command fails with actionable file diagnostics (for example `git diff --check` reporting trailing whitespace), fix the reported files with the dedicated edit tools, re-stage affected paths when needed, then rerun the check. If `git diff --cached --check` fails, remember that the staged index is stale until you run `git add <reported-path>` after the edit. Do not repeat the same failing command unchanged."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds (max 600000)"
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does in 5-10 words"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Set to true to run this command in the background"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        match &params.description {
            Some(desc) => {
                tracing::info!(command = %params.command, description = %desc, "Executing bash command")
            }
            None => tracing::info!(command = %params.command, "Executing bash command"),
        }

        if let Some(ref sandbox) = ctx.sandbox_root {
            let canonical = match ctx.working_dir.canonicalize() {
                Ok(c) => c,
                Err(_) => {
                    return ToolResult::error(
                        "Access denied: cannot verify working directory".to_string(),
                    );
                }
            };
            if !canonical.starts_with(sandbox) {
                return ToolResult::error(
                    "Access denied: working directory is outside the configured filesystem access root".to_string(),
                );
            }
        }

        let effective_command = if let Some(ref identity) = ctx.git_identity {
            identity.apply_to_command(&params.command)
        } else {
            params.command.clone()
        };

        let (clean_command, inferred_from_shell_suffix, removed_detachment_wrapper) =
            normalize_tracked_background_command(&effective_command);

        if params.run_in_background.unwrap_or(false) || inferred_from_shell_suffix {
            let mut warnings = Vec::new();
            if inferred_from_shell_suffix {
                warnings.push(
                    "Background mode inferred from trailing '&'; prefer run_in_background:true for clarity."
                        .to_string(),
                );
            }
            if removed_detachment_wrapper {
                warnings.push(
                    "Removed redundant nohup and /dev/null detachment syntax; the shared process registry owns lifecycle and output capture."
                        .to_string(),
                );
            }

            if let Some(ref registry) = ctx.process_registry {
                let endpoint_hints = background_endpoint_hints(&clean_command);
                let spawn_result = match ctx.user_id.as_deref() {
                    Some(uid) => {
                        registry
                            .spawn_or_reuse_matching_for_user(
                                uid,
                                clean_command.clone(),
                                ctx.working_dir.clone(),
                                params.description.clone(),
                                |process| {
                                    same_background_launch(
                                        process,
                                        &clean_command,
                                        &ctx.working_dir,
                                    )
                                },
                            )
                            .await
                    }
                    None => {
                        registry
                            .spawn_or_reuse_matching(
                                clean_command.clone(),
                                ctx.working_dir.clone(),
                                params.description.clone(),
                                |process| {
                                    same_background_launch(
                                        process,
                                        &clean_command,
                                        &ctx.working_dir,
                                    )
                                },
                            )
                            .await
                    }
                };
                match spawn_result {
                    Ok((process, true)) => {
                        return existing_background_result(process, endpoint_hints, warnings);
                    }
                    Ok((process, false)) => {
                        return background_start_result(
                            registry,
                            ctx.user_id.as_deref(),
                            process.id,
                            endpoint_hints,
                            warnings,
                        )
                        .await;
                    }
                    Err(e) => {
                        return ToolResult::error(format!("Failed to start: {}", e));
                    }
                }
            } else {
                return ToolResult::error_with_details(
                    "background_registry_unavailable",
                    "Background execution requires the shared process registry; refusing to start an untracked detached process",
                    Some(json!({
                        "status": "not_started",
                        "endpoint_hints": background_endpoint_hints(&clean_command)
                    })),
                    None,
                );
            }
        }

        // Shell commands are intentionally broad, so the agent cannot infer a
        // durable state change from a zero exit code alone. Capture only
        // explicit, workspace-scoped mutation targets before execution. The
        // probe reports only a positive delta. Equality, timeout, or any
        // unparsed/ambiguous surface remains opaque rather than risking a
        // false no-change claim.
        let state_delta_probe = state_delta::BashStateDeltaProbe::capture(
            &params.command,
            &ctx.working_dir,
            ctx.sandbox_root.as_deref(),
        )
        .await;

        let mut cmd = build_shell_command(&effective_command, ctx);
        configure_foreground_process_group(&mut cmd);
        cmd.kill_on_drop(true);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let timeout_ms = params.timeout.unwrap_or(30_000).min(600_000);
        let timeout_duration = Duration::from_millis(timeout_ms);

        let stream = match (ctx.output_tx.as_ref(), ctx.tool_use_id.as_ref()) {
            (Some(tx), Some(id)) => Some(StreamContext {
                output_tx: tx.clone(),
                tool_use_id: id.clone(),
            }),
            (None, None) => None,
            _ => return ToolResult::error("Streaming context incomplete for bash tool"),
        };

        let result =
            execute_foreground(cmd, timeout_duration, stream, output_spool_path(ctx)).await;
        let result = normalize_tailscale_serve_result(&effective_command, result);
        if result.is_error {
            return result;
        }

        match state_delta_probe {
            Some(probe) => match probe.changed().await {
                Some(change_key) => result
                    .with_changed(true)
                    .with_progress_change_key(change_key),
                None => result,
            },
            None => result,
        }
    }
}

/// Apply ANSI stripping and truncation to the final output sent to the AI model.
fn process_output(combined: String, full_output_path: Option<&std::path::Path>) -> String {
    let stripped = strip_ansi(&combined);
    let result = truncation::truncate_tail(&stripped, MAX_OUTPUT_LINES, MAX_OUTPUT_BYTES);
    if let Some(notice) = result.notice() {
        let recovery = full_output_path
            .map(|path| format!(" Full output saved to {}.", path.display()))
            .unwrap_or_default();
        format!("{}{}{}", result.text, notice, recovery)
    } else {
        result.text
    }
}
