//! Mitsuro - local-first AI coding assistant
//!
//! A terminal-based AI coding assistant with:
//! - Multi-provider AI with API key authentication
//! - Single-mode Chat UI with slash commands
//! - `krusty serve` — unified server + embedded web app + Tailscale
//! - Clean architecture from day one

use std::io::{self, Write};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures::StreamExt;
use serde::{de::DeserializeOwned, Deserialize};

// Re-export core modules for TUI usage
use krusty_core::{acp, agent, ai, extensions, paths, plan, plugins, process, storage, tools};

mod serve;
mod tui;

/// Mitsuro - AI Coding Assistant
#[derive(Parser)]
#[command(name = "krusty")]
#[command(about = "Mitsuro, a local-first AI coding assistant", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run as ACP (Agent Client Protocol) server
    ///
    /// Mitsuro runs as an ACP-compatible Agent that communicates
    /// via JSON-RPC over stdin/stdout. This mode is used when Mitsuro is
    /// spawned by an ACP-compatible editor (Zed, Neovim, etc.).
    ///
    /// Uses credentials from TUI configuration, or override with env vars:
    /// - KRUSTY_PROVIDER + KRUSTY_API_KEY (+ optional KRUSTY_MODEL)
    /// - Or provider-specific: ANTHROPIC_API_KEY, OPENROUTER_API_KEY, etc.
    Acp,

    /// Start the Mitsuro web server with embedded web frontend
    ///
    /// Launches the API server with the web bundle embedded into the binary.
    /// On first run, prompts for provider and API key configuration.
    /// Automatically configures Tailscale for remote access if available.
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value_t = 3000)]
        port: u16,
    },

    /// Hive autonomous agent system
    #[command(
        name = "hive",
        visible_alias = "mako",
        args_conflicts_with_subcommands = true
    )]
    Mako {
        #[command(subcommand)]
        command: Option<MakoCommand>,
        /// Submit a task directly without specifying `run`
        #[arg(value_name = "TASK")]
        task: Option<String>,
        /// Project directory (shorthand task submission only)
        #[arg(long, requires = "task")]
        project_dir: Option<String>,
        /// Attach to the live event stream after shorthand task submission
        #[arg(long, requires = "task")]
        attach: bool,
    },
}

#[derive(Subcommand)]
enum MakoCommand {
    /// Submit a task to Hive
    Run {
        /// The task to perform
        task: String,
        /// Project directory (defaults to current)
        #[arg(long)]
        project_dir: Option<String>,
        /// Attach to the live event stream after dispatch
        #[arg(long)]
        attach: bool,
    },
    /// Show status for Hive sessions
    Status {
        /// Optional session id for detailed status
        session_id: Option<String>,
    },
    /// Attach to a running Hive session's live event stream
    Attach {
        /// Hive session id
        session_id: String,
    },
    /// Pause a running Hive session
    Pause {
        /// Hive session id
        session_id: String,
    },
    /// Resume a paused or idle Hive session
    Resume {
        /// Hive session id
        session_id: String,
    },
    /// Cancel and delete a Hive session
    Cancel {
        /// Hive session id
        session_id: String,
    },
    /// Send a follow-up message to an existing Hive session
    Send {
        /// Hive session id
        session_id: String,
        /// Follow-up message
        message: String,
    },
}

