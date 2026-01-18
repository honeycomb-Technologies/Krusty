//! LSP client - JSON-RPC communication with a single language server

use anyhow::{anyhow, Result};
use lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidOpenTextDocumentParams, InitializeParams,
    InitializeResult, InitializedParams, PublishDiagnosticsParams, TextDocumentContentChangeEvent,
    TextDocumentItem, Uri, VersionedTextDocumentIdentifier, WorkspaceFolder,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info};

use super::diagnostics::DiagnosticsCache;
use super::transport::StdioTransport;

/// JSON-RPC request
#[derive(Debug, Serialize)]
struct Request {
    jsonrpc: &'static str,
    id: i64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

/// JSON-RPC notification (no id)
#[derive(Debug, Serialize)]
struct Notification {
    jsonrpc: &'static str,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

/// JSON-RPC response
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(rename = "jsonrpc")]
    _jsonrpc: String,
    id: Option<i64>,
    result: Option<Value>,
    error: Option<RpcError>,
    #[serde(default)]
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// LSP client for a single language server
pub struct LspClient {
    name: String,
    transport: Arc<StdioTransport>,
    next_id: AtomicI64,
    pending_requests: Arc<RwLock<HashMap<i64, tokio::sync::oneshot::Sender<Result<Value>>>>>,
    diagnostics: Arc<DiagnosticsCache>,
    /// Health status - set to false when receive loop encounters an error
    healthy: Arc<AtomicBool>,
    /// Keep child handle alive so process doesn't terminate
    _child: Child,
}

impl LspClient {
    /// Spawn a new language server and create client
    pub async fn spawn(
        name: &str,
        command: &str,
        args: &[&str],
        root_path: &Path,
        diagnostics: Arc<DiagnosticsCache>,
    ) -> Result<Self> {
        info!("Spawning LSP server: {} {} {:?}", name, command, args);

        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(root_path)
            .kill_on_drop(true)
            .spawn()?;

        let transport = Arc::new(StdioTransport::from_tokio_child(&mut child)?);

        let client = Self {
            name: name.to_string(),
            transport,
            next_id: AtomicI64::new(1),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            diagnostics,
            healthy: Arc::new(AtomicBool::new(true)),
            _child: child,
        };

        Ok(client)
    }

    /// Initialize the language server
    pub async fn initialize(&self, root_uri: Uri) -> Result<InitializeResult> {
        // Extract workspace name from URI path
        let workspace_name = root_uri
            .path()
            .segments()
            .next_back()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "workspace".to_string());

        let params = InitializeParams {
            // Use workspace_folders instead of deprecated root_uri
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: workspace_name,
            }]),
            capabilities: ClientCapabilities::default(),
            ..Default::default()
        };

        let result: InitializeResult = self
            .request("initialize", Some(serde_json::to_value(params)?))
            .await?;

        // Send initialized notification
        self.notify(
            "initialized",
            Some(serde_json::to_value(InitializedParams {})?),
        )
        .await?;

        info!("LSP server {} initialized", self.name);

        Ok(result)
    }

    /// Send a request and wait for response
    pub async fn request<R: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<R> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let request = Request {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let json = serde_json::to_string(&request)?;
        debug!("LSP request [{}]: {}", id, method);

        // Create response channel
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_requests.write().await.insert(id, tx);

        // Send request
        self.transport.send(&json).await?;

        // Wait for response
        let result = rx.await??;

        Ok(serde_json::from_value(result)?)
    }

    /// Send a notification (no response expected)
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notification = Notification {
            jsonrpc: "2.0",
            method: method.to_string(),
            params,
        };

        let json = serde_json::to_string(&notification)?;
        debug!("LSP notification: {}", method);

        self.transport.send(&json).await
    }

    /// Start the message receive loop
    pub fn start_receive_loop(
        self: Arc<Self>,
    ) -> mpsc::UnboundedReceiver<PublishDiagnosticsParams> {
        let (tx, rx) = mpsc::unbounded_channel();
        let client = self;

        tokio::spawn(async move {
            loop {
                match client.transport.receive().await {
                    Ok(message) => {
                        if let Err(e) = client.handle_message(&message, &tx).await {
                            error!("Error handling LSP message: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("LSP {} receive error: {}", client.name, e);
                        client.healthy.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });

        rx
    }

    /// Check if the LSP server is healthy (receive loop still running)
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
    }

    /// Get the server name
    pub fn name(&self) -> &str {
        &self.name
    }

    async fn handle_message(
        &self,
        message: &str,
        diagnostics_tx: &mpsc::UnboundedSender<PublishDiagnosticsParams>,
    ) -> Result<()> {
        let response: Response = serde_json::from_str(message)?;

        // Check if it's a response to a request
        if let Some(id) = response.id {
            let mut pending = self.pending_requests.write().await;
            if let Some(tx) = pending.remove(&id) {
                if let Some(error) = response.error {
                    let _ = tx.send(Err(anyhow!("LSP error {}: {}", error.code, error.message)));
                } else {
                    let _ = tx.send(Ok(response.result.unwrap_or(Value::Null)));
                }
            }
            return Ok(());
        }

        // Handle notifications
        if let Some(method) = &response.method {
            match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = response.params {
                        let diag: PublishDiagnosticsParams = serde_json::from_value(params)?;

                        info!(
                            "LSP diagnostics received: {} items for {:?}",
                            diag.diagnostics.len(),
                            diag.uri
                        );

                        // Update cache
                        self.diagnostics
                            .update(diag.uri.clone(), diag.diagnostics.clone());

                        // Forward to channel
                        let _ = diagnostics_tx.send(diag);
                    }
                }
                "window/logMessage" | "window/showMessage" => {
                    // Log server messages
                    if let Some(params) = response.params {
                        if let Some(message) = params.get("message").and_then(|v| v.as_str()) {
                            debug!("LSP [{}]: {}", self.name, message);
                        }
                    }
                }
                _ => {
                    debug!("Unhandled LSP notification: {}", method);
                }
            }
        }

        Ok(())
    }

    /// Notify server that a document changed
    pub async fn did_change(&self, uri: Uri, version: i32, text: &str) -> Result<()> {
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_string(),
            }],
        };

        self.notify(
            "textDocument/didChange",
            Some(serde_json::to_value(params)?),
        )
        .await
    }

    /// Notify server that a document was opened
    pub async fn did_open(
        &self,
        uri: Uri,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> Result<()> {
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: language_id.to_string(),
                version,
                text: text.to_string(),
            },
        };

        self.notify("textDocument/didOpen", Some(serde_json::to_value(params)?))
            .await
    }
}
