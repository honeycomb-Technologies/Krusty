# Agent Client Protocol (ACP) Implementation Plan for Krusty

## Executive Summary

This document outlines the implementation plan for adding Agent Client Protocol (ACP) support to Krusty, enabling it to function as an ACP-compatible agent that can integrate with any ACP-supporting editor (Zed, Neovim, Marimo, JetBrains IDEs).

**Goal**: Transform Krusty into a standards-compliant ACP agent while maintaining full backward compatibility with the existing CLI/TUI interface.

---

## 1. ACP Protocol Overview

### 1.1 What is ACP?

The Agent Client Protocol (ACP) standardizes communication between **code editors** (clients) and **AI coding agents** (servers). It's analogous to LSP (Language Server Protocol) but for AI agents instead of language servers.

**Key characteristics:**
- JSON-RPC 2.0 based protocol
- Bidirectional communication (requests + notifications)
- Transport-agnostic (stdio, HTTP, WebSocket)
- Capability negotiation during initialization
- Seamless MCP (Model Context Protocol) integration

### 1.2 Protocol Architecture

```
┌─────────────────────┐     JSON-RPC 2.0      ┌─────────────────────┐
│      Editor         │◄────── stdio ────────►│      Agent          │
│    (ACP Client)     │                       │   (ACP Server)      │
│                     │   initialize          │                     │
│  - Zed              │   session/new         │  - Krusty (target)  │
│  - Neovim           │   session/prompt      │  - Claude Code      │
│  - JetBrains        │   session/update      │  - Gemini CLI       │
│  - Marimo           │   fs/read_text_file   │  - Codex CLI        │
│                     │   terminal/create     │  - goose            │
└─────────────────────┘                       └─────────────────────┘
```

### 1.3 Message Flow

```
Client                                  Agent
  │                                       │
  │────── initialize ────────────────────►│
  │◄───── InitializeResponse ────────────│
  │                                       │
  │────── session/new ───────────────────►│
  │◄───── NewSessionResponse ────────────│
  │                                       │
  │────── session/prompt ────────────────►│
  │◄───── session/update (streaming) ────│
  │◄───── session/update (tool call) ────│
  │◄───── fs/read_text_file ─────────────│  (agent requests file)
  │────── ReadTextFileResponse ──────────►│
  │◄───── session/update (content) ──────│
  │◄───── PromptResponse ────────────────│
  │                                       │
  │────── session/cancel ────────────────►│ (notification)
  │                                       │
```

---

## 2. Krusty Architecture Analysis

### 2.1 Current Architecture

```
Krusty/
├── crates/
│   ├── krusty-core/              # Shared library
│   │   └── src/
│   │       ├── agent/            # Event bus, hooks, sub-agents
│   │       ├── ai/               # Multi-provider AI client
│   │       ├── tools/            # Tool registry (14+ tools)
│   │       ├── mcp/              # MCP client (already implemented)
│   │       ├── skills/           # Modular instructions
│   │       └── storage/          # SQLite persistence
│   └── krusty-cli/               # Terminal UI (ratatui)
└── Cargo.toml
```

### 2.2 Alignment Points

| Krusty Component | ACP Equivalent | Integration Strategy |
|------------------|----------------|---------------------|
| `ai/client` | Prompt processing | Reuse for LLM calls |
| `tools/registry` | Tool execution | Map to ACP tool calls |
| `mcp/` | MCP integration | Pass-through to editor |
| `agent/event_bus` | Session updates | Emit as ACP notifications |
| `storage/sessions` | Session management | Adapt for ACP sessions |
| `agent/hooks` | Permission requests | Trigger on sensitive ops |

### 2.3 Key Design Decisions

1. **Modular ACP Module**: Create new `krusty-core/src/acp/` module, isolated from existing code
2. **Dual Interface**: Support both TUI and ACP modes via feature flags or runtime detection
3. **Shared Core**: Reuse AI client, tools, and agent logic for both interfaces
4. **No Breaking Changes**: Existing CLI/TUI functionality remains unchanged

---

## 3. Implementation Architecture

### 3.1 New Module Structure

