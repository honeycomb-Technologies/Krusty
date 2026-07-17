#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
use anyhow::{bail, Context, Result};
#[cfg(unix)]
use krusty_mako::{DaemonServer, MakoDaemonConfig, UnavailableCommandHandler, DAEMON_VERSION};

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("krusty_mako=info")),
        )
        .with_target(false)
        .init();

    let mut config = MakoDaemonConfig::discover().context("discovering Mako daemon paths")?;
    parse_arguments(&mut config)?;

    let server = DaemonServer::bind(config, Arc::new(UnavailableCommandHandler))
        .await
        .context("starting Mako daemon")?;
    let handle = server.handle();
    let signal_task = tokio::spawn(async move {
        let signal = wait_for_shutdown_signal().await.unwrap_or_else(|error| {
            tracing::error!(error = %error, "Mako signal handler failed");
            format!("signal handler failed: {error}")
        });
        handle.shutdown(signal);
    });

    let result = server.serve().await;
    signal_task.abort();
    result
}

#[cfg(unix)]
fn parse_arguments(config: &mut MakoDaemonConfig) -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("daemon") => {}
            Some("--socket") => {
                config.paths.socket_path = PathBuf::from(
                    arguments
                        .next()
                        .context("--socket requires a filesystem path")?,
                );
            }
            Some("--key") => {
                config.paths.key_path = PathBuf::from(
                    arguments
                        .next()
                        .context("--key requires a filesystem path")?,
                );
            }
            Some("--instance-id") => {
                config.instance_id = arguments
                    .next()
                    .context("--instance-id requires a value")?
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("--instance-id must be valid UTF-8"))?;
                if config.instance_id.trim().is_empty() {
                    bail!("--instance-id cannot be empty");
                }
            }
            Some("--help" | "-h") => {
                print_help();
                std::process::exit(0);
            }
            Some("--version" | "-V") => {
                println!("krusty-mako {DAEMON_VERSION}");
                std::process::exit(0);
            }
            Some(value) => bail!("unknown Mako daemon argument: {value}"),
            None => bail!("Mako daemon arguments must be valid UTF-8"),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn print_help() {
    println!(
        "krusty-mako {DAEMON_VERSION}\n\n\
         Usage: krusty-mako [daemon] [OPTIONS]\n\n\
         Options:\n  \
           --socket <PATH>       Private Unix socket path\n  \
           --key <PATH>          32-byte private IPC key path\n  \
           --instance-id <ID>    Stable identifier for this daemon process\n  \
           -h, --help            Print help\n  \
           -V, --version         Print version"
    );
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<String> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("installing SIGTERM handler")?;
    let signal = tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.context("waiting for Ctrl-C")?;
            "received Ctrl-C".to_string()
        }
        _ = terminate.recv() => "received SIGTERM".to_string(),
    };
    Ok(signal)
}

#[cfg(not(unix))]
fn main() {
    eprintln!("krusty-mako requires Unix-domain sockets and peer credentials");
    std::process::exit(1);
}
