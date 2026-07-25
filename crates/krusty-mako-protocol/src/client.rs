use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    AuthPolicy, ClientError, Command, EventEnvelope, IpcKey, ProtocolViolation, RequestEnvelope,
    ResponseEnvelope, ResponseOutcome, ResponsePayload, ServerFrame,
};

#[derive(Debug, Clone)]
pub struct MakoIpcClientConfig {
    pub socket_path: PathBuf,
    pub client_id: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub auth_policy: AuthPolicy,
}

impl MakoIpcClientConfig {
    pub fn new(socket_path: impl Into<PathBuf>, client_id: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            client_id: client_id.into(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            auth_policy: AuthPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct MakoIpcClient {
    config: MakoIpcClientConfig,
    key: Arc<IpcKey>,
}

impl std::fmt::Debug for MakoIpcClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MakoIpcClient")
            .field("config", &self.config)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl MakoIpcClient {
    pub fn new(config: MakoIpcClientConfig, key: IpcKey) -> Self {
        Self {
            config,
            key: Arc::new(key),
        }
    }

    pub fn from_key_path(
        config: MakoIpcClientConfig,
        key_path: impl AsRef<std::path::Path>,
    ) -> Result<Self, ClientError> {
        Ok(Self::new(config, IpcKey::load(key_path.as_ref())?))
    }

    /// Load the shared key or atomically initialize it before connecting.
    ///
    /// Use this only for trusted same-user control clients that are allowed to
    /// bootstrap daemon authority (for example a socket-activating service or
    /// its local diagnostic CLI). Other clients should retain load-only
    /// [`Self::from_key_path`] behavior.
    pub fn from_key_path_or_create(
        config: MakoIpcClientConfig,
        key_path: impl AsRef<std::path::Path>,
    ) -> Result<Self, ClientError> {
        Ok(Self::new(
            config,
            IpcKey::load_or_create(key_path.as_ref())?,
        ))
    }

    pub async fn command(
        &self,
        actor: crate::Actor,
        command: Command,
        idempotency_key: Option<String>,
    ) -> Result<ResponsePayload, ClientError> {
        let timeout_ms = u64::try_from(self.config.request_timeout.as_millis()).unwrap_or(u64::MAX);
        let mut request = RequestEnvelope::new(actor, command, timeout_ms);
        if let Some(idempotency_key) = idempotency_key {
            request.idempotency_key = idempotency_key;
        }
        let response = self.request(request).await?;
        match response.outcome {
            ResponseOutcome::Success { payload } => Ok(payload),
            ResponseOutcome::Failure { error } => Err(ClientError::Remote {
                code: error.code,
                message: error.message,
            }),
        }
    }

    #[cfg(unix)]
    pub async fn request(&self, request: RequestEnvelope) -> Result<ResponseEnvelope, ClientError> {
        request.validate(crate::unix_time_millis())?;
        let request_id = request.request_id.clone();
        let timeout = request_timeout(&self.config, request.deadline_unix_ms)?;
        let command_name = request.command.name();
        let minimum_protocol_minor = request.command.minimum_protocol_minor();
        let mut stream = self
            .connect_and_authenticate(command_name, minimum_protocol_minor)
            .await?;

        tokio::time::timeout(
            timeout,
            crate::write_frame(&mut stream, &crate::ClientFrame::Request(Box::new(request))),
        )
        .await
        .map_err(|_| ClientError::RequestTimeout)??;

        let frame: ServerFrame = tokio::time::timeout(timeout, crate::read_frame(&mut stream))
            .await
            .map_err(|_| ClientError::RequestTimeout)??
            .ok_or(ClientError::Closed)?;
        match frame {
            ServerFrame::Response(response) => {
                response.version.negotiate()?;
                if response.request_id != request_id {
                    return Err(ProtocolViolation::RequestIdMismatch {
                        expected: request_id,
                        actual: response.request_id,
                    }
                    .into());
                }
                Ok(response)
            }
            ServerFrame::Error(error) => Err(ClientError::Remote {
                code: error.code,
                message: error.message,
            }),
            other => Err(ProtocolViolation::UnexpectedFrame {
                expected: "response",
                actual: other.kind(),
            }
            .into()),
        }
    }

    #[cfg(not(unix))]
    pub async fn request(
        &self,
        _request: RequestEnvelope,
    ) -> Result<ResponseEnvelope, ClientError> {
        Err(ClientError::UnsupportedPlatform)
    }

    #[cfg(unix)]
    pub async fn subscribe(
        &self,
        request: RequestEnvelope,
    ) -> Result<EventSubscription, ClientError> {
        if !matches!(request.command, Command::Subscribe(_)) {
            return Err(ProtocolViolation::UnexpectedFrame {
                expected: "subscribe command",
                actual: request.command.name(),
            }
            .into());
        }
        request.validate(crate::unix_time_millis())?;
        let request_id = request.request_id.clone();
        let timeout = request_timeout(&self.config, request.deadline_unix_ms)?;
        let command_name = request.command.name();
        let minimum_protocol_minor = request.command.minimum_protocol_minor();
        let mut stream = self
            .connect_and_authenticate(command_name, minimum_protocol_minor)
            .await?;
        tokio::time::timeout(
            timeout,
            crate::write_frame(&mut stream, &crate::ClientFrame::Request(Box::new(request))),
        )
        .await
        .map_err(|_| ClientError::RequestTimeout)??;

        let frame: ServerFrame = tokio::time::timeout(timeout, crate::read_frame(&mut stream))
            .await
            .map_err(|_| ClientError::RequestTimeout)??
            .ok_or(ClientError::Closed)?;
        if let ServerFrame::Response(response) = &frame {
            response.version.negotiate()?;
        }
        match frame {
            ServerFrame::Response(ResponseEnvelope {
                request_id: actual_request_id,
                outcome:
                    ResponseOutcome::Success {
                        payload: ResponsePayload::SubscriptionAccepted(accepted),
                    },
                ..
            }) if actual_request_id == request_id => Ok(EventSubscription { stream, accepted }),
            ServerFrame::Response(ResponseEnvelope {
                request_id: actual_request_id,
                outcome: ResponseOutcome::Failure { error },
                ..
            }) if actual_request_id == request_id => Err(ClientError::Remote {
                code: error.code,
                message: error.message,
            }),
            ServerFrame::Error(error) => Err(ClientError::Remote {
                code: error.code,
                message: error.message,
            }),
            ServerFrame::Response(response) if response.request_id != request_id => {
                Err(ProtocolViolation::RequestIdMismatch {
                    expected: request_id,
                    actual: response.request_id,
                }
                .into())
            }
            other => Err(ProtocolViolation::UnexpectedFrame {
                expected: "subscription_accepted",
                actual: other.kind(),
            }
            .into()),
        }
    }

    #[cfg(not(unix))]
    pub async fn subscribe(
        &self,
        _request: RequestEnvelope,
    ) -> Result<EventSubscription, ClientError> {
        Err(ClientError::UnsupportedPlatform)
    }

    #[cfg(unix)]
    async fn connect_and_authenticate(
        &self,
        command_name: &'static str,
        minimum_protocol_minor: u16,
    ) -> Result<tokio::net::UnixStream, ClientError> {
        let mut stream = tokio::time::timeout(
            self.config.connect_timeout,
            tokio::net::UnixStream::connect(&self.config.socket_path),
        )
        .await
        .map_err(|_| ClientError::ConnectTimeout)??;
        crate::verify_same_user(&stream)?;

        let hello = self.key.hello(self.config.client_id.clone());
        let expected_nonce = hello.nonce.clone();
        crate::write_frame(&mut stream, &crate::ClientFrame::Hello(hello)).await?;
        let response: ServerFrame =
            tokio::time::timeout(self.config.request_timeout, crate::read_frame(&mut stream))
                .await
                .map_err(|_| ClientError::RequestTimeout)??
                .ok_or(ClientError::Closed)?;

        match response {
            ServerFrame::HelloAck(ack) => {
                let negotiated = ack.version.negotiate()?;
                self.key.verify_hello_ack(
                    &ack,
                    &expected_nonce,
                    self.config.auth_policy,
                    crate::unix_time_millis(),
                )?;
                if negotiated.minor < minimum_protocol_minor {
                    return Err(ProtocolViolation::FeatureRequiresMinor {
                        command: command_name,
                        required_minor: minimum_protocol_minor,
                        actual_minor: negotiated.minor,
                    }
                    .into());
                }
                Ok(stream)
            }
            ServerFrame::Error(error) => Err(ClientError::Remote {
                code: error.code,
                message: error.message,
            }),
            other => Err(ProtocolViolation::UnexpectedFrame {
                expected: "hello_ack",
                actual: other.kind(),
            }
            .into()),
        }
    }
}

#[cfg(unix)]
pub struct EventSubscription {
    stream: tokio::net::UnixStream,
    pub accepted: crate::SubscriptionAccepted,
}

#[cfg(unix)]
impl EventSubscription {
    pub async fn next_event(&mut self) -> Result<Option<EventEnvelope>, ClientError> {
        let Some(frame) = crate::read_frame::<_, ServerFrame>(&mut self.stream).await? else {
            return Ok(None);
        };
        match frame {
            ServerFrame::Event(event) => {
                event.version.negotiate()?;
                Ok(Some(event))
            }
            ServerFrame::Error(error) => Err(ClientError::Remote {
                code: error.code,
                message: error.message,
            }),
            other => Err(ProtocolViolation::UnexpectedFrame {
                expected: "event",
                actual: other.kind(),
            }
            .into()),
        }
    }
}

#[cfg(not(unix))]
pub struct EventSubscription {
    pub accepted: crate::SubscriptionAccepted,
}

#[cfg(not(unix))]
impl EventSubscription {
    pub async fn next_event(&mut self) -> Result<Option<EventEnvelope>, ClientError> {
        Err(ClientError::UnsupportedPlatform)
    }
}

fn request_timeout(
    config: &MakoIpcClientConfig,
    deadline_unix_ms: i64,
) -> Result<Duration, ClientError> {
    let remaining_ms = deadline_unix_ms.saturating_sub(crate::unix_time_millis());
    if remaining_ms <= 0 {
        return Err(ProtocolViolation::DeadlineExpired.into());
    }
    let deadline_timeout = Duration::from_millis(remaining_ms as u64);
    Ok(config.request_timeout.min(deadline_timeout))
}
