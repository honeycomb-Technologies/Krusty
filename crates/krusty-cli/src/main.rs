//! Krusty - The most elegant coding CLI to ever exist
//!
//! A terminal-based AI coding assistant with:
//! - Multi-provider AI with API key authentication
//! - Single-mode Chat UI with slash commands
//! - `krusty serve` — unified server + PWA + Tailscale
//! - Clean architecture from day one

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

// Re-export core modules for TUI usage
use krusty_core::{
    acp, agent, ai, constants, extensions, paths, plan, plugins, process, storage, tools,
};

mod serve;
mod tui;

/// Krusty - AI Coding Assistant
#[derive(Parser)]
#[command(name = "krusty")]
#[command(about = "The most elegant coding CLI to ever exist", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run as ACP (Agent Client Protocol) server
    ///
    /// Krusty runs as an ACP-compatible agent that communicates
    /// via JSON-RPC over stdin/stdout. This mode is used when Krusty is
    /// spawned by an ACP-compatible editor (Zed, Neovim, etc.).
    ///
    /// Uses credentials from TUI configuration, or override with env vars:
    /// - KRUSTY_PROVIDER + KRUSTY_API_KEY (+ optional KRUSTY_MODEL)
    /// - Or provider-specific: ANTHROPIC_API_KEY, OPENROUTER_API_KEY, etc.
    Acp,

    /// Start the Krusty web server with embedded PWA frontend
    ///
    /// Launches the API server with the PWA bundled into the binary.
    /// On first run, prompts for provider and API key configuration.
    /// Automatically configures Tailscale for remote access if available.
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value_t = 3000)]
        port: u16,
    },

    /// Mako autonomous agent
    Mako {
        #[command(subcommand)]
        command: MakoCommand,
    },
}

#[derive(Subcommand)]
enum MakoCommand {
    /// Submit a task to Mako
    Run {
        /// The task to perform
        task: String,
        /// Project directory (defaults to current)
        #[arg(long)]
        project_dir: Option<String>,
    },
    /// Show status of Mako sessions
    Status,
}

fn mako_server_url() -> String {
    std::env::var("KRUSTY_SERVER_URL").unwrap_or_else(|_| "http://localhost:3001".to_string())
}

async fn run_mako_command(command: MakoCommand) -> Result<()> {
    let base = mako_server_url();
    let client = reqwest::Client::new();

    match command {
        MakoCommand::Run { task, project_dir } => {
            let mut body = serde_json::json!({ "task": task });
            if let Some(dir) = &project_dir {
                body["project_dir"] = serde_json::json!(dir);
            }

            let resp = client
                .post(format!("{base}/api/mako/dispatch"))
                .json(&body)
                .send()
                .await
                .context("Failed to reach Krusty server")?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("Server returned {status}: {text}");
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .context("Failed to parse dispatch response")?;

            let session_id = data["session_id"].as_str().unwrap_or("unknown");

            println!("Mako task dispatched");
            println!("  Session: {session_id}");
            println!("  Observe: {base}/session/{session_id}");
        }
        MakoCommand::Status => {
            let resp = client
                .get(format!("{base}/api/mako/sessions"))
                .send()
                .await
                .context("Failed to reach Krusty server")?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("Server returned {status}: {text}");
            }

            let sessions: Vec<serde_json::Value> = resp
                .json()
                .await
                .context("Failed to parse sessions response")?;

            if sessions.is_empty() {
                println!("No active Mako sessions.");
            } else {
                println!("{:<38} {:<12} {:<30}", "SESSION ID", "STATUS", "TASK");
                println!("{}", "-".repeat(80));
                for s in &sessions {
                    let id = s["id"].as_str().unwrap_or("-");
                    let status = s["status"].as_str().unwrap_or("-");
                    let task = s["task"]
                        .as_str()
                        .unwrap_or(s["title"].as_str().unwrap_or("-"));
                    let task_display = if task.len() > 28 {
                        format!("{}...", &task[..25])
                    } else {
                        task.to_string()
                    };
                    println!("{:<38} {:<12} {:<30}", id, status, task_display);
                }
            }
        }
    }

    Ok(())
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

    // Mako subcommand runs HTTP requests and exits, no TUI needed
    if let Some(Commands::Mako { command }) = cli.command {
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
    if let Ok(Some(version)) = krusty_core::updater::apply_pending_update() {
        tracing::info!("Applied pending update to v{}", version);
    }

    match cli.command {
        Some(Commands::Acp) => {
            tracing::info!("Starting Krusty in ACP server mode");
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