#[derive(Debug, Deserialize)]
struct MakoDispatchResponse {
    session_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct MakoOkResponse {
    ok: bool,
}

#[derive(Debug, Deserialize)]
struct MakoRuntimeStateResponse {
    session_id: String,
    status: String,
    next_wake_at: Option<String>,
    sleep_reason: Option<String>,
    last_error: Option<String>,
    current_run_id: Option<String>,
    last_wake_reason: Option<String>,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct MakoSessionSummaryResponse {
    session_id: String,
    title: String,
    agent_state: String,
    runtime: Option<MakoRuntimeStateResponse>,
}

#[derive(Debug, Deserialize)]
struct MakoTaskResponse {
    id: String,
    subject: String,
    description: String,
    status: String,
    owner: Option<String>,
    blocked_by: Vec<String>,
    result: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MakoSessionStatusResponse {
    session_id: String,
    session_type: String,
    title: String,
    tasks: Vec<MakoTaskResponse>,
    agent_state: String,
    runtime: Option<MakoRuntimeStateResponse>,
}

fn mako_server_url() -> String {
    std::env::var("KRUSTY_SERVER_URL").unwrap_or_else(|_| "http://localhost:3001".to_string())
}

async fn run_mako_command(command: MakoCommand) -> Result<()> {
    let base = mako_server_url();
    let client = reqwest::Client::new();

    match command {
        MakoCommand::Run {
            task,
            project_dir,
            attach,
        } => {
            let mut body = serde_json::json!({ "task": task });
            if let Some(dir) = &project_dir {
                body["project_dir"] = serde_json::json!(dir);
            }

            let response: MakoDispatchResponse = request_json(
                client.post(format!("{base}/api/mako/dispatch")).json(&body),
                "Failed to dispatch Hive task",
            )
            .await?;

            println!("Hive task dispatched");
            println!("  Session: {}", response.session_id);
            println!("  Status: {}", response.status);
            println!("  Observe: krusty hive attach {}", response.session_id);
            println!("  Status: krusty hive status {}", response.session_id);

            if attach {
                attach_mako_session(&client, &base, &response.session_id).await?;
            }
        }
        MakoCommand::Status { session_id } => {
            if let Some(session_id) = session_id {
                let status: MakoSessionStatusResponse = request_json(
                    client.get(format!("{base}/api/mako/sessions/{session_id}/status")),
                    "Failed to fetch Hive session status",
                )
                .await?;
                print_mako_session_status(&status);
            } else {
                let sessions: Vec<MakoSessionSummaryResponse> = request_json(
                    client.get(format!("{base}/api/mako/sessions")),
                    "Failed to fetch Hive sessions",
                )
                .await?;
                print_mako_session_summaries(&sessions);
            }
        }
        MakoCommand::Attach { session_id } => {
            attach_mako_session(&client, &base, &session_id).await?;
        }
        MakoCommand::Pause { session_id } => {
            let response: MakoOkResponse = request_json(
                client.post(format!("{base}/api/mako/sessions/{session_id}/pause")),
                "Failed to pause Hive session",
            )
            .await?;
            if response.ok {
                println!("Paused Hive session {session_id}");
            }
        }
        MakoCommand::Resume { session_id } => {
            let response: MakoOkResponse = request_json(
                client.post(format!("{base}/api/mako/sessions/{session_id}/resume")),
                "Failed to resume Hive session",
            )
            .await?;
            if response.ok {
                println!("Resumed Hive session {session_id}");
            }
        }
        MakoCommand::Cancel { session_id } => {
            request_empty(
                client.delete(format!("{base}/api/mako/sessions/{session_id}")),
                "Failed to cancel Hive session",
            )
            .await?;
            println!("Cancelled Hive session {session_id}");
        }
        MakoCommand::Send {
            session_id,
            message,
        } => {
            let response: MakoOkResponse = request_json(
                client
                    .post(format!("{base}/api/mako/sessions/{session_id}/message"))
                    .json(&serde_json::json!({ "message": message })),
                "Failed to send message to Hive session",
            )
            .await?;
            if response.ok {
                println!("Queued message for Hive session {session_id}");
            }
        }
    }

    Ok(())
}

async fn request_json<T>(request: reqwest::RequestBuilder, context: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let response = request.send().await.with_context(|| context.to_string())?;
    let response = ensure_success(response).await?;
    response
        .json::<T>()
        .await
        .with_context(|| format!("{context}: failed to parse response"))
}

async fn request_empty(request: reqwest::RequestBuilder, context: &str) -> Result<()> {
    let response = request.send().await.with_context(|| context.to_string())?;
    let _ = ensure_success(response).await?;
    Ok(())
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    anyhow::bail!("Server returned {status}: {text}");
}

fn print_mako_session_summaries(sessions: &[MakoSessionSummaryResponse]) {
    if sessions.is_empty() {
        println!("No Hive sessions found.");
        return;
    }

    println!(
        "{:<38} {:<16} {:<14} {:<36}",
        "SESSION ID", "RUNTIME", "AGENT", "TITLE"
    );
    println!("{}", "-".repeat(110));

    for session in sessions {
        let runtime_status = session
            .runtime
            .as_ref()
            .map(|runtime| runtime.status.as_str())
            .unwrap_or(session.agent_state.as_str());
        println!(
            "{:<38} {:<16} {:<14} {:<36}",
            session.session_id,
            runtime_status,
            session.agent_state,
            truncate(&session.title, 36)
        );
    }
}

fn print_mako_session_status(status: &MakoSessionStatusResponse) {
    println!("Session: {}", status.session_id);
    println!("Type: {}", status.session_type);
    println!("Title: {}", status.title);
    println!("Agent State: {}", status.agent_state);

    if let Some(runtime) = &status.runtime {
        println!("Runtime: {}", runtime.status);
        println!("Runtime Session: {}", runtime.session_id);
        println!("Updated: {}", runtime.updated_at);
        if let Some(wake_reason) = &runtime.last_wake_reason {
            println!("Wake Reason: {}", wake_reason);
        }
        if let Some(run_id) = &runtime.current_run_id {
            println!("Run ID: {}", run_id);
        }
        if let Some(next_wake_at) = &runtime.next_wake_at {
            println!("Next Wake: {}", next_wake_at);
        }
        if let Some(reason) = &runtime.sleep_reason {
            println!("Sleep Reason: {}", reason);
        }
        if let Some(error) = &runtime.last_error {
            println!("Last Error: {}", error);
        }
    }

    if status.tasks.is_empty() {
        println!("Tasks: none");
        return;
    }

    println!("Tasks:");
    for task in &status.tasks {
        println!("- [{}] {} ({})", task.status, task.subject, task.id);
        if !task.description.is_empty() {
            println!("  {}", task.description);
        }
        if let Some(owner) = &task.owner {
            println!("  owner: {}", owner);
        }
        if !task.blocked_by.is_empty() {
            println!("  blocked_by: {}", task.blocked_by.join(", "));
        }
        if let Some(result) = &task.result {
            println!("  result: {}", truncate(result, 120));
        }
    }
}

async fn attach_mako_session(client: &reqwest::Client, base: &str, session_id: &str) -> Result<()> {
    let response = client
        .get(format!("{base}/api/mako/sessions/{session_id}/events"))
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .context("Failed to attach to Hive session")?;
    let response = ensure_success(response).await?;

    println!("Attaching to Hive session {}", session_id);
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Failed to read Hive event stream")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_idx) = buffer.find('\n') {
            let line = buffer[..newline_idx].trim_end_matches('\r').to_string();
            buffer.drain(..=newline_idx);

            if let Some(data) = line.strip_prefix("data: ") {
                if data.trim().is_empty() {
                    continue;
                }

                let event: serde_json::Value = serde_json::from_str(data)
                    .with_context(|| format!("Failed to parse Hive event: {data}"))?;
                if print_mako_event(&event)? {
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}

fn print_mako_event(event: &serde_json::Value) -> Result<bool> {
    let Some(event_type) = event.get("type").and_then(|value| value.as_str()) else {
        println!("[event] {}", event);
        return Ok(false);
    };

    match event_type {
        "text_delta" | "text_delta_with_citations" => {
            if let Some(delta) = event.get("delta").and_then(|value| value.as_str()) {
                print!("{}", delta);
                io::stdout().flush()?;
            }
        }
        "tool_output_delta" => {
            if let Some(delta) = event.get("delta").and_then(|value| value.as_str()) {
                print!("{}", delta);
                io::stdout().flush()?;
            }
        }
        "tool_call_start" => {
            let name = event
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            println!("\n[tool:start] {}", name);
        }
        "tool_call_complete" => {
            let name = event
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            println!("\n[tool:call] {}", name);
        }
        "tool_result" => {
            if let Some(output) = event.get("output").and_then(|value| value.as_str()) {
                let output = output.trim();
                if !output.is_empty() {
                    println!("\n[tool:result] {}", output);
                }
            }
        }
        "user_message" => {
            let level = event
                .get("level")
                .and_then(|value| value.as_str())
                .unwrap_or("info");
            let title = event
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let message = event
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if title.is_empty() {
                println!("\n[user:{}] {}", level, message);
            } else {
                println!("\n[user:{}] {}: {}", level, title, message);
            }
        }
        "agent_sleeping" => {
            let duration = event
                .get("duration_secs")
                .and_then(|value| value.as_u64())
                .unwrap_or_default();
            let reason = event
                .get("reason")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            println!("\n[sleep] {}s {}", duration, reason);
        }
        "tick_injected" => {
            let tick = event
                .get("tick_number")
                .and_then(|value| value.as_u64())
                .unwrap_or_default();
            println!("\n[tick] #{}", tick);
        }
        "classifier_decision" => {
            let tool_name = event
                .get("tool_name")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let decision = event
                .get("decision")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let reason = event
                .get("reason")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            println!("\n[classifier] {} => {} ({})", tool_name, decision, reason);
        }
        "delegated_progress" => {
            let agent_name = event
                .get("agent_name")
                .and_then(|value| value.as_str())
                .unwrap_or("agent");
            let stage = event
                .get("stage")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let status = event
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let action = event
                .get("current_action")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if action.is_empty() {
                println!("\n[delegated] {} {} {}", agent_name, stage, status);
            } else {
                println!(
                    "\n[delegated] {} {} {}: {}",
                    agent_name, stage, status, action
                );
            }
        }
        "plan_update" => {
            let count = event
                .get("items")
                .and_then(|value| value.as_array())
                .map(|items| items.len())
                .unwrap_or_default();
            println!("\n[plan] {} items", count);
        }
        "finish" => {
            let stop_reason = event
                .get("stop_reason")
                .and_then(|value| value.as_str())
                .unwrap_or("completed");
            println!("\n[finish] {}", stop_reason);
            return Ok(true);
        }
        "error" => {
            let error = event
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown error");
            println!("\n[error] {}", error);
            return Ok(true);
        }
        _ => {
            println!("\n[{}] {}", event_type, event);
        }
    }

    Ok(false)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }

    let truncated: String = value.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{}...", truncated)
}

/// Restore terminal state - called on panic or unexpected exit
fn restore_terminal() {
    use std::io::Write;

    use crossterm::{
        event::DisableMouseCapture,
        execute,
        terminal::{disable_raw_mode, LeaveAlternateScreen},
    };
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = std::io::stdout().flush();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Serve mode has its own logging (stdout), skip TUI logging setup
    if matches!(cli.command, Some(Commands::Serve { .. })) {
        if let Some(Commands::Serve { port }) = cli.command {
            return serve::run(port).await;
        }
    }

    // Hive subcommand uses Mako-compatible HTTP routes and exits, no TUI needed.
    if let Some(Commands::Mako {
        command,
        task,
        project_dir,
        attach,
    }) = cli.command
    {
        let command = if let Some(command) = command {
            command
        } else if let Some(task) = task {
            MakoCommand::Run {
                task,
                project_dir,
                attach,
            }
        } else {
            anyhow::bail!("Provide a task or a Hive subcommand");
        };
        return run_mako_command(command).await;
    }

    // Set up panic hook to restore terminal state (TUI/ACP modes)
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        original_hook(panic_info);
    }));

