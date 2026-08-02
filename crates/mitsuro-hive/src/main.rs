#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
use anyhow::{bail, Context, Result};
#[cfg(unix)]
use mitsuro_hive::{
    start_runtime, DaemonServer, HiveDaemonConfig, HiveRuntimeConfig, HiveRuntimeHandle,
    MitsuroExecutionBackend, DAEMON_VERSION,
};
#[cfg(unix)]
use mitsuro_hive_protocol::{
    Actor, Command, DispatchCommand, HiveEvent, HiveIpcClient, HiveIpcClientConfig, ModelKey,
    RequestEnvelope, ResponsePayload, ShutdownCommand, SubscribeCommand,
};
#[cfg(unix)]
use mitsuro_server::hive_execution_host::{HiveExecutionHost, HiveExecutionHostConfig};

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<()> {
    mitsuro_core::identity::import_legacy_environment();
    mitsuro_core::identity::require_startup_identity()
        .context("validating Mitsuro configuration authority")?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mitsuro_hive=info")),
        )
        .with_target(false)
        .init();

    let mut config = HiveDaemonConfig::discover().context("discovering Hive daemon paths")?;
    let mut database_path = mitsuro_core::paths::config_dir().join("mitsuro.db");
    let mut working_dir = std::env::current_dir().context("resolving Hive working directory")?;
    let command = parse_arguments(&mut config, &mut database_path, &mut working_dir)?;

    match command {
        CliCommand::Daemon => run_daemon(config, database_path, working_dir).await,
        command => run_diagnostic(&config, &working_dir, command).await,
    }
}

