use std::fs;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use krusty_mako_protocol::{
    read_frame, unix_time_millis, verify_same_user, write_frame, AckResponse, ClientFrame, Command,
    DaemonStats, IpcKey, MakoEvent, NonceReplayGuard, PongResponse, ProtocolErrorPayload,
    ProtocolVersion, RequestEnvelope, ResponseEnvelope, ResponsePayload, ServerFrame,
};
use tokio::io::AsyncWrite;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use crate::handler::{CommandContext, CommandHandler, HandlerReply};
use crate::{MakoDaemonConfig, DAEMON_VERSION};

const MAX_SHUTDOWN_REASON_BYTES: usize = 1024;

#[derive(Debug)]
pub struct DaemonInfo {
    instance_id: String,
    daemon_version: String,
    started_at: Instant,
}

impl DaemonInfo {
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn daemon_version(&self) -> &str {
        &self.daemon_version
    }

    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }
}

#[derive(Debug, Default)]
struct ServerMetrics {
    active_connections: AtomicUsize,
    handled_requests: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct DaemonServerHandle {
    shutdown_tx: watch::Sender<Option<String>>,
    info: Arc<DaemonInfo>,
}

impl DaemonServerHandle {
    pub fn shutdown(&self, reason: impl Into<String>) {
        self.shutdown_tx
            .send_replace(Some(bounded_shutdown_reason(reason.into())));
    }

    pub fn info(&self) -> Arc<DaemonInfo> {
        Arc::clone(&self.info)
    }
}

pub struct DaemonServer {
    config: MakoDaemonConfig,
    listener: UnixListener,
    owns_socket_path: bool,
    key: Arc<IpcKey>,
    handler: Arc<dyn CommandHandler>,
    nonce_guard: Arc<NonceReplayGuard>,
    metrics: Arc<ServerMetrics>,
    connection_slots: Arc<Semaphore>,
    handle: DaemonServerHandle,
    shutdown_rx: watch::Receiver<Option<String>>,
}

impl DaemonServer {
    pub async fn bind(config: MakoDaemonConfig, handler: Arc<dyn CommandHandler>) -> Result<Self> {
        if config.max_connections == 0 {
            bail!("Mako max_connections must be greater than zero");
        }
        let socket_parent = config
            .paths
            .socket_path
            .parent()
            .context("Mako socket path has no parent directory")?;
        krusty_mako_protocol::ensure_private_dir(socket_parent)
            .context("securing Mako socket directory")?;
        let key = Arc::new(
            IpcKey::load_or_create(&config.paths.key_path)
                .context("loading Mako IPC authentication key")?,
        );

        let (listener, owns_socket_path) = match activated_listener()? {
            Some(listener) => {
                secure_activated_socket(&config.paths.socket_path)?;
                (listener, false)
            }
            None => (bind_private_socket(&config.paths.socket_path).await?, true),
        };

        let info = Arc::new(DaemonInfo {
            instance_id: config.instance_id.clone(),
            daemon_version: DAEMON_VERSION.to_string(),
            started_at: Instant::now(),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(None);
        let handle = DaemonServerHandle { shutdown_tx, info };
        let connection_slots = Arc::new(Semaphore::new(config.max_connections));

        Ok(Self {
            config,
            listener,
            owns_socket_path,
            key,
            handler,
            nonce_guard: Arc::new(NonceReplayGuard::default()),
            metrics: Arc::new(ServerMetrics::default()),
            connection_slots,
            handle,
            shutdown_rx,
        })
    }

    pub fn handle(&self) -> DaemonServerHandle {
        self.handle.clone()
    }

    pub async fn serve(mut self) -> Result<()> {
        let mut socket_cleanup = OwnedSocketCleanup::new(
            self.owns_socket_path
                .then(|| self.config.paths.socket_path.clone()),
        );
        tracing::info!(
            socket = %self.config.paths.socket_path.display(),
            instance_id = %self.handle.info.instance_id,
            version = %self.handle.info.daemon_version,
            "Mako daemon IPC ready"
        );

        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                changed = self.shutdown_rx.changed() => {
                    if changed.is_err() || self.shutdown_rx.borrow().is_some() {
                        break;
                    }
                }
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.context("accepting Mako IPC connection")?;
                    let permit = match Arc::clone(&self.connection_slots).try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            tracing::warn!(
                                max_connections = self.config.max_connections,
                                "Rejecting Mako IPC connection at the configured limit"
                            );
                            drop(stream);
                            continue;
                        }
                    };
                    let connection = ConnectionServices {
                        key: Arc::clone(&self.key),
                        handler: Arc::clone(&self.handler),
                        nonce_guard: Arc::clone(&self.nonce_guard),
                        metrics: Arc::clone(&self.metrics),
                        info: Arc::clone(&self.handle.info),
                        auth_policy: self.config.auth_policy,
                        control_io_timeout: self.config.control_io_timeout,
                        shutdown: self.handle.clone(),
                    };
                    let shutdown_rx = self.shutdown_rx.clone();
                    connections.spawn(async move {
                        let _guard = ActiveConnectionGuard::new(
                            Arc::clone(&connection.metrics),
                            permit,
                        );
                        if let Err(error) = serve_connection(stream, connection, shutdown_rx).await {
                            tracing::warn!(error = %error, "Mako IPC connection ended with an error");
                        }
                    });
                }
                completed = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Err(error)) = completed {
                        tracing::warn!(error = %error, "Mako IPC connection task panicked");
                    }
                }
            }
        }

        // Shutdown reasons can originate in authenticated user input. Keep
        // them out of logs and other unbounded observability fields.
        tracing::info!("Mako daemon stopping");

        let grace_period = self.config.connection_grace_period;
        if tokio::time::timeout(grace_period, async {
            while let Some(result) = connections.join_next().await {
                if let Err(error) = result {
                    tracing::warn!(error = %error, "Mako IPC connection task panicked");
                }
            }
        })
        .await
        .is_err()
        {
            tracing::warn!(?grace_period, "Aborting lingering Mako IPC connections");
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        }

        if self.owns_socket_path {
            remove_owned_socket(&self.config.paths.socket_path)?;
            socket_cleanup.disarm();
        }
        Ok(())
    }
}

