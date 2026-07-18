# Krusty Server

Self-hosted API server used by the Expo mobile app, the embedded web client, and the desktop shell.

## Scope

- Session lifecycle and message history
- Agentic chat streaming (`/api/chat`, SSE)
- Tool execution, file APIs, process control
- Model listing and provider credential management
- MCP server management and user hooks

No Kubernetes-specific runtime is required for this phase.

## Run Locally

```bash
cargo run -p krusty-server
```

Default bind address: `0.0.0.0:3000`  
Health check: `GET /health`

## Configuration

- `PORT` (optional): server port (default `3000`)
- `KRUSTY_PROVIDER` (optional): `minimax`, `openai`, `openrouter`, `zai`
- `KRUSTY_MODEL` (optional): override default model for selected provider
- Provider API keys (optional): `MINIMAX_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `Z_AI_API_KEY`

Credentials can also be set through:

- `POST /api/credentials/:provider`

## Client Connection

- Expo web is exported and embedded into the server build for single-process web delivery.
- Desktop shell wraps the same Expo web bundle and therefore uses the same server API.
- Mobile clients connect directly to the same HTTP and WebSocket surface.

## Optional Loopback User Scoping

Requests without auth headers run in single-user local mode.

For local development, loopback clients connecting to a localhost host may send:

- `X-User-Id`
- `X-Workspace-Dir` (optional)

Remote access uses one server-wide bearer token and therefore remains
single-tenant. Remote requests that include either identity header are rejected:
the shared bearer token never authorizes a caller to select another user or
workspace. A multi-user deployment must put a real identity provider in front
of Krusty and bind verified principals through a dedicated server integration;
forwarding arbitrary client headers is not an authentication mechanism.

## Main API Groups

- `/api/sessions`
- `/api/chat`
- `/api/models`
- `/api/tools`
- `/api/files`
- `/api/credentials`
- `/api/mcp`
- `/api/processes`
- `/api/hooks`
- `/api/ports` (preview discovery + path proxy)
- `/api/settings/preview` (preview forwarding policy/preferences)
