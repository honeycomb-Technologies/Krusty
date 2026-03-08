//! WebSocket terminal handler with PTY support.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::AppState;

const MAX_INPUT_SIZE: usize = 64 * 1024;
const MAX_TERMINAL_COLS: u16 = 500;
const MAX_TERMINAL_ROWS: u16 = 500;
const MAX_OUTPUT_BATCH_BYTES: usize = 64 * 1024;
const OUTPUT_COALESCE_WINDOW: Duration = Duration::from_millis(4);

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Hello { binary_output: Option<bool> },
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Ping,
}

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn send_ws_error(sink: &mut futures::stream::SplitSink<WebSocket, Message>, msg: &str) {
    let error = serde_json::json!({ "type": "error", "error": msg });
    let _ = sink.send(Message::Text(error.to_string())).await;
}

fn clamp_terminal_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.clamp(1, MAX_TERMINAL_ROWS),
        cols: cols.clamp(1, MAX_TERMINAL_COLS),
        pixel_width: 0,
        pixel_height: 0,
    }
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();
    let pty_system = native_pty_system();

    let pair = match pty_system.openpty(clamp_terminal_size(80, 24)) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("Failed to open PTY: {}", e);
            send_ws_error(&mut ws_sink, &format!("Failed to open PTY: {}", e)).await;
            return;
        }
    };

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.cwd(&*state.working_dir);

    let child = match pair.slave.spawn_command(cmd) {
        Ok(child) => child,
        Err(e) => {
            tracing::error!("Failed to spawn shell: {}", e);
            send_ws_error(&mut ws_sink, &format!("Failed to spawn shell: {}", e)).await;
            return;
        }
    };

    let process_id = uuid::Uuid::new_v4().to_string();
    state
        .process_registry
        .register_external(
            process_id.clone(),
            shell,
            Some("Terminal session".to_string()),
            child.process_id(),
            (*state.working_dir).clone(),
        )
        .await;

    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(e) => {
            tracing::error!("Failed to clone PTY reader: {}", e);
            send_ws_error(&mut ws_sink, &format!("Failed to clone PTY reader: {}", e)).await;
            return;
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(e) => {
            tracing::error!("Failed to take PTY writer: {}", e);
            send_ws_error(&mut ws_sink, &format!("Failed to take PTY writer: {}", e)).await;
            return;
        }
    };

    let (output_tx, mut output_rx) = mpsc::channel::<Vec<u8>>(256);

    let reader_handle = {
        let tx = output_tx.clone();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    };

    let ws_sink = Arc::new(tokio::sync::Mutex::new(ws_sink));
    let binary_output = Arc::new(AtomicBool::new(false));
    let ws_sender_handle = {
        let ws_sink = Arc::clone(&ws_sink);
        let binary_output = Arc::clone(&binary_output);
        tokio::spawn(async move {
            let mut pending_output: Option<Vec<u8>> = None;
            loop {
                let mut batch = if let Some(pending) = pending_output.take() {
                    pending
                } else {
                    match output_rx.recv().await {
                        Some(data) => data,
                        None => break,
                    }
                };

                let deadline = tokio::time::Instant::now() + OUTPUT_COALESCE_WINDOW;
                while batch.len() < MAX_OUTPUT_BATCH_BYTES {
                    let now = tokio::time::Instant::now();
                    if now >= deadline {
                        break;
                    }

                    match tokio::time::timeout(deadline - now, output_rx.recv()).await {
                        Ok(Some(next)) => {
                            if batch.len() + next.len() > MAX_OUTPUT_BATCH_BYTES {
                                pending_output = Some(next);
                                break;
                            }
                            batch.extend_from_slice(&next);
                        }
                        Ok(None) | Err(_) => break,
                    }
                }

                let send_result = if binary_output.load(Ordering::Relaxed) {
                    ws_sink.lock().await.send(Message::Binary(batch)).await
                } else {
                    let msg = serde_json::json!({
                        "type": "output",
                        "data": String::from_utf8_lossy(&batch),
                    });
                    ws_sink
                        .lock()
                        .await
                        .send(Message::Text(msg.to_string()))
                        .await
                };

                if send_result.is_err() {
                    break;
                }
            }
        })
    };

    let master = Arc::new(tokio::sync::Mutex::new(pair.master));
    {
        let master = Arc::clone(&master);
        let ws_sink = Arc::clone(&ws_sink);
        let binary_output = Arc::clone(&binary_output);
        let mut writer = writer;
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                Message::Text(ref text) if text.len() > MAX_INPUT_SIZE => {
                    tracing::warn!(
                        "Rejected oversized WebSocket message ({} bytes)",
                        text.len()
                    );
                }
                Message::Text(text) => {
                    if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                        match client_msg {
                            ClientMessage::Hello {
                                binary_output: flag,
                            } => {
                                if let Some(enabled) = flag {
                                    binary_output.store(enabled, Ordering::Relaxed);
                                }
                            }
                            ClientMessage::Input { data } => {
                                if data.len() > MAX_INPUT_SIZE {
                                    tracing::warn!(
                                        "Rejected oversized terminal input ({} bytes)",
                                        data.len()
                                    );
                                    continue;
                                }
                                use std::io::Write;
                                let _ = writer.write_all(data.as_bytes());
                                let _ = writer.flush();
                            }
                            ClientMessage::Resize { cols, rows } => {
                                let m = master.lock().await;
                                let _ = m.resize(clamp_terminal_size(cols, rows));
                            }
                            ClientMessage::Ping => {
                                let mut sink = ws_sink.lock().await;
                                if sink
                                    .send(Message::Text(r#"{"type":"pong"}"#.to_string()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                }
                Message::Binary(data) => {
                    if data.len() > MAX_INPUT_SIZE {
                        tracing::warn!(
                            "Rejected oversized binary terminal input ({} bytes)",
                            data.len()
                        );
                        continue;
                    }
                    use std::io::Write;
                    let _ = writer.write_all(&data);
                    let _ = writer.flush();
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    }

    drop(output_tx);
    // PTY reader runs blocking I/O that cannot be gracefully cancelled, so abort is appropriate.
    reader_handle.abort();
    let _ = ws_sender_handle.await;

    {
        let m = master.lock().await;
        drop(m);
    }
    state.process_registry.unregister(&process_id).await;
    tracing::debug!(process_id = %process_id, "Terminal session closed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_terminal_size_bounds() {
        let clamped = clamp_terminal_size(0, 900);
        assert_eq!(clamped.cols, 1);
        assert_eq!(clamped.rows, MAX_TERMINAL_ROWS);
    }

    #[test]
    fn parses_hello_and_ping_messages() {
        let hello: ClientMessage =
            serde_json::from_str(r#"{"type":"hello","binary_output":true}"#).unwrap();
        assert!(matches!(
            hello,
            ClientMessage::Hello {
                binary_output: Some(true)
            }
        ));

        let ping: ClientMessage = serde_json::from_str(r#"{"type":"ping"}"#).unwrap();
        assert!(matches!(ping, ClientMessage::Ping));
    }
}