struct OwnedSocketCleanup {
    path: Option<PathBuf>,
}

impl OwnedSocketCleanup {
    fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for OwnedSocketCleanup {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if let Err(error) = remove_owned_socket(&path) {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "Failed to remove owned Mako socket during cleanup"
            );
        }
    }
}

#[derive(Clone)]
struct ConnectionServices {
    key: Arc<IpcKey>,
    handler: Arc<dyn CommandHandler>,
    nonce_guard: Arc<NonceReplayGuard>,
    metrics: Arc<ServerMetrics>,
    info: Arc<DaemonInfo>,
    auth_policy: krusty_mako_protocol::AuthPolicy,
    control_io_timeout: Duration,
    shutdown: DaemonServerHandle,
}

struct ActiveConnectionGuard {
    metrics: Arc<ServerMetrics>,
    _permit: OwnedSemaphorePermit,
}

impl ActiveConnectionGuard {
    fn new(metrics: Arc<ServerMetrics>, permit: OwnedSemaphorePermit) -> Self {
        metrics.active_connections.fetch_add(1, Ordering::Relaxed);
        Self {
            metrics,
            _permit: permit,
        }
    }
}

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.metrics
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    services: ConnectionServices,
    mut shutdown_rx: watch::Receiver<Option<String>>,
) -> Result<()> {
    let peer = verify_same_user(&stream).context("rejecting cross-user Mako IPC peer")?;

    let first = read_client_frame(&mut stream, services.control_io_timeout, "hello")
        .await?
        .context("client closed before hello")?;
    let hello = match first {
        ClientFrame::Hello(hello) => hello,
        other => {
            send_protocol_error(
                &mut stream,
                "expected_hello",
                format!("expected hello frame, received {}", other.kind()),
                false,
                services.control_io_timeout,
            )
            .await?;
            return Ok(());
        }
    };

    let negotiated = match hello.version.negotiate() {
        Ok(version) => version,
        Err(error) => {
            send_protocol_error(
                &mut stream,
                "version_mismatch",
                error.to_string(),
                false,
                services.control_io_timeout,
            )
            .await?;
            return Ok(());
        }
    };
    if let Err(error) = services
        .key
        .verify_hello(&hello, services.auth_policy, unix_time_millis())
    {
        send_protocol_error(
            &mut stream,
            "authentication_failed",
            error.to_string(),
            false,
            services.control_io_timeout,
        )
        .await?;
        return Ok(());
    }
    if let Err(error) = services.nonce_guard.check_and_record(
        &hello.nonce,
        services.auth_policy,
        unix_time_millis(),
    ) {
        send_protocol_error(
            &mut stream,
            "authentication_failed",
            error.to_string(),
            false,
            services.control_io_timeout,
        )
        .await?;
        return Ok(());
    }

    let ack = services.key.hello_ack(
        negotiated,
        services.info.instance_id.clone(),
        services.info.daemon_version.clone(),
        hello.nonce,
    );
    write_server_frame(
        &mut stream,
        &ServerFrame::HelloAck(ack),
        services.control_io_timeout,
    )
    .await?;

    let next = read_client_frame(&mut stream, services.control_io_timeout, "request")
        .await?
        .context("client closed before request")?;
    let request = match next {
        ClientFrame::Request(request) => request,
        other => {
            send_protocol_error(
                &mut stream,
                "expected_request",
                format!("expected request frame, received {}", other.kind()),
                false,
                services.control_io_timeout,
            )
            .await?;
            return Ok(());
        }
    };

    if let Err(error) = request.validate(unix_time_millis()) {
        let response = ResponseEnvelope::failure(
            request.request_id,
            ProtocolErrorPayload::new("invalid_request", error.to_string(), false),
        );
        write_server_frame(
            &mut stream,
            &ServerFrame::Response(response),
            services.control_io_timeout,
        )
        .await?;
        return Ok(());
    }
    services
        .metrics
        .handled_requests
        .fetch_add(1, Ordering::Relaxed);

    let request_id = request.request_id.clone();
    let (reply, shutdown_reason) = dispatch_request(*request, peer, &services).await;
    match reply {
        Ok(HandlerReply::Response(payload)) => {
            let response = ResponseEnvelope::success(request_id, payload);
            write_server_frame(
                &mut stream,
                &ServerFrame::Response(response),
                services.control_io_timeout,
            )
            .await?;
        }
        Ok(HandlerReply::Subscription {
            accepted,
            mut events,
        }) => {
            let response = ResponseEnvelope::success(
                request_id,
                ResponsePayload::SubscriptionAccepted(accepted),
            );
            write_server_frame(
                &mut stream,
                &ServerFrame::Response(response),
                services.control_io_timeout,
            )
            .await?;
            let (mut peer_reader, mut event_writer) = stream.into_split();
            // A subscription is server-to-client after its one accepted
            // request. Keep one cancellation-safe read future alive for the
            // entire stream so quiet peer EOF releases the connection permit
            // and drops the handler receiver. Recreating a read on every event
            // could consume a partial frame before select cancellation.
            let peer_input = async move {
                read_frame::<_, ClientFrame>(&mut peer_reader)
                    .await
                    .map_err(anyhow::Error::from)
            };
            tokio::pin!(peer_input);
            loop {
                tokio::select! {
                    peer = &mut peer_input => {
                        match peer? {
                            None => break,
                            Some(frame) => {
                                tracing::warn!(
                                    frame_kind = frame.kind(),
                                    "Closing Mako subscription after unexpected client input"
                                );
                                break;
                            }
                        }
                    }
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || shutdown_rx.borrow().is_some() {
                            let event = krusty_mako_protocol::EventEnvelope {
                                version: ProtocolVersion::CURRENT,
                                session_id: None,
                                run_id: None,
                                sequence: None,
                                emitted_at_unix_ms: unix_time_millis(),
                                event: MakoEvent::DaemonShuttingDown {
                                    reason: shutdown_rx.borrow().clone(),
                                },
                            };
                            let _ = write_server_frame(
                                &mut event_writer,
                                &ServerFrame::Event(event),
                                services.control_io_timeout,
                            )
                            .await;
                            break;
                        }
                    }
                    event = events.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        write_server_frame(
                            &mut event_writer,
                            &ServerFrame::Event(event),
                            services.control_io_timeout,
                        )
                        .await?;
                    }
                }
            }
        }
        Err(error) => {
            let response = ResponseEnvelope::failure(request_id, error);
            write_server_frame(
                &mut stream,
                &ServerFrame::Response(response),
                services.control_io_timeout,
            )
            .await?;
        }
    }

    if let Some(reason) = shutdown_reason {
        services.shutdown.shutdown(reason);
    }
    Ok(())
}

