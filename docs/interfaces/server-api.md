# The Web Server & API

Krusty ships a self-hosted web server built on Axum, Rust's async web framework. The server powers every frontend surface -- the embedded web client, the Expo mobile app, and the desktop shell -- through a single HTTP and WebSocket API. This document walks through the server's architecture, its major API groups, and the services that support them.

## Architecture

The server lives in `crates/krusty-server/`. It exposes an Axum `Router` assembled by the `build_router()` function in `lib.rs`. All shared state lives in a single `AppState` struct that gets cloned (cheaply, via `Arc`) into every request handler.

`AppState` holds everything the server needs at runtime: the AI client, a tool registry, credential store, model registry, MCP manager, process registry, session locks, push notification services, and the Mako runtime manager. Each field is wrapped in an `Arc` (and often an `RwLock`) so concurrent requests can share state safely without contention.

Routes are organized into a protected group and a small public surface. The protected group nests all `/api/*` endpoints and the WebSocket terminal handler behind authentication middleware. Outside that boundary sit the health check (`GET /health`), OAuth callbacks, and the static asset fallback that serves the web frontend.

The route tree is assembled in `routes/mod.rs`, which nests each API group under its own prefix:

- `/api/sessions` -- session lifecycle
- `/api/chat` -- agentic chat streaming
- `/api/models` -- model listing
- `/api/tools` -- tool registry and execution
- `/api/files` -- filesystem read/write/tree
- `/api/git` -- repository status, branches, worktrees
- `/api/credentials` -- provider API key management
- `/api/mako` -- autonomous agent dispatch
- `/api/mcp` -- MCP server management
- `/api/processes` -- user-scoped background process tracking and lifecycle status
- `/api/processes/:id/output` -- bounded recent stdout/stderr replay for a tracked process
- `/api/push` -- Web Push subscriptions
- `/api/apns` -- Apple Push Notification device management
- `/api/hooks` -- user-defined pre/post tool hooks
- `/api/server` -- server status and remote access config
- `/api/skills` -- skill registration and listing
- `/api/auth/oauth` -- OAuth flow management

## The Embedded Web Frontend

Krusty uses `rust-embed` to compile the Expo web build directly into the server binary. At compile time, the `WebAssets` struct includes all files from `apps/mobile/dist`. When a request doesn't match any API route, the `serve_web_app` fallback handler looks for a matching static asset. If none is found, it serves `index.html` for SPA client-side routing.

This means a production Krusty build is a single binary with no external files. Run `krusty serve` and every client surface is available immediately at `http://localhost:3000`. If the web build directory is absent at compile time (common during backend-only development), `rust-embed` gracefully produces an empty asset set and the server falls back to a plain-text message confirming the API is running.

Static assets get intelligent caching headers. Immutable bundled files (those under `_expo/static/`) receive a one-year `Cache-Control` with the `immutable` directive. HTML files are served with `no-cache` to ensure clients always get the latest shell. Everything else gets a one-hour cache.

## Chat API

The chat endpoint at `POST /api/chat` is the heart of the server. It accepts a JSON body with a message, optional session ID, model override, thinking level, content blocks (text and images), and permission mode. It returns a Server-Sent Events (SSE) stream.

When a request arrives, the handler either creates a new session or loads an existing one. It acquires a per-session mutex to prevent concurrent agentic loops on the same session -- if the session is already busy, the request gets a 409 Conflict response. The handler then resolves an AI client for the requested model, loads the conversation history from the database, and launches the `AgenticOrchestrator` from `krusty-core`.

The orchestrator runs an agentic loop: it sends messages to the AI provider, processes tool calls, and emits `LoopEvent`s through a channel. The chat handler translates these events into `AgenticEvent` variants and forwards them over the SSE stream.

### The Event Protocol

SSE events are JSON objects with a `type` field that tells the client what happened. The key event types are:

- `text_delta` -- incremental text from the AI response
- `thinking_delta` / `thinking_complete` -- extended reasoning content
- `tool_call_start` / `tool_call_complete` -- the AI wants to use a tool
- `tool_executing` / `tool_output_delta` / `tool_result` -- tool execution lifecycle
- `tool_approval_required` -- the tool needs user permission (supervised mode)
- `steering_injected` -- a durable user steering message was injected at a safe loop boundary
- `delegated_progress` -- live status from sub-agent runs (explore, plan, verify, build)
- `awaiting_input` -- the agent is asking the user a question
- `plan_update` / `plan_complete` -- plan lifecycle events
- `context_compacted` -- the conversation was summarized to fit within context limits
- `turn_complete` -- an agentic turn finished, possibly with more to come
- `finish` -- the loop ended, with a stop reason
- `title_update` -- an auto-generated session title
- `error` -- something went wrong

### Backpressure

The SSE channel uses a bounded buffer of 256 events. If the client falls behind, non-critical events (like text deltas) are dropped and a `lagged` event is sent to tell the client how many events it missed. Critical events like `awaiting_input`, `tool_approval_required`, and `finish` are always delivered -- these represent state transitions that the client must not miss.