```
krusty-core/src/acp/
├── mod.rs                    # Module root, public API
├── protocol.rs               # JSON-RPC types, ACP message definitions
├── agent.rs                  # Agent trait implementation
├── session.rs                # Session state management
├── transport/
│   ├── mod.rs
│   └── stdio.rs              # Stdio transport layer
├── handlers/
│   ├── mod.rs
│   ├── initialize.rs         # Initialize handler
│   ├── authenticate.rs       # Authentication handler
│   ├── session.rs            # Session lifecycle handlers
│   ├── prompt.rs             # Prompt processing
│   └── cancel.rs             # Cancellation handling
├── capabilities.rs           # Capability negotiation
├── updates.rs                # Session update streaming
└── error.rs                  # ACP-specific errors
```

### 3.2 Core Components

#### 3.2.1 Protocol Types (`protocol.rs`)

```rust
use serde::{Deserialize, Serialize};

/// Protocol version (major only, incremented on breaking changes)
pub const PROTOCOL_VERSION: u16 = 10;

/// JSON-RPC 2.0 request
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,  // "2.0"
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 notification (no id, no response expected)
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// ACP error codes (JSON-RPC 2.0 + ACP extensions)
#[derive(Debug, Clone, Copy)]
pub enum ErrorCode {
    // JSON-RPC standard errors
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
    // ACP-specific errors
    AuthenticationRequired = -32000,
    ResourceNotFound = -32002,
}
```

#### 3.2.2 Agent Implementation (`agent.rs`)

```rust
use async_trait::async_trait;
use crate::ai::AiClient;
use crate::tools::ToolRegistry;

/// Krusty's ACP Agent implementation
pub struct KrustyAgent {
    /// AI client for LLM calls
    ai_client: Arc<AiClient>,
    /// Tool registry
    tools: Arc<ToolRegistry>,
    /// Active sessions
    sessions: DashMap<SessionId, SessionState>,
    /// Session ID counter
    next_session_id: AtomicU64,
    /// Client capabilities (received during init)
    client_capabilities: RwLock<Option<ClientCapabilities>>,
    /// Update sender for streaming
    update_tx: mpsc::UnboundedSender<SessionUpdate>,
}

#[async_trait]
impl Agent for KrustyAgent {
    async fn initialize(
        &self,
        request: InitializeRequest,
    ) -> Result<InitializeResponse, AcpError> {
        // Store client capabilities
        *self.client_capabilities.write().await = Some(request.capabilities);

        Ok(InitializeResponse {
            protocol_version: PROTOCOL_VERSION,
            capabilities: self.agent_capabilities(),
            agent_info: Implementation {
                name: "krusty".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                display_title: Some("Krusty AI Agent".to_string()),
            },
        })
    }

    async fn new_session(
        &self,
        request: NewSessionRequest,
    ) -> Result<NewSessionResponse, AcpError> {
        let session_id = self.next_session_id.fetch_add(1, Ordering::SeqCst);
        let session = SessionState::new(
            session_id,
            request.cwd,
            request.mcp_servers,
        );
        self.sessions.insert(session_id.into(), session);

        Ok(NewSessionResponse {
            session_id: session_id.into(),
        })
    }

    async fn prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptResponse, AcpError> {
        // Process through existing agent loop
        // Stream updates via session/update notifications
        // Return final response
    }

    async fn cancel(&self, session_id: SessionId) {
        if let Some(session) = self.sessions.get(&session_id) {
            session.cancel();
        }
    }
}
```

#### 3.2.3 Session Management (`session.rs`)

```rust
/// Per-session state
pub struct SessionState {
    pub id: SessionId,
    pub cwd: PathBuf,
    pub mcp_servers: Vec<McpServerConfig>,
    pub mode: Option<String>,
    pub messages: Vec<Message>,
    pub cancelled: AtomicBool,
    pub tool_context: ToolContext,
}

impl SessionState {
    pub fn new(
        id: u64,
        cwd: Option<PathBuf>,
        mcp_servers: Option<Vec<McpServerConfig>>,
    ) -> Self {
        Self {
            id: id.into(),
            cwd: cwd.unwrap_or_else(|| std::env::current_dir().unwrap()),
            mcp_servers: mcp_servers.unwrap_or_default(),
            mode: None,
            messages: Vec::new(),
            cancelled: AtomicBool::new(false),
            tool_context: ToolContext::default(),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}
```