async fn dispatch_request(
    request: RequestEnvelope,
    peer: krusty_mako_protocol::PeerIdentity,
    services: &ConnectionServices,
) -> (Result<HandlerReply, ProtocolErrorPayload>, Option<String>) {
    let remaining_ms = request.deadline_unix_ms.saturating_sub(unix_time_millis());
    if remaining_ms <= 0 {
        return (
            Err(ProtocolErrorPayload::new(
                "deadline_expired",
                "request deadline has expired",
                true,
            )),
            None,
        );
    }
    let timeout = Duration::from_millis(remaining_ms as u64);
    let context = CommandContext {
        request_id: request.request_id,
        idempotency_key: request.idempotency_key,
        actor: request.actor,
        deadline_unix_ms: request.deadline_unix_ms,
        peer,
        daemon_instance_id: services.info.instance_id.clone(),
    };

    match request.command {
        Command::Ping => (
            Ok(HandlerReply::Response(ResponsePayload::Pong(
                PongResponse {
                    instance_id: services.info.instance_id.clone(),
                    daemon_version: services.info.daemon_version.clone(),
                    uptime_secs: services.info.uptime().as_secs(),
                    server_time_unix_ms: unix_time_millis(),
                },
            ))),
            None,
        ),
        Command::Stats => {
            let runtime =
                tokio::time::timeout(timeout, services.handler.runtime_stats(&context.actor)).await;
            match runtime {
                Ok(runtime) => (
                    Ok(HandlerReply::Response(ResponsePayload::Stats(
                        DaemonStats {
                            instance_id: services.info.instance_id.clone(),
                            daemon_version: services.info.daemon_version.clone(),
                            protocol: ProtocolVersion::CURRENT,
                            uptime_secs: services.info.uptime().as_secs(),
                            active_connections: services
                                .metrics
                                .active_connections
                                .load(Ordering::Relaxed),
                            handled_requests: services
                                .metrics
                                .handled_requests
                                .load(Ordering::Relaxed),
                            runtime,
                        },
                    ))),
                    None,
                ),
                Err(_) => (
                    Err(ProtocolErrorPayload::new(
                        "deadline_exceeded",
                        "runtime stats exceeded the request deadline",
                        true,
                    )),
                    None,
                ),
            }
        }
        Command::Shutdown(command) => {
            let reason = match command.reason {
                Some(reason) if reason.len() > MAX_SHUTDOWN_REASON_BYTES => {
                    return (
                        Err(ProtocolErrorPayload::new(
                            "invalid_request",
                            format!("shutdown reason exceeds {MAX_SHUTDOWN_REASON_BYTES} bytes"),
                            false,
                        )),
                        None,
                    );
                }
                Some(reason) => reason,
                None => "requested over authenticated IPC".to_string(),
            };
            (
                Ok(HandlerReply::Response(ResponsePayload::Ack(AckResponse {
                    accepted: true,
                    message: Some("daemon shutdown requested".to_string()),
                }))),
                Some(reason),
            )
        }
        command => {
            let result = tokio::time::timeout(timeout, services.handler.handle(context, command))
                .await
                .unwrap_or_else(|_| {
                    Err(ProtocolErrorPayload::new(
                        "deadline_exceeded",
                        "command handler exceeded the request deadline",
                        true,
                    ))
                });
            (result, None)
        }
    }
}

