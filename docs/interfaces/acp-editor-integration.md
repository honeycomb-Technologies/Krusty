# Editor Integration (ACP)

Mitsuro can run inside your code editor. Instead of switching to a terminal or browser, you stay in your editor and talk to the same canonical agent loop and providers through a deliberately bounded editor-safe tool surface. The feature that makes this work is called ACP -- the Agent Client Protocol.

This document explains what ACP is, how Mitsuro implements it, and what happens under the hood when your editor spawns a Mitsuro process and starts sending it prompts.

## What Is ACP?

The Agent Client Protocol is a standardized way for code editors to communicate with AI agents. It defines JSON-RPC methods that an editor (the client) can call on an agent (the server), and notifications the agent can push back to the editor. The protocol covers session lifecycle, prompt processing, tool execution, model selection, and streaming responses.

Think of it like LSP, but for AI agents instead of language servers. LSP standardized how editors talk to code intelligence tools. ACP standardizes how editors talk to AI coding assistants. Any editor that speaks ACP can work with any agent that implements it.

Mitsuro implements the agent side. When you configure your editor to use Mitsuro, the editor spawns `mitsuro acp` as a subprocess. From that point on, the two communicate over stdin/stdout using JSON-RPC 2.0 messages.

## How the Connection Works

The transport layer is deliberately simple. The editor spawns Mitsuro as a child process:

```
mitsuro acp
```

Once running, Mitsuro takes over stdin and stdout for ACP communication. All diagnostic output goes to stderr so it doesn't interfere with the protocol messages. The editor writes JSON-RPC requests to Mitsuro's stdin. Mitsuro writes JSON-RPC responses and notifications to stdout. No TCP sockets, no HTTP, no WebSockets -- just piped stdio.

When the ACP server starts, it goes through a short initialization sequence:

1. **Tool registration.** Mitsuro registers its ACP-compatible tool set. This is a subset of the full tool catalog: path-scoped file operations, search, read-only web access, patching, and bounded deferred-tool discovery. Arbitrary host command execution and long-lived process management are excluded because an editor approval prompt is not an OS sandbox.

2. **Credential detection.** The server looks for API credentials in three places, checked in order: explicit environment variables (`MITSURO_PROVIDER` + `MITSURO_API_KEY`), provider-specific environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc.), and finally Mitsuro's stored credential file at `~/.mitsuro/tokens/credentials.json`. The first match wins.

3. **Waiting for the handshake.** The server creates a notification channel and waits for the editor to send the `initialize` request. This is where the two sides exchange capabilities and version information.

The connection stays alive until the editor closes the subprocess or the stdin pipe is closed. On disconnect, Mitsuro cleans up all active sessions.

## The MitsuroAgent

At the center of the ACP implementation is `MitsuroAgent`. This is a Rust struct that implements the `Agent` trait from the `agent_client_protocol` crate, which defines all the methods an ACP agent must support.

When the editor sends a JSON-RPC request, the ACP connection dispatches it to the corresponding method on `MitsuroAgent`. The main protocol methods are:

- **`initialize`** -- The editor introduces itself, sends its capabilities, and receives Mitsuro's capabilities in return. Mitsuro advertises that it supports embedded context (file content sent inline with prompts), session loading, and session modes.

- **`new_session`** -- The editor asks for a new conversation. It provides a working directory and optionally a list of MCP servers. Mitsuro creates the session, scans the workspace to build context for the AI, detects all available models from configured providers, and returns the session ID along with available modes and models.

- **`prompt`** -- The editor sends a user message. Mitsuro converts ACP content blocks to its internal format, runs the same `AgenticOrchestrator` used by the other interactive surfaces, and maps canonical loop events back to ACP notifications.

- **`cancel`** -- The editor wants to stop an in-progress prompt. Mitsuro sends cancellation through the active canonical loop input channel so provider streaming, tool work, and the turn lifecycle stop together.