#### 3.2.4 Transport Layer (`transport/stdio.rs`)

```rust
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Stdio transport for ACP communication
pub struct StdioTransport {
    reader: BufReader<tokio::io::Stdin>,
    writer: tokio::io::Stdout,
}

impl StdioTransport {
    pub fn new() -> Self {
        Self {
            reader: BufReader::new(tokio::io::stdin()),
            writer: tokio::io::stdout(),
        }
    }

    /// Read next JSON-RPC message (newline-delimited)
    pub async fn read_message(&mut self) -> Result<IncomingMessage, TransportError> {
        let mut line = String::new();
        self.reader.read_line(&mut line).await?;

        if line.is_empty() {
            return Err(TransportError::ConnectionClosed);
        }

        let message: IncomingMessage = serde_json::from_str(&line)?;
        Ok(message)
    }

    /// Write JSON-RPC message (newline-delimited)
    pub async fn write_message(&mut self, message: &OutgoingMessage) -> Result<(), TransportError> {
        let json = serde_json::to_string(message)?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}
```

#### 3.2.5 Capability Negotiation (`capabilities.rs`)

```rust
/// Agent capabilities advertised during initialization
pub fn agent_capabilities() -> AgentCapabilities {
    AgentCapabilities {
        // Prompt capabilities
        prompt: Some(PromptCapabilities {
            image: false,           // No image support initially
            audio: false,           // No audio support initially
            embedded_context: true, // Support embedded resources
        }),
        // Session capabilities
        session: Some(SessionCapabilities {
            load_session: true,     // Support session persistence
            modes: Some(vec![       // Supported modes
                "code".to_string(),
                "architect".to_string(),
                "ask".to_string(),
            ]),
        }),
        // MCP capabilities
        mcp: Some(McpCapabilities {
            stdio: true,            // Support stdio MCP servers
            http: false,            // No HTTP MCP initially
            sse: false,             // No SSE MCP initially
        }),
        // Implementation info
        implementation: Some(Implementation {
            name: "krusty".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            display_title: Some("Krusty AI Agent".to_string()),
        }),
    }
}

/// Required client capabilities for full functionality
pub fn required_client_capabilities() -> Vec<&'static str> {
    vec![
        "fs.readTextFile",      // File reading
        "fs.writeTextFile",     // File writing
        "terminal",             // Terminal execution
    ]
}
```

### 3.3 Integration with Existing Krusty Components

#### 3.3.1 Tool Registry Bridge

```rust
/// Convert Krusty tool calls to ACP tool call updates
pub fn tool_call_to_acp_update(
    tool_name: &str,
    tool_input: serde_json::Value,
    call_id: &str,
) -> SessionUpdate {
    SessionUpdate::ToolCall(ToolCallUpdate {
        id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        input: tool_input,
        status: ToolCallStatus::Running,
        content: None,
    })
}

/// Convert Krusty tool result to ACP tool call update
pub fn tool_result_to_acp_update(
    call_id: &str,
    result: ToolResult,
) -> SessionUpdate {
    SessionUpdate::ToolCall(ToolCallUpdate {
        id: call_id.to_string(),
        tool_name: String::new(), // Already known from previous update
        input: serde_json::Value::Null,
        status: ToolCallStatus::Completed,
        content: Some(result_to_content(result)),
    })
}
```

#### 3.3.2 Permission Request Integration

```rust
/// Request permission from client for sensitive operations
pub async fn request_permission(
    client: &impl Client,
    session_id: SessionId,
    tool_call: &ToolCall,
    options: Vec<PermissionOption>,
) -> Result<PermissionOutcome, AcpError> {
    let request = RequestPermissionRequest {
        session_id,
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        tool_input: tool_call.input.clone(),
        options,
    };

    client.request_permission(request).await
}
```

#### 3.3.3 File Operations Bridge