fn bounded_shutdown_reason(mut reason: String) -> String {
    if reason.len() <= MAX_SHUTDOWN_REASON_BYTES {
        return reason;
    }
    let mut cutoff = MAX_SHUTDOWN_REASON_BYTES.saturating_sub(" [truncated]".len());
    while cutoff > 0 && !reason.is_char_boundary(cutoff) {
        cutoff -= 1;
    }
    reason.truncate(cutoff);
    reason.push_str(" [truncated]");
    reason
}

async fn send_protocol_error(
    stream: &mut UnixStream,
    code: &str,
    message: String,
    retryable: bool,
    timeout: Duration,
) -> Result<()> {
    write_server_frame(
        stream,
        &ServerFrame::Error(ProtocolErrorPayload::new(code, message, retryable)),
        timeout,
    )
    .await
    .context("sending Mako protocol error")
}

async fn read_client_frame(
    stream: &mut UnixStream,
    timeout: Duration,
    phase: &str,
) -> Result<Option<ClientFrame>> {
    tokio::time::timeout(timeout, read_frame(stream))
        .await
        .with_context(|| format!("Mako IPC {phase} read timed out"))?
        .map_err(Into::into)
}

async fn write_server_frame<W>(stream: &mut W, frame: &ServerFrame, timeout: Duration) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    tokio::time::timeout(timeout, write_frame(stream, frame))
        .await
        .context("Mako IPC response write timed out")??;
    Ok(())
}

