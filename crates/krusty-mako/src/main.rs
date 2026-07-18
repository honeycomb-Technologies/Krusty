#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
use anyhow::{bail, Context, Result};
#[cfg(unix)]
use krusty_mako::{
    start_runtime, DaemonServer, KrustyExecutionBackend, MakoDaemonConfig, MakoRuntimeConfig,
    MakoRuntimeHandle, DAEMON_VERSION,
};
#[cfg(unix)]
use krusty_mako_protocol::{
    Actor, Command, DispatchCommand, MakoEvent, MakoIpcClient, MakoIpcClientConfig,
    RequestEnvelope, ResponsePayload, ShutdownCommand, SubscribeCommand,
};
#[cfg(unix)]
use krusty_server::mako_execution_host::{MakoExecutionHost, MakoExecutionHostConfig};

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
    let mut database_path = krusty_core::paths::config_dir().join("krusty.db");
    let mut working_dir = std::env::current_dir().context("resolving Mako working directory")?;
    let command = parse_arguments(&mut config, &mut database_path, &mut working_dir)?;

    match command {
        CliCommand::Daemon => run_daemon(config, database_path, working_dir).await,
        command => run_diagnostic(&config, &working_dir, command).await,
    }
}

#[cfg(unix)]
async fn run_daemon(
    config: MakoDaemonConfig,
    database_path: PathBuf,
    working_dir: PathBuf,
) -> Result<()> {
    let execution_host = MakoExecutionHost::build(MakoExecutionHostConfig::new(
        database_path.clone(),
        working_dir,
    ))
    .await
    .context("starting the Mako agent execution host")?;
    let backend = Arc::new(KrustyExecutionBackend::new(execution_host));
    let runtime = start_runtime(
        MakoRuntimeConfig::for_database(database_path),
        config.instance_id.clone(),
        backend,
    )
    .await
    .context("starting the durable Mako scheduler")?;
    let (server, mut runtime) = bind_server_or_shutdown(config, runtime).await?;
    let handle = server.handle();
    let signal_handle = handle.clone();
    let signal_task = tokio::spawn(async move {
        let signal = wait_for_shutdown_signal().await.unwrap_or_else(|error| {
            tracing::error!(error = %error, "Mako signal handler failed");
            format!("signal handler failed: {error}")
        });
        signal_handle.shutdown(signal);
    });

    let mut serving = Box::pin(server.serve());
    let result = tokio::select! {
        // An authenticated Shutdown or OS signal stops IPC first. Only after
        // that branch wins do we request the expected pump shutdown below, so
        // a graceful stop cannot be misclassified as scheduler failure.
        result = &mut serving => result,
        failure = runtime.wait_for_scheduler_failure() => {
            tracing::error!(error = %failure, "Mako scheduler supervision tripped");
            handle.shutdown("scheduler pump stopped unexpectedly");
            if let Err(error) = serving.await {
                tracing::warn!(error = %error, "Mako IPC shutdown after scheduler failure failed");
            }
            Err(failure)
        }
    };
    signal_task.abort();
    runtime.shutdown().await;
    result
}

