//! `krusty serve` — unified server with embedded PWA and Tailscale integration
//!
//! Starts the Krusty API server with the PWA frontend embedded in the binary.
//! On first run, prompts for provider and API key configuration.
//! Detects and reuses an already-running instance if present.

use anyhow::{Context, Result};
use std::io::{self, Write};

use krusty_core::ai::providers::ProviderId;
use krusty_core::server_instance;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::tailscale::{self, TailscaleServeSetup};

/// Run the serve command.
pub async fn run(port: u16) -> Result<()> {
    // Check for existing running server
    if let Some(instance) = server_instance::detect_running_server().await {
        print_banner(instance.port, false);
        println!(
            "  Server already running (PID {}). Reusing existing instance.\n",
            instance.pid
        );
        // Don't start a new server — just print the URLs and exit
        print_tailscale_status(tailscale::setup_tailscale_serve(instance.port));
        return Ok(());
    }

    // First-run setup: check if credentials are configured
    let store = CredentialStore::load().unwrap_or_default();
    if store.providers_with_auth().is_empty() {
        run_setup_wizard()?;
    }

    // Write PID file
    server_instance::write_pid_file(port)?;

    // Setup shutdown handler to clean PID file
    let shutdown_signal = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for ctrl+c");
    };

    print_banner(port, true);

    // Setup Tailscale serve (non-blocking, best-effort)
    print_tailscale_status(tailscale::setup_tailscale_serve(port));

    // Initialize tracing for server mode (stdout, not file)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let config = krusty_server::ServerConfig {
        port,
        ..Default::default()
    };

    // Start server with graceful shutdown
    let server = krusty_server::start_server(config);

    tokio::select! {
        result = server => {
            server_instance::remove_pid_file();
            result?;
        }
        _ = shutdown_signal => {
            server_instance::remove_pid_file();
            println!("\n  Shutting down...");
        }
    }

    Ok(())
}

fn print_tailscale_status(setup: TailscaleServeSetup) {
    match setup {
        TailscaleServeSetup::Configured { url } => {
            println!("  Tailscale: {}\n", url);
        }
        TailscaleServeSetup::PermissionDenied { url, detail } => {
            println!("  Tailscale URL: {}", url);
            println!("  Tailscale setup skipped: permission denied for `tailscale serve`.");
            println!("  Fix once: sudo tailscale set --operator=$USER");
            println!("  Then rerun: krusty serve");
            println!("  Details: {}\n", detail);
        }
        TailscaleServeSetup::NotInstalled => {
            println!("  Tip: Install Tailscale to access Krusty from any device.");
            println!("       https://tailscale.com/download\n");
        }
        TailscaleServeSetup::Offline => {
            println!("  Tailscale installed, but this device is offline.\n");
        }
        TailscaleServeSetup::Failed { detail } => {
            println!("  Tailscale setup failed: {}\n", detail);
        }
    }
}

fn print_banner(port: u16, starting: bool) {
    println!();
    println!(
        "  \x1b[1;36mKrusty\x1b[0m server {}",
        if starting { "starting" } else { "running" }
    );
    println!("  ─────────────────────────────────────");
    println!("  Local:  http://localhost:{}", port);
}

/// Interactive CLI setup wizard for first-time configuration.
fn run_setup_wizard() -> Result<()> {
    println!();
    println!("  \x1b[1;36mKrusty\x1b[0m — First-time setup");
    println!("  ─────────────────────────────────────");
    println!();

    let providers = ProviderId::all();
    println!("  Select a provider:");
    for (i, provider) in providers.iter().enumerate() {
        let marker = if i == 0 { " (default)" } else { "" };
        println!("    {}. {}{}", i + 1, provider, marker);
    }
    println!();

    print!("  Choice [1]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    let provider = if input.is_empty() {
        providers[0]
    } else {
        let idx: usize = input.parse().context("Invalid number")?;
        if idx == 0 || idx > providers.len() {
            anyhow::bail!("Invalid choice: {}", idx);
        }
        providers[idx - 1]
    };

    println!();
    print!("  API key for {}: ", provider);
    io::stdout().flush()?;

    let mut api_key = String::new();
    io::stdin().read_line(&mut api_key)?;
    let api_key = api_key.trim().to_string();

    if api_key.is_empty() {
        anyhow::bail!("API key cannot be empty");
    }

    let mut store = CredentialStore::load().unwrap_or_default();
    store.set(provider, api_key);
    store.save().context("Failed to save credentials")?;

    println!();
    println!("  \x1b[32m✓\x1b[0m Credentials saved for {}", provider);
    println!();

    Ok(())
}