Three companion endpoints complete the chat surface: `POST /api/chat/tool-result` lets the client submit tool results (or plan confirmation choices), `POST /api/chat/tool-approval` lets the client approve or deny tool calls in supervised mode, and `POST /api/chat/steer` durably queues a user message for an active run. Steering is ownership-checked and becomes canonical conversation history only when the orchestrator reaches a safe boundary; if the run rolls over first, the queued message is recovered by the next run without duplication.

## Session Management

Sessions are the primary organizational unit. Each session has an ID, title, working directory, project directory, workspace mode, model preference, work mode (plan/build), and optional target branch. The `/api/sessions` endpoints provide full CRUD.

Sessions support multi-tenant ownership. When auth headers are present, sessions are scoped to a user ID and the API enforces ownership on every operation. Foreign users get a 404, not a 403, to avoid leaking session existence.

Key session endpoints:

- `GET /api/sessions` -- list sessions, optionally filtered by working directory
- `POST /api/sessions` -- create a session with optional working directory, model, and session type (Code, Chat, or Mako)
- `GET /api/sessions/:id` -- get a session with its messages, supporting pagination via `limit` and `offset`
- `PATCH /api/sessions/:id` -- update title, working directory, mode, model, or target branch
- `DELETE /api/sessions/:id` -- delete a session and release its lock
- `GET /api/sessions/:id/state` -- get the live agent execution state (idle, streaming, tool_executing, awaiting_input)
- `GET /api/sessions/:id/trace` -- get the runtime trace summary and recent events
- `PUT /api/sessions/:id/presence` -- heartbeat for client presence tracking (viewer/controller)
- `POST /api/sessions/:id/cancel` -- idempotently signal cancellation to an active run
- `POST /api/sessions/:id/pinch` -- compact the current session in place with an AI-generated continuation summary

The pinch operation is noteworthy. It runs the same durable compaction pipeline used for automatic context pressure and provider-overflow recovery: old content is summarized, a recent verbatim tail is retained, and the session's messages are replaced atomically. It does not fork a child session, so the session ID, ownership, active plan, and client continuity remain unchanged.

## Tool Execution

The tool API at `/api/tools` has two endpoints. `GET /api/tools` lists all registered tools with their names and descriptions. `POST /api/tools/execute` runs a tool directly, bypassing the agentic loop.

Direct tool execution creates a `ToolContext` with the caller's working directory, owning user ID, process registry, MCP manager, and skills manager. Tools run in autonomous permission mode -- the API trusts authenticated callers. Path validation ensures the working directory stays within the user's allowed root, and process operations remain owner-scoped.

The allowed root is not a shell sandbox. A direct Bash call has the authority of the server's OS account, so this endpoint is suitable for trusted private deployments only. A public multi-tenant deployment must disable host execution or place it behind per-tenant OS/container isolation.

During normal chat flows, tools run inside the orchestrator loop rather than through this endpoint. The orchestrator handles the full lifecycle: the AI proposes a tool call, the server executes it (or asks for approval in supervised mode), and the result feeds back into the conversation.

## File and Git Endpoints

The file API at `/api/files` gives frontends direct filesystem access within security boundaries. `GET /api/files?path=...` reads a file. `PUT /api/files?path=...` writes content (up to 100MB). `GET /api/files/tree` returns a recursive directory listing with configurable depth, capped at 10,000 entries to prevent runaway scans. `GET /api/files/browse` lists directories for project selection, scoped to the user's home directory.

Every file operation validates that the resolved path stays within the user's allowed root. In multi-tenant mode, that root is the user's workspace directory. In single-tenant mode, it falls back to the system home directory. Path traversal attacks are blocked by canonicalization and prefix checking.

The git API at `/api/git` exposes repository operations. `GET /api/git/status` returns branch name, HEAD commit, upstream tracking, staged/modified/untracked counts, and diff statistics. `GET /api/git/branches` lists local and remote branches. `GET /api/git/worktrees` lists active worktrees. `POST /api/git/checkout` switches branches, optionally creating new ones. All git endpoints accept a `path` parameter to target repositories outside the default working directory.

## Push Notifications

The server supports two push notification channels: Web Push (VAPID) for browser clients and Apple Push Notifications (APNs) for iOS devices.

### Web Push

The `PushService` manages VAPID key generation, subscription storage, and notification delivery. On first run, it generates an ES256 keypair and saves it to disk. The public key is exposed at `GET /api/push/vapid-public-key` so browser clients can create push subscriptions.

Clients register via `POST /api/push/subscribe` with their endpoint URL, p256dh public key, and auth secret. The server stores subscriptions in SQLite, scoped by user ID. When the agentic loop completes or needs input, the server sends a Web Push notification to all of the user's subscriptions.

Delivery uses retry logic with exponential backoff (up to 3 attempts). Stale subscriptions (those returning 403, 404, or 410) are automatically cleaned up. Every delivery attempt is recorded for diagnostics, viewable via `GET /api/push/status`.