```rust
/// Delegate file reading to client (editor has file access)
pub async fn read_file_via_client(
    client: &impl Client,
    session_id: SessionId,
    path: &Path,
    start_line: Option<u32>,
    end_line: Option<u32>,
) -> Result<String, AcpError> {
    let request = ReadTextFileRequest {
        session_id,
        path: path.to_string_lossy().to_string(),
        start_line,
        end_line,
    };

    let response = client.read_text_file(request).await?;
    Ok(response.content)
}

/// Delegate file writing to client
pub async fn write_file_via_client(
    client: &impl Client,
    session_id: SessionId,
    path: &Path,
    content: &str,
) -> Result<(), AcpError> {
    let request = WriteTextFileRequest {
        session_id,
        path: path.to_string_lossy().to_string(),
        content: content.to_string(),
    };

    client.write_text_file(request).await?;
    Ok(())
}
```

---

## 4. Implementation Phases

### Phase 1: Core Protocol Layer (Foundation)

**Objective**: Establish JSON-RPC communication infrastructure

**Tasks**:
1. Add `agent-client-protocol` crate dependency
2. Create `acp/` module structure
3. Implement stdio transport layer
4. Implement JSON-RPC message parsing/serialization
5. Add basic error handling

**Files to Create**:
- `krusty-core/src/acp/mod.rs`
- `krusty-core/src/acp/protocol.rs`
- `krusty-core/src/acp/transport/mod.rs`
- `krusty-core/src/acp/transport/stdio.rs`
- `krusty-core/src/acp/error.rs`

**Tests**:
- JSON-RPC message roundtrip
- Stdio transport read/write
- Error code mapping

### Phase 2: Agent Implementation (Core Logic)

**Objective**: Implement the ACP Agent trait

**Tasks**:
1. Implement `initialize` handler with capability negotiation
2. Implement `authenticate` handler (API key validation)
3. Implement `new_session` / `load_session` handlers
4. Implement session state management
5. Implement `set_session_mode` handler

**Files to Create**:
- `krusty-core/src/acp/agent.rs`
- `krusty-core/src/acp/session.rs`
- `krusty-core/src/acp/capabilities.rs`
- `krusty-core/src/acp/handlers/mod.rs`
- `krusty-core/src/acp/handlers/initialize.rs`
- `krusty-core/src/acp/handlers/session.rs`

**Tests**:
- Initialize handshake
- Session creation/loading
- Mode switching

### Phase 3: Prompt Processing (AI Integration)

**Objective**: Connect prompt handling to existing AI client

**Tasks**:
1. Implement `prompt` handler
2. Bridge to existing `AiClient` for LLM calls
3. Implement streaming session updates
4. Handle content types (text, resources)
5. Implement `cancel` notification handler

**Files to Create/Modify**:
- `krusty-core/src/acp/handlers/prompt.rs`
- `krusty-core/src/acp/handlers/cancel.rs`
- `krusty-core/src/acp/updates.rs`

**Tests**:
- Prompt processing
- Streaming updates
- Cancellation handling

### Phase 4: Tool Execution (Tool Integration)

**Objective**: Integrate tool calls with ACP protocol

**Tasks**:
1. Map Krusty tools to ACP tool call format
2. Implement tool call streaming updates
3. Implement permission request flow
4. Bridge file operations to client (fs/read, fs/write)
5. Bridge terminal operations to client

**Files to Create**:
- `krusty-core/src/acp/tools.rs`
- `krusty-core/src/acp/handlers/tools.rs`

**Tests**:
- Tool call formatting
- Permission request flow
- File operation delegation

### Phase 5: CLI Entry Point (Binary)

**Objective**: Add ACP server mode to krusty-cli

**Tasks**:
1. Add `--acp` CLI flag for ACP mode
2. Implement ACP server main loop
3. Add graceful shutdown handling
4. Add logging in ACP mode (to stderr, not stdout)

**Files to Modify**:
- `krusty-cli/src/main.rs`
- `krusty-cli/src/app.rs` (conditional initialization)

**Tests**:
- CLI flag parsing
- ACP mode startup
- Graceful shutdown

### Phase 6: Integration Testing & Polish

**Objective**: End-to-end validation and documentation