- **`set_session_mode`** -- Switch between "code" mode (the AI writes and edits code directly) and "plan" mode (the AI designs changes before implementing them).

- **`set_session_model`** -- Switch to a different AI model mid-conversation. The agent looks up the model in its detected list, reconfigures the AI client, and continues.

- **`load_session`** -- Attempt to restore a previous session. Mitsuro checks in-memory sessions first, then falls back to SQLite storage to recover persisted conversation history and any interrupted-turn recovery state.

The agent also registers slash commands (`/compact`, `/clear`, `/help`, `/model`, `/mode`) that it pushes to the editor as available commands when a session starts. Editors can surface these in their UI for quick access.

## Session Management

Each editor conversation maps to a Mitsuro session. The `SessionManager` holds all active sessions in a concurrent `DashMap`, indexed by session ID.

A session (`SessionState`) tracks the working directory, conversation history, active loop input, selected model/client, work mode, MCP server configurations, and its SQLite storage identity. Prompt execution is serialized per session, while model and mode changes remain isolated from other editor sessions.

When storage is configured, every message is persisted to SQLite. If Mitsuro crashes or the editor restarts, `load_session` reconstructs the conversation from storage. The system also tracks recovery state for interrupted turns -- if the AI was mid-response when the connection dropped, a recovery notice is injected into the next prompt so the AI can pick up where it left off.

## The Notification Bridge

ACP is a streaming protocol. When the AI generates a response, the editor doesn't wait for the complete answer -- it receives chunks as they arrive. This is handled by the `NotificationBridge`.

The bridge implements the ACP `Client` trait using a bounded tokio channel (capacity 1000) to decouple the prompt processor from the transport layer. The processor pushes notifications into the channel, and a forwarding task sends them over the stdio connection.

Permission requests travel over the same bridge. A request carries a one-shot response channel, the server forwards it to the live editor connection, and the canonical loop waits for the editor's allow, reject, or cancel decision. A closed connection or timed-out permission request fails closed; it never becomes an implicit approval.

The bridge handles backpressure too. If the channel fills up, it waits up to 10 seconds before dropping the notification rather than blocking the processor.

## Tool Bridge

When the AI decides it needs to read a file, run a command, or search the codebase, it emits a tool call. The ACP tool bridge translates between Mitsuro's internal tool system and the ACP protocol's tool call format.

Each tool call goes through several steps:

1. **Classification.** The bridge maps tool names to ACP `ToolKind` categories (Read, Edit, Search, Execute, Fetch, Think, Delete, Move) so the editor can render appropriate UI.

2. **Location extraction.** For file-based tools, the bridge pulls out file paths and line numbers. The editor uses this for "follow-along" -- automatically opening or scrolling to the file being modified.

3. **Title generation.** Each tool call gets a human-readable title like "Reading server.rs" or "Searching for: authenticate".

4. **Approval and execution.** The canonical orchestrator applies the persisted supervised permission mode, relays any required decision to the editor, and executes an allowed tool with the session workspace as its path-policy root.

5. **Result streaming.** The bridge sends a start notification when the tool begins and a completion or failure notification with output when it finishes.

The protocol also defines client-side operations such as editor-buffer reads and terminal commands. Mitsuro does not currently delegate tool execution to those operations. Its ACP catalog therefore omits arbitrary Bash and process control instead of treating local path validation as host isolation.

## Workspace Context

When a new session starts, Mitsuro scans the editor's working directory and builds a workspace context string that gets injected as a system message. This gives the AI an immediate understanding of the project it's working in.

The context includes project type detection (checking for Cargo.toml, package.json, go.mod, etc.), a top-level directory listing (capped at 50 entries), and the current git branch if applicable.

The `WorkspaceContextBuilder` caches this with a 5-minute TTL so multiple sessions in the same workspace skip the filesystem scan. The scan runs in a blocking thread to avoid stalling the async runtime.

## Model Management