### Apple Push Notifications

The `ApnsService` uses JWT token-based authentication with an ES256 `.p8` key from Apple. Configuration comes from environment variables: `KRUSTY_APNS_KEY_PATH`, `KRUSTY_APNS_KEY_ID`, `KRUSTY_APNS_TEAM_ID`, and `KRUSTY_APNS_BUNDLE_ID`. The service caches JWT tokens for 50 minutes (Apple allows up to 60) and sends notifications through Apple's HTTP/2 API.

APNs supports event types including tool approval requests, completions, and Mako status updates. Device tokens are stored in SQLite, and devices that fail repeatedly (more than 10 consecutive failures) are automatically pruned.

## WebSocket Terminal

The server provides a browser-based terminal at `GET /ws/terminal`. The WebSocket handler spawns a real PTY using the `portable-pty` crate, running the user's default shell in the server's working directory.

The client communicates through a JSON message protocol. A `hello` message negotiates options (like binary output mode). `input` messages send keystrokes to the PTY. `resize` messages adjust the terminal dimensions (clamped to 500x500 maximum). `ping`/`pong` messages keep the connection alive.

PTY output flows back through the WebSocket, either as JSON `output` messages or raw binary frames depending on the negotiated mode. Output is coalesced over a 4ms window and batched up to 64KB to reduce WebSocket frame overhead during fast-scrolling output. The terminal session is registered in the process registry so it shows up alongside other tracked processes.

## Mako Runtime

Mako is Krusty's autonomous agent mode. While normal chat sessions are request-response (the user sends a message, the agent responds), Mako sessions run continuously in the background with full tool access.

The Mako API at `/api/mako` provides:

- `POST /api/mako/dispatch` -- start a new autonomous task with a description and optional project directory
- `GET /api/mako/sessions` -- list all Mako sessions with their runtime state
- `GET /api/mako/sessions/:id/status` -- detailed status including task list and agent state
- `GET /api/mako/sessions/:id/events` -- SSE stream of live events (with replay from persisted trace)
- `POST /api/mako/sessions/:id/message` -- inject a user message into a running session
- `POST /api/mako/sessions/:id/pause` / `POST .../resume` -- pause and resume execution
- `DELETE /api/mako/sessions/:id` -- cancel and delete a session

The `MakoRuntimeManager` owns the lifecycle of autonomous sessions. Each session gets its own tokio task running the orchestrator loop in `PermissionMode::Autonomous` with a `TickEngine` that injects synthetic ticks every 30 seconds to keep the agent working. Events flow through a broadcast channel so multiple clients can observe the same session.

Mako sessions persist their runtime state (running, sleeping, paused, error) to SQLite. On server restart, `restore_persisted_sessions` resumes any sessions that were running or sleeping. Sleeping sessions that have a future wake time get a scheduled timer; those past due resume immediately.

## Authentication

The auth middleware in `auth.rs` takes a layered approach. Local requests from loopback addresses to localhost are always allowed -- this is the default single-tenant mode. No API keys, no tokens, no friction.

Remote requests require a bearer token. On first startup, the server generates a random token (`kr_remote_<uuid>`) and stores it in the database. This token is shown in the server status API and can be rotated. Clients accessing the server over a network (including Tailscale) must include `Authorization: Bearer <token>` on every request.

Loopback development clients connecting to a localhost host may send
`X-User-Id` and optionally `X-Workspace-Dir`. The middleware resolves these
into an `AuthenticatedUser` for ownership checks and path scoping.

The remote-access token is a server-wide, single-tenant capability rather than
a per-user credential. Remote requests carrying either identity header are
rejected, even when the bearer token is valid, so a token holder cannot choose
another session owner. Deployments that need multiple remote users must add a
server-side identity-provider integration that binds each verified credential
to its principal; an untrusted forwarded header is insufficient.

## First-Run Setup

When you run `krusty serve` for the first time, the CLI checks for configured credentials. If none exist, it launches an interactive setup wizard that prompts for a provider selection and API key. The wizard saves credentials to the encrypted credential store so subsequent starts are immediate.

The serve command also integrates with Tailscale. If Tailscale is installed and the device is online, the server automatically configures `tailscale serve` to proxy the local port, making Krusty accessible at `https://<machine-name>.<tailnet>.ts.net`. If permissions are insufficient, it prints a one-time fix command (`sudo tailscale set --operator=$USER`). If Tailscale is not installed, it suggests installing it.

The server writes a PID file on startup and cleans it up on shutdown. If a server is already running on the same machine, `krusty serve` detects it, prints its URL, and exits rather than starting a duplicate instance.

The credential management API at `/api/credentials` provides runtime configuration without restarting. `GET /api/credentials` lists all providers with their configuration status. `POST /api/credentials/:provider` sets an API key and triggers a background model catalog refresh for providers that support dynamic model lists (OpenRouter, OpenAI). `DELETE /api/credentials/:provider` removes a provider's credentials.