**Tasks**:
1. Integration tests with mock client
2. Test with Zed editor
3. Test with Neovim ACP plugin
4. Add ACP documentation
5. Update README with ACP usage

**Files to Create**:
- `tests/acp_integration.rs`
- `docs/ACP_USAGE.md`

---

## 5. Detailed API Mapping

### 5.1 ACP Methods → Krusty Handlers

| ACP Method | Direction | Krusty Handler |
|------------|-----------|----------------|
| `initialize` | Client→Agent | `handlers::initialize::handle` |
| `authenticate` | Client→Agent | `handlers::authenticate::handle` |
| `session/new` | Client→Agent | `handlers::session::new_session` |
| `session/load` | Client→Agent | `handlers::session::load_session` |
| `session/prompt` | Client→Agent | `handlers::prompt::handle` |
| `session/set_mode` | Client→Agent | `handlers::session::set_mode` |
| `session/cancel` | Client→Agent | `handlers::cancel::handle` |
| `session/update` | Agent→Client | `updates::send_update` |
| `session/request_permission` | Agent→Client | `tools::request_permission` |
| `fs/read_text_file` | Agent→Client | `tools::read_file` |
| `fs/write_text_file` | Agent→Client | `tools::write_file` |
| `terminal/create` | Agent→Client | `tools::create_terminal` |
| `terminal/output` | Agent→Client | `tools::get_terminal_output` |
| `terminal/release` | Agent→Client | `tools::release_terminal` |

### 5.2 Krusty Tools → ACP Tool Calls

| Krusty Tool | ACP Delegation | Notes |
|-------------|----------------|-------|
| `read` | `fs/read_text_file` | Delegate to client |
| `write` | `fs/write_text_file` | Delegate to client |
| `edit` | `fs/write_text_file` | Read first, then write |
| `bash` | `terminal/create` + polling | Stream output via updates |
| `grep` | Local execution | Use agent's ripgrep |
| `glob` | Local execution | Use agent's glob |
| `explore` | Hybrid | Local + client fs |
| `ask_user` | `session/request_permission` | Map to permission flow |

---

## 6. Configuration

### 6.1 ACP-Specific Configuration

```toml
# ~/.krusty/config.toml

[acp]
# Enable ACP mode by default when stdin is not a TTY
auto_detect = true

# Protocol version to advertise
protocol_version = 10

# Supported session modes
modes = ["code", "architect", "ask"]

# Permission behavior
[acp.permissions]
# Auto-approve read operations
auto_approve_reads = false
# Auto-approve in specified directories
auto_approve_paths = []
```

### 6.2 Editor Integration Examples

**Zed** (`settings.json`):
```json
{
  "agent": {
    "default_agent": {
      "name": "krusty",
      "command": "krusty",
      "args": ["--acp"]
    }
  }
}
```

**Neovim** (ACP plugin):
```lua
require('acp').setup({
  agents = {
    krusty = {
      command = 'krusty',
      args = { '--acp' },
    },
  },
  default_agent = 'krusty',
})
```

---

## 7. Dependencies

### 7.1 New Crate Dependencies

```toml
# Cargo.toml additions for krusty-core

[dependencies]
# ACP SDK (official Rust implementation)
agent-client-protocol = "0.9"

# Already present, ensure versions compatible
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.40", features = ["full"] }
async-trait = "0.1"
```

### 7.2 Feature Flags

```toml
[features]
default = ["tui", "acp"]
tui = ["ratatui", "crossterm"]  # Terminal UI
acp = ["agent-client-protocol"]  # ACP support
```

---

## 8. Testing Strategy