async fn bind_private_socket(path: &Path) -> Result<UnixListener> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() {
            bail!("refusing to replace non-socket path {}", path.display());
        }
        let expected_uid = krusty_mako_protocol::current_effective_uid();
        if metadata.uid() != expected_uid {
            bail!(
                "refusing to replace socket {} owned by uid {}",
                path.display(),
                metadata.uid()
            );
        }

        match tokio::time::timeout(Duration::from_millis(250), UnixStream::connect(path)).await {
            Ok(Ok(_)) => bail!("Mako daemon is already listening at {}", path.display()),
            Err(_) => bail!(
                "existing Mako socket {} did not respond; refusing destructive takeover",
                path.display()
            ),
            Ok(Err(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) =>
            {
                fs::remove_file(path)
                    .with_context(|| format!("removing stale Mako socket {}", path.display()))?;
            }
            Ok(Err(error)) => {
                return Err(error)
                    .with_context(|| format!("checking existing Mako socket {}", path.display()));
            }
        }
    }

    let listener = UnixListener::bind(path)
        .with_context(|| format!("binding Mako socket {}", path.display()))?;
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        drop(listener);
        let _ = remove_owned_socket(path);
        return Err(error).context("securing newly bound Mako socket");
    }
    Ok(listener)
}

fn secure_activated_socket(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspecting activated Mako socket {}", path.display()))?;
    if !metadata.file_type().is_socket() {
        bail!("activated Mako path {} is not a socket", path.display());
    }
    if metadata.uid() != krusty_mako_protocol::current_effective_uid() {
        bail!("activated Mako socket is owned by another user");
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn remove_owned_socket(path: &Path) -> Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != krusty_mako_protocol::current_effective_uid()
    {
        bail!(
            "refusing to remove unowned or non-socket path {}",
            path.display()
        );
    }
    fs::remove_file(path).with_context(|| format!("removing Mako socket {}", path.display()))
}

fn activated_listener() -> Result<Option<UnixListener>> {
    let Some(listen_pid) = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return Ok(None);
    };
    if listen_pid != std::process::id() {
        return Ok(None);
    }
    let listen_fds = std::env::var("LISTEN_FDS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    if listen_fds == 0 {
        return Ok(None);
    }
    if listen_fds != 1 {
        bail!("expected exactly one systemd-activated socket, received {listen_fds}");
    }

    const SD_LISTEN_FDS_START: i32 = 3;
    // SAFETY: systemd transfers ownership of descriptors beginning at fd 3 to
    // this process when LISTEN_PID/LISTEN_FDS match. We validated both values.
    let standard = unsafe { std::os::unix::net::UnixListener::from_raw_fd(SD_LISTEN_FDS_START) };
    // systemd deliberately passes activation descriptors without CLOEXEC. Set
    // it before spawning any tool processes so children cannot keep the Mako
    // listening socket alive after the daemon exits.
    let descriptor_flags = unsafe { libc::fcntl(standard.as_raw_fd(), libc::F_GETFD) };
    if descriptor_flags < 0 {
        return Err(std::io::Error::last_os_error()).context("reading activated socket flags");
    }
    // SAFETY: `standard` owns a live descriptor and F_SETFD only updates its
    // descriptor-local flags.
    if unsafe {
        libc::fcntl(
            standard.as_raw_fd(),
            libc::F_SETFD,
            descriptor_flags | libc::FD_CLOEXEC,
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error()).context("securing activated socket flags");
    }
    standard.set_nonblocking(true)?;
    Ok(Some(UnixListener::from_std(standard)?))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use krusty_mako_protocol::{
        Actor, Command, DaemonRuntimeStats, ExtensionCommand, ExtensionResponse, MakoIpcClient,
        MakoIpcClientConfig, RequestEnvelope, ResponsePayload, ShutdownCommand, SubscribeCommand,
        SubscriptionAccepted,
    };
    use tokio::sync::mpsc;

    use super::*;
    use crate::handler::{CommandContext, HandlerResult};
    use crate::MakoPaths;

    #[test]
    fn direct_shutdown_reasons_are_utf8_safely_bounded() {
        let reason = "é".repeat(MAX_SHUTDOWN_REASON_BYTES);
        let bounded = bounded_shutdown_reason(reason);
        assert!(bounded.len() <= MAX_SHUTDOWN_REASON_BYTES);
        assert!(bounded.ends_with(" [truncated]"));
    }

    #[derive(Debug)]
    struct TestHandler;

    #[derive(Debug)]
    struct QuietSubscriptionHandler {
        subscribed: AtomicBool,
        bridge_stopped: Arc<AtomicBool>,
    }

    impl QuietSubscriptionHandler {
        fn new() -> Self {
            Self {
                subscribed: AtomicBool::new(false),
                bridge_stopped: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    #[async_trait]
    impl CommandHandler for QuietSubscriptionHandler {
        async fn handle(&self, _context: CommandContext, command: Command) -> HandlerResult {
            let Command::Subscribe(command) = command else {
                return Err(ProtocolErrorPayload::new(
                    "unsupported",
                    "unsupported test command",
                    false,
                ));
            };
            assert!(!self.subscribed.swap(true, Ordering::SeqCst));
            let (sender, events) = mpsc::channel(1);
            let bridge_stopped = Arc::clone(&self.bridge_stopped);
            tokio::spawn(async move {
                sender.closed().await;
                bridge_stopped.store(true, Ordering::SeqCst);
            });
            Ok(HandlerReply::Subscription {
                accepted: SubscriptionAccepted {
                    session_id: command.session_id,
                    high_water_sequence: None,
                },
                events,
            })
        }
    }

    #[async_trait]
    impl CommandHandler for TestHandler {
        async fn handle(&self, _context: CommandContext, command: Command) -> HandlerResult {
            match command {
                Command::Extension(extension) => Ok(HandlerReply::Response(
                    ResponsePayload::Extension(ExtensionResponse {
                        name: extension.name,
                        payload: extension.payload,
                    }),
                )),
                _ => Err(ProtocolErrorPayload::new(
                    "unsupported",
                    "unsupported test command",
                    false,
                )),
            }
        }

        async fn runtime_stats(&self, _actor: &Actor) -> DaemonRuntimeStats {
            DaemonRuntimeStats {
                active_controllers: 4,
                active_runs: 3,
                queued_runs: 2,
                recovery_required: 1,
                pump_alive: true,
                scheduler_ready: true,
            }
        }
    }

    fn test_config(temp: &tempfile::TempDir) -> MakoDaemonConfig {
        MakoDaemonConfig {
            paths: MakoPaths {
                socket_path: temp.path().join("run").join("mako.sock"),
                key_path: temp.path().join("config").join("mako.key"),
            },
            instance_id: "test-instance".to_string(),
            auth_policy: Default::default(),
            control_io_timeout: Duration::from_secs(1),
            connection_grace_period: Duration::from_secs(1),
            max_connections: 16,
        }
    }

    #[tokio::test]
    async fn serves_ping_stats_extension_and_graceful_shutdown() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(&temp);
        let key_path = config.paths.key_path.clone();
        let socket_path = config.paths.socket_path.clone();
        let server = DaemonServer::bind(config, Arc::new(TestHandler))
            .await
            .unwrap();
        let server_task = tokio::spawn(server.serve());

        let client = MakoIpcClient::from_key_path(
            MakoIpcClientConfig::new(socket_path, "test-client"),
            key_path,
        )
        .unwrap();
        let actor = Actor::local("test");

        let pong = client
            .command(actor.clone(), Command::Ping, None)
            .await
            .unwrap();
        assert!(matches!(pong, ResponsePayload::Pong(_)));

        let stats = client
            .command(actor.clone(), Command::Stats, None)
            .await
            .unwrap();
        match stats {
            ResponsePayload::Stats(stats) => {
                assert_eq!(stats.instance_id, "test-instance");
                assert_eq!(stats.runtime.active_controllers, 4);
                assert_eq!(stats.runtime.active_runs, 3);
                assert_eq!(stats.runtime.queued_runs, 2);
                assert_eq!(stats.runtime.recovery_required, 1);
                assert!(stats.runtime.pump_alive);
                assert!(stats.runtime.scheduler_ready);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        let extension = client
            .command(
                actor.clone(),
                Command::Extension(ExtensionCommand {
                    name: "echo".to_string(),
                    payload: serde_json::json!({"ok": true}),
                }),
                Some("extension-once".to_string()),
            )
            .await
            .unwrap();
        assert!(matches!(extension, ResponsePayload::Extension(_)));

        let oversized_shutdown = client
            .command(
                actor.clone(),
                Command::Shutdown(ShutdownCommand {
                    reason: Some("x".repeat(MAX_SHUTDOWN_REASON_BYTES + 1)),
                }),
                None,
            )
            .await
            .expect_err("an oversized shutdown reason must not stop the daemon");
        assert!(matches!(
            oversized_shutdown,
            krusty_mako_protocol::ClientError::Remote { ref code, .. }
                if code == "invalid_request"
        ));

        let still_ready = client
            .command(actor.clone(), Command::Ping, None)
            .await
            .expect("daemon should remain ready after rejecting an oversized reason");
        assert!(matches!(still_ready, ResponsePayload::Pong(_)));

        let shutdown = client
            .command(
                actor,
                Command::Shutdown(ShutdownCommand {
                    reason: Some("test complete".to_string()),
                }),
                None,
            )
            .await
            .unwrap();
        assert!(matches!(shutdown, ResponsePayload::Ack(_)));
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn rejects_a_client_with_the_wrong_key() {
        let temp = tempfile::tempdir().unwrap();
        let config = test_config(&temp);
        let socket_path = config.paths.socket_path.clone();
        let server = DaemonServer::bind(config, Arc::new(TestHandler))
            .await
            .unwrap();
        let handle = server.handle();
        let server_task = tokio::spawn(server.serve());

        let client = MakoIpcClient::new(
            MakoIpcClientConfig::new(socket_path, "wrong-key"),
            IpcKey::generate(),
        );
        let error = client
            .command(Actor::local("test"), Command::Ping, None)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            krusty_mako_protocol::ClientError::Remote { .. }
        ));

        handle.shutdown("test complete");
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn reaps_a_stalled_unauthenticated_connection() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = test_config(&temp);
        config.control_io_timeout = Duration::from_millis(50);
        config.max_connections = 1;
        let key_path = config.paths.key_path.clone();
        let socket_path = config.paths.socket_path.clone();
        let server = DaemonServer::bind(config, Arc::new(TestHandler))
            .await
            .unwrap();
        let handle = server.handle();
        let server_task = tokio::spawn(server.serve());

        let stalled = UnixStream::connect(&socket_path).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = MakoIpcClient::from_key_path(
            MakoIpcClientConfig::new(socket_path, "test-client"),
            key_path,
        )
        .unwrap();
        let response = client
            .command(Actor::local("test"), Command::Ping, None)
            .await
            .unwrap();
        assert!(matches!(response, ResponsePayload::Pong(_)));

        drop(stalled);
        handle.shutdown("test complete");
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn quiet_subscription_disconnect_releases_receiver_and_connection_slot() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = test_config(&temp);
        config.max_connections = 1;
        let key_path = config.paths.key_path.clone();
        let socket_path = config.paths.socket_path.clone();
        let handler = Arc::new(QuietSubscriptionHandler::new());
        let bridge_stopped = Arc::clone(&handler.bridge_stopped);
        let server = DaemonServer::bind(config, handler).await.unwrap();
        let handle = server.handle();
        let server_task = tokio::spawn(server.serve());
        let client = MakoIpcClient::from_key_path(
            MakoIpcClientConfig::new(socket_path, "quiet-subscription-test"),
            key_path,
        )
        .unwrap();

        let subscription = client
            .subscribe(RequestEnvelope::new(
                Actor::local("test"),
                Command::Subscribe(SubscribeCommand {
                    session_id: "quiet-session".into(),
                    after_sequence: Some(0),
                    replay_limit: Some(0),
                }),
                1_000,
            ))
            .await
            .unwrap();
        assert!(!bridge_stopped.load(Ordering::SeqCst));
        drop(subscription);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !bridge_stopped.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disconnect should drop the receiver and stop its quiet bridge task");

        let pong = client
            .command(Actor::local("test"), Command::Ping, None)
            .await
            .expect("the sole connection slot should be reusable");
        assert!(matches!(pong, ResponsePayload::Pong(_)));
        handle.shutdown("test complete");
        server_task.await.unwrap().unwrap();
    }
}
