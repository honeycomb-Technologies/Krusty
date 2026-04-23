//! `krusty serve` — unified server with embedded web app and Tailscale integration
//!
//! Starts the Krusty API server with the web frontend embedded in the binary.
//! On first run, prompts for provider and API key configuration.
//! Detects and reuses an already-running instance if present.

use anyhow::{Context, Result};
use std::io::{self, Write};
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};

use krusty_core::ai::providers::ProviderId;
use krusty_core::server_instance;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::tailscale::{self, TailscaleServeSetup};

const DEFAULT_SERVER_PORT: u16 = 3000;
const DEFAULT_PORT_SEARCH_SPAN: u16 = 100;

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

    let (port, listener) = reserve_server_listener(port)?;

    if port != DEFAULT_SERVER_PORT {
        println!(
            "  Port {} is already in use. Using {} instead.",
            DEFAULT_SERVER_PORT, port
        );
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
    let server = krusty_server::start_server_with_listener(config, listener);

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

fn reserve_server_listener(port: u16) -> Result<(u16, tokio::net::TcpListener)> {
    if port == DEFAULT_SERVER_PORT {
        return reserve_listener_with_fallback(port, DEFAULT_PORT_SEARCH_SPAN);
    }

    let listener =
        bind_listener(port).with_context(|| format!("Failed to bind server port {}.", port))?;
    Ok((port, listener))
}

fn reserve_listener_with_fallback(
    start_port: u16,
    search_span: u16,
) -> Result<(u16, tokio::net::TcpListener)> {
    let mut reserved_listener = None;
    let selected_port =
        find_available_port(start_port, search_span, |candidate| {
            match bind_listener(candidate) {
                Ok(listener) => {
                    reserved_listener = Some(listener);
                    Ok(true)
                }
                Err(err) if err.kind() == io::ErrorKind::AddrInUse => Ok(false),
                Err(err) => Err(err)
                    .with_context(|| format!("Failed while probing server port {}.", candidate)),
            }
        })?;

    let listener = reserved_listener.expect("selected port must have a reserved listener");
    Ok((selected_port, listener))
}

fn bind_listener(port: u16) -> io::Result<tokio::net::TcpListener> {
    let listener = StdTcpListener::bind((Ipv4Addr::UNSPECIFIED, port))?;
    listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(listener)
}

fn find_available_port<F>(start_port: u16, search_span: u16, mut probe: F) -> Result<u16>
where
    F: FnMut(u16) -> Result<bool>,
{
    let end_port = start_port.saturating_add(search_span);

    for offset in 0..=search_span {
        let Some(candidate) = start_port.checked_add(offset) else {
            break;
        };

        if probe(candidate)? {
            return Ok(candidate);
        }
    }

    anyhow::bail!(
        "No available server port found in the {}-{} range.",
        start_port,
        end_port
    );
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

#[cfg(test)]
mod tests {
    use super::find_available_port;

    #[test]
    fn falls_forward_when_preferred_port_is_busy() {
        let selected =
            find_available_port(3000, 5, |port| Ok(port == 3002)).expect("find fallback port");

        assert_eq!(selected, 3002);
    }

    #[test]
    fn explicit_non_default_port_does_not_fallback() {
        let err = find_available_port(4242, 0, |_| Ok(false))
            .expect_err("non-default explicit port should fail when occupied");

        assert!(err.to_string().contains("4242-4242"));
    }
}