    // Initialize logging to file (not stdout/stderr which would mess up TUI)
    let log_dir = paths::logs_dir();
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("Failed to create log directory: {}", e);
    }

    #[cfg(unix)]
    let null_device = "/dev/null";
    #[cfg(windows)]
    let null_device = "NUL";

    let log_file = match std::fs::File::create(log_dir.join("krusty.log")) {
        Ok(file) => file,
        Err(e) => {
            eprintln!(
                "Failed to create log file: {}, falling back to null device",
                e
            );
            match std::fs::File::create(null_device) {
                Ok(file) => file,
                Err(e) => {
                    eprintln!(
                        "Failed to create null device {}: {}, logging disabled",
                        null_device, e
                    );
                    return Err(e.into());
                }
            }
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    // Apply any pending update before starting TUI
    match krusty_core::updater::apply_pending_update() {
        Ok(Some(version)) => tracing::info!("Applied pending update to v{}", version),
        Ok(None) => {}
        Err(error) => tracing::warn!("Pending update was not applied: {}", error),
    }

    match cli.command {
        Some(Commands::Acp) => {
            tracing::info!("Starting Mitsuro in ACP server mode");
            let server = acp::AcpServer::new()?;
            server.run().await?;
        }
        Some(Commands::Serve { .. } | Commands::Mako { .. }) => unreachable!(),
        None => {
            let mut app = tui::App::new().await;
            app.run().await?;
        }
    }

    Ok(())
}