### 8.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_version_negotiation() {
        let request = InitializeRequest {
            protocol_version: 10,
            capabilities: ClientCapabilities::default(),
            client_info: None,
        };

        let agent = KrustyAgent::new();
        let response = agent.initialize(request).await.unwrap();

        assert_eq!(response.protocol_version, PROTOCOL_VERSION);
    }

    #[test]
    fn test_session_creation() {
        let agent = KrustyAgent::new();
        agent.initialize(default_init()).await.unwrap();

        let response = agent.new_session(NewSessionRequest {
            cwd: Some("/tmp".into()),
            mcp_servers: None,
        }).await.unwrap();

        assert!(agent.sessions.contains_key(&response.session_id));
    }
}
```

### 8.2 Integration Tests

```rust
#[tokio::test]
async fn test_full_acp_conversation() {
    let (client, agent) = create_test_pair().await;

    // Initialize
    let init_response = client.initialize().await.unwrap();
    assert_eq!(init_response.agent_info.name, "krusty");

    // Create session
    let session = client.new_session(None, None).await.unwrap();

    // Send prompt
    let updates = client.prompt(session.id, "Hello").await;

    // Verify streaming updates
    assert!(updates.iter().any(|u| matches!(u, SessionUpdate::ContentChunk(_))));
}
```

---

## 9. Error Handling

### 9.1 ACP Error Codes

```rust
pub enum AcpError {
    /// JSON-RPC parse error (-32700)
    ParseError(String),
    /// Invalid request (-32600)
    InvalidRequest(String),
    /// Method not found (-32601)
    MethodNotFound(String),
    /// Invalid params (-32602)
    InvalidParams(String),
    /// Internal error (-32603)
    InternalError(String),
    /// Authentication required (-32000)
    AuthenticationRequired,
    /// Resource not found (-32002)
    ResourceNotFound(String),
    /// Request cancelled (by client)
    RequestCancelled,
}

impl From<AcpError> for JsonRpcError {
    fn from(err: AcpError) -> Self {
        match err {
            AcpError::ParseError(msg) => JsonRpcError {
                code: -32700,
                message: msg,
                data: None,
            },
            // ... other mappings
        }
    }
}
```

---

## 10. Security Considerations

### 10.1 Permission Model

- All file write operations require explicit permission
- Terminal commands require permission unless auto-approved
- MCP server connections inherit client's security context

### 10.2 Sandboxing

- ACP mode respects existing sandbox restrictions
- Working directory constrained to session's `cwd`
- Path traversal prevention maintained

### 10.3 Authentication

- Support API key authentication via `authenticate` method
- Credentials stored securely (existing mechanism)
- No credential exposure in logs or updates

---

## 11. Migration Path

### 11.1 Backward Compatibility

- Existing CLI/TUI mode unchanged
- ACP mode activated only via `--acp` flag
- Configuration files remain compatible

### 11.2 Feature Parity

| Feature | TUI Mode | ACP Mode |
|---------|----------|----------|
| Multi-provider AI | ✓ | ✓ |
| Tool execution | ✓ | ✓ (delegated) |
| MCP integration | ✓ | ✓ (pass-through) |
| Session persistence | ✓ | ✓ |
| Syntax highlighting | ✓ | N/A (client handles) |
| File watching | ✓ | N/A (client handles) |

---

## 12. Success Metrics

1. **Protocol Compliance**: Pass ACP conformance tests
2. **Editor Integration**: Work with Zed, Neovim, JetBrains
3. **Performance**: Sub-100ms response for non-LLM operations
4. **Reliability**: No crashes or hangs during normal operation
5. **Maintainability**: <10% increase in codebase complexity

---

## 13. References

- [ACP Official Documentation](https://agentclientprotocol.com/)
- [ACP GitHub Repository](https://github.com/agentclientprotocol/agent-client-protocol)
- [ACP Rust SDK](https://crates.io/crates/agent-client-protocol)
- [ACP JSON Schema](https://github.com/agentclientprotocol/agent-client-protocol/tree/main/schema)
- [Zed ACP Integration](https://zed.dev/acp)
- [MCP Specification](https://modelcontextprotocol.io/)

---

## Appendix A: Complete Type Definitions

See the official `agent-client-protocol-schema` crate for complete Rust type definitions:
- `InitializeRequest` / `InitializeResponse`
- `NewSessionRequest` / `NewSessionResponse`
- `PromptRequest` / `PromptResponse`
- `SessionUpdate` variants
- `ToolCallUpdate`
- `ContentBlock` variants
- `ClientCapabilities` / `AgentCapabilities`

---

*Document Version: 1.0*
*Last Updated: 2026-01-20*
*Author: Krusty Development Team*