#[cfg(unix)]
async fn run_daemon(
    config: HiveDaemonConfig,
    database_path: PathBuf,
    working_dir: PathBuf,
) -> Result<()> {
    let execution_host = HiveExecutionHost::build(HiveExecutionHostConfig::new(
        database_path.clone(),
        working_dir,
    ))
    .await
    .context("starting the Hive agent execution host")?;
    let backend = Arc::new(MitsuroExecutionBackend::new(execution_host));
    let runtime = start_runtime(
        HiveRuntimeConfig::for_database(database_path),
        config.instance_id.clone(),
        backend,
    )
    .await
    .context("starting the durable Hive scheduler")?;
    let (server, mut runtime) = bind_server_or_shutdown(config, runtime).await?;
    let handle = server.handle();
    let signal_handle = handle.clone();
    let signal_task = tokio::spawn(async move {
        let signal = wait_for_shutdown_signal().await.unwrap_or_else(|error| {
            tracing::error!(error = %error, "Hive signal handler failed");
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
            tracing::error!(error = %failure, "Hive scheduler supervision tripped");
            handle.shutdown("scheduler pump stopped unexpectedly");
            if let Err(error) = serving.await {
                tracing::warn!(error = %error, "Hive IPC shutdown after scheduler failure failed");
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
    config: HiveDaemonConfig,
    runtime: HiveRuntimeHandle,
) -> Result<(DaemonServer, HiveRuntimeHandle)> {
    match DaemonServer::bind(config, runtime.handler()).await {
        Ok(server) => Ok((server, runtime)),
        Err(error) => {
            // Binding happens after the scheduler starts so it can initialize
            // durable state. Give the pump a graceful stop on bind failure;
            // dropping the handle would abort it before releasing its lease.
            runtime.shutdown().await;
            Err(error).context("starting Hive daemon")
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
        model: String,
        model_key: Option<Box<ModelKey>>,
        model_catalog_revision: Option<String>,
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
    config: &mut HiveDaemonConfig,
    database_path: &mut PathBuf,
    working_dir: &mut PathBuf,
) -> Result<CliCommand> {
    let mut mode = None;
    let mut task = None;
    let mut project_dir = None;
    let mut model = None;
    let mut model_provider = None;
    let mut model_auth_scope = None;
    let mut model_api_format = None;
    let mut model_catalog_revision = None;
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
                    bail!("only one Hive subcommand may be specified");
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
            Some("--provider") => {
                model_provider = Some(argument_value(&mut arguments, "--provider")?);
            }
            Some("--auth-scope") => {
                model_auth_scope = Some(argument_value(&mut arguments, "--auth-scope")?);
            }
            Some("--api-format") => {
                model_api_format = Some(argument_value(&mut arguments, "--api-format")?);
            }
            Some("--model-catalog-revision") => {
                model_catalog_revision =
                    Some(argument_value(&mut arguments, "--model-catalog-revision")?);
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
                println!("mitsuro-hive {DAEMON_VERSION}");
                std::process::exit(0);
            }
            Some(value) => bail!("unknown Hive daemon argument: {value}"),
            None => bail!("Hive daemon arguments must be valid UTF-8"),
        }
    }

    match mode.as_deref().unwrap_or("daemon") {
        "daemon" => Ok(CliCommand::Daemon),
        "ping" => Ok(CliCommand::Ping),
        "stats" => Ok(CliCommand::Stats),
        "shutdown" => Ok(CliCommand::Shutdown {
            reason: shutdown_reason,
        }),
        "dispatch" => {
            let (model, model_key, model_catalog_revision) = diagnostic_model_identity(
                model,
                model_provider,
                model_auth_scope,
                model_api_format,
                model_catalog_revision,
            )?;
            Ok(CliCommand::Dispatch {
                task: task
                    .filter(|value| !value.trim().is_empty())
                    .context("dispatch requires --task")?,
                project_dir,
                model,
                model_key: model_key.map(Box::new),
                model_catalog_revision,
                priority,
                crew_slug,
                start_at_unix_ms,
            })
        }
        "events" => Ok(CliCommand::Events {
            session_id: session_id
                .filter(|value| !value.trim().is_empty())
                .context("events requires --session")?,
            after_sequence,
            replay_limit,
            follow,
        }),
        _ => unreachable!("validated Hive subcommand"),
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
fn trimmed(value: String, option: &str) -> Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("{option} cannot be empty");
    }
    Ok(value)
}

#[cfg(unix)]
fn required_trimmed(value: Option<String>, option: &str) -> Result<String> {
    trimmed(
        value.with_context(|| {
            format!("exact model identity requires {option} together with --model")
        })?,
        option,
    )
}

#[cfg(unix)]
fn diagnostic_model_identity(
    model: Option<String>,
    provider: Option<String>,
    auth_scope: Option<String>,
    api_format: Option<String>,
    catalog_revision: Option<String>,
) -> Result<(String, Option<ModelKey>, Option<String>)> {
    let model = model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .context("dispatch requires an explicit --model")?;
    let exact_identity_requested = provider.is_some()
        || auth_scope.is_some()
        || api_format.is_some()
        || catalog_revision.is_some();
    let model_key = if exact_identity_requested {
        Some(ModelKey {
            provider: required_trimmed(provider, "--provider")?,
            model_id: model.clone(),
            auth_scope: auth_scope
                .map(|value| trimmed(value, "--auth-scope"))
                .transpose()?,
            api_format: required_trimmed(api_format, "--api-format")?,
        })
    } else {
        None
    };
    let catalog_revision = catalog_revision
        .map(|value| trimmed(value, "--model-catalog-revision"))
        .transpose()?;
    Ok((model, model_key, catalog_revision))
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
    config: &HiveDaemonConfig,
    working_dir: &std::path::Path,
    command: CliCommand,
) -> Result<()> {
    let mut client_config =
        HiveIpcClientConfig::new(config.paths.socket_path.clone(), "mitsuro-hive-cli");
    client_config.request_timeout = config.control_io_timeout;
    let client = HiveIpcClient::from_key_path_or_create(client_config, &config.paths.key_path)
        .with_context(|| {
            format!(
                "loading or initializing Hive IPC key at {}",
                config.paths.key_path.display()
            )
        })?;
    let actor = Actor::local("mitsuro-hive-cli");

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
            model_key,
            model_catalog_revision,
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
                        model: Some(model),
                        model_key: model_key.map(|key| *key),
                        model_catalog_revision,
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
fn is_terminal_event(event: &HiveEvent) -> bool {
    match event {
        HiveEvent::Runtime(event) => matches!(
            event.event_type.as_str(),
            "run_completed"
                | "run_failed"
                | "run_cancelled"
                | "run_dead_lettered"
                | "recovery_required"
        ),
        HiveEvent::Extension(event) if event.name == "agentic_event" => {
            event
                .payload
                .get("type")
                .and_then(serde_json::Value::as_str)
                == Some("finish")
        }
        HiveEvent::DaemonShuttingDown { .. } => true,
        _ => false,
    }
}

#[cfg(unix)]
fn print_help() {
    println!(
        "mitsuro-hive {DAEMON_VERSION}\n\n\
         Usage:\n  \
           mitsuro-hive [daemon] [OPTIONS]\n  \
           mitsuro-hive ping|stats [--socket PATH] [--key PATH]\n  \
           mitsuro-hive shutdown [--reason TEXT] [--socket PATH] [--key PATH]\n  \
           mitsuro-hive dispatch --task TEXT --model ID [--provider ID --api-format FORMAT] [--working-dir PATH] [--start-at-unix-ms N]\n  \
           mitsuro-hive events --session ID [--after N] [--follow]\n\n\
         Options:\n  \
           --socket <PATH>       Private Unix socket path\n  \
           --key <PATH>          32-byte private IPC key path\n  \
           --instance-id <ID>    Stable identifier for this daemon process\n  \
           --database <PATH>     Shared Mitsuro SQLite database path\n  \
           --working-dir <PATH>  Default tool working directory\n  \
           --task <TEXT>         Task for diagnostic dispatch\n  \
           --project-dir <PATH>  Project scope for diagnostic dispatch\n  \
           --model <ID>          Model for diagnostic dispatch (bare ID uses legacy fallback)\n  \
           --provider <ID>       Provider for an exact diagnostic model key\n  \
           --api-format <FORMAT> Wire format for an exact diagnostic model key\n  \
           --auth-scope <SCOPE>  Optional auth scope for an exact diagnostic model key\n  \
           --model-catalog-revision <REV>  Catalog revision observed at selection\n  \
           --priority <NAME>     Priority for diagnostic dispatch\n  \
           --crew <SLUG>         Crew profile for diagnostic dispatch\n  \
           --start-at-unix-ms N  Do not claim the dispatched run before this Unix time\n  \
           --reason <TEXT>       Audit reason for a diagnostic daemon shutdown\n  \
           --session <ID>        Session for diagnostic event replay\n  \
           --after <SEQUENCE>    Replay events after this sequence\n  \
           --replay-limit <N>    Maximum replay events (default 1000)\n  \
           --follow              Follow events until a terminal event\n  \
           -h, --help            Print help\n  \
           -V, --version         Print version"
    );
}

#[cfg(all(test, unix))]
mod model_identity_tests {
    use super::diagnostic_model_identity;

    #[test]
    fn diagnostic_dispatch_accepts_exact_or_legacy_model_identity() {
        let (_, legacy_key, legacy_revision) =
            diagnostic_model_identity(Some("grok-4.5".into()), None, None, None, None)
                .expect("legacy fallback should remain accepted");
        assert!(legacy_key.is_none());
        assert!(legacy_revision.is_none());

        let (model, key, revision) = diagnostic_model_identity(
            Some(" grok-4.5 ".into()),
            Some("grok".into()),
            Some("oauth".into()),
            Some("open_ai_responses".into()),
            Some("catalog-42".into()),
        )
        .expect("complete exact identity should be accepted");
        assert_eq!(model, "grok-4.5");
        let key = key.expect("exact key should exist");
        assert_eq!(key.provider, "grok");
        assert_eq!(key.model_id, model);
        assert_eq!(key.auth_scope.as_deref(), Some("oauth"));
        assert_eq!(key.api_format, "open_ai_responses");
        assert_eq!(revision.as_deref(), Some("catalog-42"));
    }

    #[test]
    fn diagnostic_dispatch_rejects_partial_exact_model_identity() {
        let error = diagnostic_model_identity(
            Some("grok-4.5".into()),
            Some("grok".into()),
            None,
            None,
            None,
        )
        .expect_err("provider without API format must fail");
        assert!(error.to_string().contains("--api-format"));
    }
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
    eprintln!("mitsuro-hive requires Unix-domain sockets and peer credentials");
    std::process::exit(1);
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use mitsuro_core::hive::parse_utc_timestamp;
    use mitsuro_core::storage::{Database, HiveDaemonLeaseStore};
    use mitsuro_hive::{
        start_runtime, HiveDaemonConfig, HivePaths, HiveRuntimeConfig, UnavailableExecutionBackend,
    };
    use mitsuro_hive_protocol::AuthPolicy;

    use super::bind_server_or_shutdown;

    #[tokio::test]
    async fn bind_failure_gracefully_releases_scheduler_lease() {
        let temp = tempfile::tempdir().unwrap();
        let database_path = temp.path().join("runtime.db");
        let mut runtime_config = HiveRuntimeConfig::for_database(&database_path);
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
                    HiveDaemonLeaseStore::new(Database::new(&database_path).unwrap())
                        .get("hive-scheduler")
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
        let daemon_config = HiveDaemonConfig {
            paths: HivePaths {
                socket_path: invalid_parent.join("hive.sock"),
                key_path: temp.path().join("hive.key"),
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

        let lease = HiveDaemonLeaseStore::new(Database::new(&database_path).unwrap())
            .get("hive-scheduler")
            .unwrap()
            .expect("released lease should remain as a fencing record");
        assert_eq!(lease.owner_id, owner);
        assert!(parse_utc_timestamp(&lease.expires_at).unwrap() <= Utc::now());
    }
}
