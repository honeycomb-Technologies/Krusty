use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures::{stream, StreamExt};
use reqwest::redirect::Policy;
use serde::Serialize;

const PORT_PROBE_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProbeStatus {
    Ok,
    Timeout,
    ConnRefused,
    NonHttp,
    Error,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProbeResult {
    pub(super) is_previewable_http: bool,
    pub(super) status: ProbeStatus,
    pub(super) duration_ms: u32,
}

pub(super) async fn probe_ports_previewability(
    ports: &[u16],
    timeout_ms: u16,
) -> HashMap<u16, ProbeResult> {
    let timeout = Duration::from_millis(timeout_ms as u64);
    if ports.is_empty() {
        return HashMap::new();
    }

    stream::iter(ports.iter().copied())
        .map(|port| async move { (port, probe_previewable_port(port, timeout).await) })
        .buffer_unordered(PORT_PROBE_CONCURRENCY)
        .collect::<Vec<(u16, ProbeResult)>>()
        .await
        .into_iter()
        .collect()
}

pub(super) async fn probe_port_previewability(port: u16, timeout_ms: u16) -> ProbeResult {
    probe_previewable_port(port, Duration::from_millis(timeout_ms as u64)).await
}

async fn probe_previewable_port(port: u16, timeout: Duration) -> ProbeResult {
    let start = Instant::now();
    let request = probe_http_client()
        .get(format!("http://127.0.0.1:{}/", port))
        .timeout(timeout)
        .send()
        .await;

    let elapsed = start.elapsed();
    let duration_ms = elapsed.as_millis().min(u128::from(u32::MAX)) as u32;

    match request {
        Ok(response) => {
            let _ = response.bytes().await;
            ProbeResult {
                is_previewable_http: true,
                status: ProbeStatus::Ok,
                duration_ms,
            }
        }
        Err(err) => {
            let err_message = err.to_string();
            let status = classify_probe_error(err.is_timeout(), err.is_connect(), &err_message);
            ProbeResult {
                is_previewable_http: false,
                status,
                duration_ms,
            }
        }
    }
}

fn probe_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .expect("probe client should initialize")
    })
}

fn classify_probe_error(is_timeout: bool, is_connect: bool, message: &str) -> ProbeStatus {
    let message_lower;
    let message_lower = if message.bytes().any(|byte| byte.is_ascii_uppercase()) {
        message_lower = message.to_ascii_lowercase();
        message_lower.as_str()
    } else {
        message
    };

    if is_timeout {
        return ProbeStatus::Timeout;
    }
    if is_connect {
        if message_lower.contains("connection refused")
            || message_lower.contains("failed to connect")
        {
            return ProbeStatus::ConnRefused;
        }
        if message_lower.contains("invalid http")
            || message_lower.contains("http parse")
            || message_lower.contains("connection closed before message completed")
        {
            return ProbeStatus::NonHttp;
        }
    }
    if message_lower.contains("invalid http")
        || message_lower.contains("http parse")
        || message_lower.contains("connection closed before message completed")
    {
        return ProbeStatus::NonHttp;
    }
    ProbeStatus::Error
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn classify_probe_error_maps_timeout_connrefused_and_non_http() {
        assert_eq!(
            classify_probe_error(true, false, "deadline elapsed"),
            ProbeStatus::Timeout
        );
        assert_eq!(
            classify_probe_error(true, true, "connection refused"),
            ProbeStatus::Timeout
        );
        assert_eq!(
            classify_probe_error(false, true, "connection refused"),
            ProbeStatus::ConnRefused
        );
        assert_eq!(
            classify_probe_error(false, true, "invalid HTTP version parsed"),
            ProbeStatus::NonHttp
        );
        assert_eq!(
            classify_probe_error(false, false, "http parse error"),
            ProbeStatus::NonHttp
        );
    }

    #[tokio::test]
    async fn probe_previewable_port_accepts_http_statuses() {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "skipping probe_previewable_port_accepts_http_statuses: bind permission denied ({err})"
                );
                return;
            }
            Err(err) => panic!("bind test http listener: {err}"),
        };
        let port = listener.local_addr().expect("listener local addr").port();

        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let result = probe_previewable_port(port, Duration::from_millis(1000)).await;
        server.await.expect("join test http server");

        assert!(result.is_previewable_http);
        assert_eq!(result.status, ProbeStatus::Ok);
        assert!(result.duration_ms <= 1000);
    }
}
