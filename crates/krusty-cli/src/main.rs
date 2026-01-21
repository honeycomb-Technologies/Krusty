//! Krusty - The most elegant coding CLI to ever exist
//!
//! A terminal-based AI coding assistant with:
//! - Zed's LSP extension ecosystem for 100+ language servers
//! - Multi-provider AI with API key authentication
//! - Single-mode Chat UI with slash commands
//! - Clean architecture from day one

use anyhow::Result;
use clap::{Parser, Subcommand};

// Re-export core modules for TUI usage
use krusty_core::{
    acp, agent, ai, constants, extensions, lsp, paths, plan, process, storage, tools,
};

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
}

/// Restore terminal state - called on panic or unexpected exit
fn restore_terminal() {
    use crossterm::{
        event::DisableMouseCapture,
        execute,
        terminal::{disable_raw_mode, LeaveAlternateScreen},
    };
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up panic hook to restore terminal state
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        original_hook(panic_info);
    }));

    // Initialize logging to file (not stdout/stderr which would mess up TUI)
    let log_dir = paths::logs_dir();
    std::fs::create_dir_all(&log_dir).ok();

    // Create null device path based on platform
    #[cfg(unix)]
    let null_device = "/dev/null";
    #[cfg(windows)]
    let null_device = "NUL";

    let log_file = std::fs::File::create(log_dir.join("krusty.log"))
        .unwrap_or_else(|_| std::fs::File::create(null_device).unwrap());

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    // Apply any pending update before starting TUI
    // This ensures updates are applied silently on restart
    if let Ok(Some(version)) = krusty_core::updater::apply_pending_update() {
        // Set env var so TUI can show success toast
        std::env::set_var("KRUSTY_JUST_UPDATED", &version);
        tracing::info!("Applied pending update to v{}", version);
    }

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Acp) => {
            // ACP mode: run as Agent Client Protocol server
            // All communication happens via stdin/stdout JSON-RPC
            tracing::info!("Starting Krusty in ACP server mode");
            let server = acp::AcpServer::new()?;
            server.run().await?;
        }
        None => {
            // Default: Start TUI chat
            let mut app = tui::App::new().await;
            app.run().await?;
        }
    }

    Ok(())
}