#[cfg(unix)]
async fn bind_server_or_shutdown(
    config: MakoDaemonConfig,
    runtime: MakoRuntimeHandle,
) -> Result<(DaemonServer, MakoRuntimeHandle)> {
    match DaemonServer::bind(config, runtime.handler()).await {
        Ok(server) => Ok((server, runtime)),
        Err(error) => {
            // Binding happens after the scheduler starts so it can initialize
            // durable state. Give the pump a graceful stop on bind failure;
            // dropping the handle would abort it before releasing its lease.
            runtime.shutdown().await;
            Err(error).context("starting Mako daemon")
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
enum CliCommand {
    Daemon,
    Ping,
    Stats,
    Shutdown {
        reason: Option<String>,
    },
    Dispatch {
        task: String,
        project_dir: Option<String>,
        model: Option<String>,
        priority: Option<String>,
        crew_slug: Option<String>,
        start_at_unix_ms: Option<i64>,
    },
    Events {
        session_id: String,
        after_sequence: i64,
        replay_limit: usize,
        follow: bool,
    },
}

#[cfg(unix)]
fn parse_arguments(
    config: &mut MakoDaemonConfig,
    database_path: &mut PathBuf,
    working_dir: &mut PathBuf,
) -> Result<CliCommand> {
    let mut mode = None;
    let mut task = None;
    let mut project_dir = None;
    let mut model = None;
    let mut priority = None;
    let mut crew_slug = None;
    let mut start_at_unix_ms = None;
    let mut session_id = None;
    let mut after_sequence = 0_i64;
    let mut replay_limit = 1_000_usize;
    let mut follow = false;
    let mut shutdown_reason = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("daemon" | "ping" | "stats" | "shutdown" | "dispatch" | "events") => {
                if mode
                    .replace(argument.to_string_lossy().into_owned())
                    .is_some()
                {
                    bail!("only one Mako subcommand may be specified");
                }
            }
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
            Some("--database") => {
                *database_path = PathBuf::from(
                    arguments
                        .next()
                        .context("--database requires a filesystem path")?,
                );
            }
            Some("--working-dir") => {
                *working_dir = PathBuf::from(
                    arguments
                        .next()
                        .context("--working-dir requires a filesystem path")?,
                );
            }
            Some("--task") => {
                task = Some(argument_value(&mut arguments, "--task")?);
            }
            Some("--project-dir") => {
                project_dir = Some(argument_value(&mut arguments, "--project-dir")?);
            }
            Some("--model") => {
                model = Some(argument_value(&mut arguments, "--model")?);
            }
            Some("--priority") => {
                priority = Some(argument_value(&mut arguments, "--priority")?);
            }
            Some("--crew") => {
                crew_slug = Some(argument_value(&mut arguments, "--crew")?);
            }
            Some("--start-at-unix-ms") => {
                start_at_unix_ms = Some(parse_argument(
                    &argument_value(&mut arguments, "--start-at-unix-ms")?,
                    "--start-at-unix-ms",
                )?);
            }
            Some("--session") => {
                session_id = Some(argument_value(&mut arguments, "--session")?);
            }
            Some("--after") => {
                after_sequence =
                    parse_argument(&argument_value(&mut arguments, "--after")?, "--after")?;
                if after_sequence < 0 {
                    bail!("--after cannot be negative");
                }
            }
            Some("--replay-limit") => {
                replay_limit = parse_argument(
                    &argument_value(&mut arguments, "--replay-limit")?,
                    "--replay-limit",
                )?;
                if replay_limit == 0 {
                    bail!("--replay-limit must be greater than zero");
                }
            }
            Some("--follow") => follow = true,
            Some("--reason") => {
                shutdown_reason = Some(argument_value(&mut arguments, "--reason")?);
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

    match mode.as_deref().unwrap_or("daemon") {
        "daemon" => Ok(CliCommand::Daemon),
        "ping" => Ok(CliCommand::Ping),
        "stats" => Ok(CliCommand::Stats),
        "shutdown" => Ok(CliCommand::Shutdown {
            reason: shutdown_reason,
        }),
        "dispatch" => Ok(CliCommand::Dispatch {
            task: task
                .filter(|value| !value.trim().is_empty())
                .context("dispatch requires --task")?,
            project_dir,
            model: Some(
                model
                    .filter(|value| !value.trim().is_empty())
                    .context("dispatch requires an explicit --model")?,
            ),
            priority,
            crew_slug,
            start_at_unix_ms,
        }),
        "events" => Ok(CliCommand::Events {
            session_id: session_id
                .filter(|value| !value.trim().is_empty())
                .context("events requires --session")?,
            after_sequence,
            replay_limit,
            follow,
        }),
        _ => unreachable!("validated Mako subcommand"),
    }
}

#[cfg(unix)]
fn argument_value(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    option: &str,
) -> Result<String> {
    arguments
        .next()
        .with_context(|| format!("{option} requires a value"))?
        .into_string()
        .map_err(|_| anyhow::anyhow!("{option} must be valid UTF-8"))
}

#[cfg(unix)]
fn parse_argument<T: std::str::FromStr>(value: &str, option: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid {option} value: {error}"))
}

#[cfg(unix)]
async fn run_diagnostic(
    config: &MakoDaemonConfig,
    working_dir: &std::path::Path,
    command: CliCommand,
) -> Result<()> {
    let mut client_config =
        MakoIpcClientConfig::new(config.paths.socket_path.clone(), "krusty-mako-cli");
    client_config.request_timeout = config.control_io_timeout;
    let client = MakoIpcClient::from_key_path_or_create(client_config, &config.paths.key_path)
        .with_context(|| {
            format!(
                "loading or initializing Mako IPC key at {}",
                config.paths.key_path.display()
            )
        })?;
    let actor = Actor::local("krusty-mako-cli");

    match command {
        CliCommand::Ping => {
            print_payload(client.command(actor, Command::Ping, None).await?)?;
        }
        CliCommand::Stats => {
            print_payload(client.command(actor, Command::Stats, None).await?)?;
        }
        CliCommand::Shutdown { reason } => {
            print_payload(
                client
                    .command(actor, Command::Shutdown(ShutdownCommand { reason }), None)
                    .await?,
            )?;
        }
        CliCommand::Dispatch {
            task,
            project_dir,
            model,
            priority,
            crew_slug,
            start_at_unix_ms,
        } => {
            let payload = client
                .command(
                    actor,
                    Command::Dispatch(DispatchCommand {
                        task,
                        working_dir: working_dir.to_string_lossy().into_owned(),
                        project_dir,
                        model,
                        start_at_unix_ms,
                        priority,
                        crew_slug,
                    }),
                    None,
                )
                .await?;
            print_payload(payload)?;
        }
        CliCommand::Events {
            session_id,
            after_sequence,
            replay_limit,
            follow,
        } => {
            let request = RequestEnvelope::new(
                actor,
                Command::Subscribe(SubscribeCommand {
                    session_id,
                    after_sequence: Some(after_sequence),
                    replay_limit: Some(replay_limit),
                }),
                u64::try_from(config.control_io_timeout.as_millis()).unwrap_or(u64::MAX),
            );
            let mut subscription = client.subscribe(request).await?;
            let high_water = subscription.accepted.high_water_sequence;
            if !follow && high_water.is_none_or(|sequence| sequence <= after_sequence) {
                return Ok(());
            }
            while let Some(event) = subscription.next_event().await? {
                println!("{}", serde_json::to_string(&event)?);
                let reached_snapshot = high_water
                    .zip(event.sequence)
                    .is_some_and(|(high, sequence)| sequence >= high);
                let terminal = is_terminal_event(&event.event);
                if (!follow && reached_snapshot) || (follow && terminal) {
                    break;
                }
            }
        }
        CliCommand::Daemon => unreachable!("daemon command is handled before diagnostics"),
    }
    Ok(())
}

#[cfg(unix)]
fn print_payload(payload: ResponsePayload) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

#[cfg(unix)]
fn is_terminal_event(event: &MakoEvent) -> bool {
    match event {
        MakoEvent::Runtime(event) => matches!(
            event.event_type.as_str(),
            "run_completed"
                | "run_failed"
                | "run_cancelled"
                | "run_dead_lettered"
                | "recovery_required"
        ),
        MakoEvent::Extension(event) if event.name == "agentic_event" => {
            event
                .payload
                .get("type")
                .and_then(serde_json::Value::as_str)
                == Some("finish")
        }
        MakoEvent::DaemonShuttingDown { .. } => true,
        _ => false,
    }
}

#[cfg(unix)]
fn print_help() {
    println!(
        "krusty-mako {DAEMON_VERSION}\n\n\
         Usage:\n  \
           krusty-mako [daemon] [OPTIONS]\n  \
           krusty-mako ping|stats [--socket PATH] [--key PATH]\n  \
           krusty-mako shutdown [--reason TEXT] [--socket PATH] [--key PATH]\n  \
           krusty-mako dispatch --task TEXT [--working-dir PATH] [--model ID]\n  \
           krusty-mako events --session ID [--after N] [--follow]\n\n\
         Options:\n  \
           --socket <PATH>       Private Unix socket path\n  \
           --key <PATH>          32-byte private IPC key path\n  \
           --instance-id <ID>    Stable identifier for this daemon process\n  \
           --database <PATH>     Shared Krusty SQLite database path\n  \
           --working-dir <PATH>  Default tool working directory\n  \
           --task <TEXT>         Task for diagnostic dispatch\n  \
           --project-dir <PATH>  Project scope for diagnostic dispatch\n  \
           --model <ID>          Model for diagnostic dispatch\n  \
           --priority <NAME>     Priority for diagnostic dispatch\n  \
           --crew <SLUG>         Crew profile for diagnostic dispatch\n  \
           --session <ID>        Session for diagnostic event replay\n  \
           --after <SEQUENCE>    Replay events after this sequence\n  \
           --replay-limit <N>    Maximum replay events (default 1000)\n  \
           --follow              Follow events until a terminal event\n  \
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

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use krusty_core::mako::parse_utc_timestamp;
    use krusty_core::storage::{Database, MakoDaemonLeaseStore};
    use krusty_mako::{
        start_runtime, MakoDaemonConfig, MakoPaths, MakoRuntimeConfig, UnavailableExecutionBackend,
    };
    use krusty_mako_protocol::AuthPolicy;

    use super::bind_server_or_shutdown;

    #[tokio::test]
    async fn bind_failure_gracefully_releases_scheduler_lease() {
        let temp = tempfile::tempdir().unwrap();
        let database_path = temp.path().join("runtime.db");
        let mut runtime_config = MakoRuntimeConfig::for_database(&database_path);
        runtime_config.scheduler_poll_interval = Duration::from_millis(20);
        runtime_config.daemon_lease_duration = Duration::from_millis(500);
        let runtime = start_runtime(
            runtime_config,
            "bind-failure-daemon",
            Arc::new(UnavailableExecutionBackend),
        )
        .await
        .unwrap();

        let owner = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(lease) =
                    MakoDaemonLeaseStore::new(Database::new(&database_path).unwrap())
                        .get("mako-scheduler")
                        .unwrap()
                {
                    break lease.owner_id;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("scheduler did not acquire its startup lease");

        let invalid_parent = temp.path().join("not-a-directory");
        std::fs::write(&invalid_parent, b"occupied").unwrap();
        let daemon_config = MakoDaemonConfig {
            paths: MakoPaths {
                socket_path: invalid_parent.join("mako.sock"),
                key_path: temp.path().join("mako.key"),
            },
            instance_id: "bind-failure-daemon".into(),
            auth_policy: AuthPolicy::default(),
            control_io_timeout: Duration::from_secs(1),
            connection_grace_period: Duration::from_secs(1),
            max_connections: 4,
        };
        assert!(
            bind_server_or_shutdown(daemon_config, runtime)
                .await
                .is_err(),
            "invalid socket parent must fail binding"
        );

        let lease = MakoDaemonLeaseStore::new(Database::new(&database_path).unwrap())
            .get("mako-scheduler")
            .unwrap()
            .expect("released lease should remain as a fencing record");
        assert_eq!(lease.owner_id, owner);
        assert!(parse_utc_timestamp(&lease.expires_at).unwrap() <= Utc::now());
    }
}