ACP mode supports dynamic model selection. When a session starts, Mitsuro probes all configured providers to discover available models:

1. Loads the credential store to find which providers have API keys.
2. For each provider, fetches its model catalog -- dynamically via API where supported, or from the built-in list otherwise.
3. Builds a unified list in the format `provider:model_id` (e.g., `anthropic:claude-sonnet-4-20250514`).
4. Returns this list in the `new_session` response so the editor can show a model picker.

The first detected model becomes the default. When the editor sends `set_session_model`, Mitsuro reconfigures the AI client (endpoint, auth headers, request format) and the next prompt uses the new model. Switching happens mid-conversation without losing history. The `ModelManager` caches provider configurations and can be invalidated when credentials change.

## Supported Editors

Mitsuro's ACP server works with any editor that implements the client side of the Agent Client Protocol. The currently supported editors are:

- **Zed** -- Native ACP support. Zed spawns `mitsuro acp` and surfaces the agent in its assistant panel. File mentions using `@` syntax send embedded resource content blocks that Mitsuro converts to formatted code blocks for the AI.
- **Neovim** -- Through ACP client plugins. The agent appears as a chat interface within Neovim.
- **JetBrains** -- IntelliJ, WebStorm, PyCharm, and the rest of the JetBrains family through their ACP integration.
- **Marimo** -- The Python notebook editor, which uses ACP for its AI assistance features.

Adding support for a new editor requires no changes to Mitsuro. Any editor that can spawn a subprocess and speak JSON-RPC over stdio can use Mitsuro as an ACP agent.

## Configuration

ACP mode is configured primarily through environment variables. The editor typically sets these before spawning the subprocess.

**Required (one of the following):**

| Variable | Description |
|----------|-------------|
| `MITSURO_PROVIDER` + `MITSURO_API_KEY` | Explicit provider and key. Provider values: `anthropic`, `openai`, `openrouter`, `minimax`, `zai` |
| `ANTHROPIC_API_KEY` | Direct Anthropic API key |
| `OPENAI_API_KEY` | Direct OpenAI API key |
| `OPENROUTER_API_KEY` | Direct OpenRouter API key |
| `MINIMAX_API_KEY` | Direct MiniMax API key |
| `ZAI_API_KEY` | Direct Z.ai API key |

If no environment variables are set, Mitsuro falls back to its stored credentials at `~/.mitsuro/tokens/credentials.json`. If you have already authenticated through the TUI or web interface, ACP mode will pick up those credentials automatically.

**Optional:**

| Variable | Description |
|----------|-------------|
| `MITSURO_MODEL` | Override the default model for the configured provider |

## How ACP Differs from the TUI and Server

All four of Mitsuro's interfaces -- TUI, web server, ACP, and Hive -- share the same core: the same AI provider layer, the same tools, and the same storage. The difference is how they interact with the user.

The TUI owns the terminal. The web server owns an HTTP port. ACP owns nothing -- it's a passive subprocess that responds to requests from the editor. The editor is the UI. Mitsuro just provides the brain.

This has a few practical consequences:

- **Tools are a subset.** ACP exposes a bounded file/search/web catalog and deferred discovery. Interactive terminal-only tools, arbitrary Bash, and process management are excluded.
- **Permissions are editor-mediated.** Supervised tool decisions are relayed to the editor. Disconnects, rejection, cancellation, and timeouts fail closed.
- **Streaming goes through notifications.** Instead of rendering text directly, Mitsuro pushes content chunks, tool call updates, and thought fragments as ACP session notifications. The editor decides how to display them.
- **Workspace context comes from the editor.** The editor tells Mitsuro what directory to work in and which MCP servers to connect to. In the TUI, you choose these yourself.

The core agent loop, conversation semantics, persistence, compaction, provider normalization, and work-mode lifecycle are the same. ACP intentionally changes only the transport and the tools that are safe to expose without an OS-isolated command runner.
