# Mitsuro GPUI Desktop

Experimental native desktop client built with GPUI and GPUI Component.

This directory contains the maintained source imported from the earlier standalone
prototype. The original local directory remains an unchanged forensic snapshot;
generated targets, self-review reports, reverse-engineering dumps, and duplicate
screenshots were intentionally not imported.

## Product boundary

The desktop must support two explicit transports through normalized product concepts:

- Mitsuro server over HTTP and SSE, using `mitsuro-client`.
- Codex app-server over managed stdio, with WebSocket support tracked separately.

Fixture mode is for deterministic development and tests. A generic fixture success is
not evidence that a product feature works.

The selected transport and selected backend-qualified session are persisted in
`~/.mitsuro/gpui-desktop-state.json` (override with `MITSURO_GPUI_STATE_PATH`). An
explicit `MITSURO_BACKEND` always takes precedence. No server token or provider
credential is written to this file.

Settings → Connections can switch between the Mitsuro HTTP/SSE client and a managed
Codex app-server stdio child without restarting GPUI. Switching disconnects the prior
transport, ignores stale bootstrap results, and restores the last selected session for
each backend. Connection errors remain errors; they do not silently enable fixtures.

## Current live surfaces

- Sessions, models, turns, files, skills, MCP servers, and extensions use the
  transport-neutral `ProductBackend` contract.
- Mitsuro Work is a read-only projection of `/api/hive/current`; Hive dispatch and
  task mutations are deliberately unavailable in GPUI for now.
- Scheduled reads `/api/hive/schedules`; create, pause, resume, and delete are not
  exposed until their product interaction and approval semantics are designed.
- Terminal shows Mitsuro's `/api/processes` catalog read-only. Codex stdio retains
  interactive `process/*`; Mitsuro does not pretend its background-process API is a PTY.

## Parity status

The home shell and Connections settings have been compared in a live 940×1054 GPUI
window against the reversed ChatGPT desktop reference. This is not yet feature-complete
UI parity. Pull requests and Sites retain their navigation destinations but now render
explicit capability states: neither backend exposes a typed API for those products, so
the native client does not show sample repositories, sample deployments, or inactive
create/review controls. Parts of Atlas/browser and several secondary Settings actions
remain fixture-only or unavailable. Those surfaces must be wired or retired before the
native client can be called finalized.

## Build

```bash
cargo check -p mitsuro-desktop-backend
cargo check -p mitsuro-gpui-desktop
cargo test -p mitsuro-desktop-backend
cargo test -p mitsuro-client
cargo test -p mitsuro-gpui-desktop --no-default-features
```

Run against the local Mitsuro server without authorizing provider turns:

```bash
MITSURO_BACKEND=mitsuro-http MITSURO_NO_LIVE_TURN=1 \
  cargo run -p mitsuro-gpui-desktop
```

Use `MITSURO_BACKEND=codex-stdio` for a managed Codex app-server child. Set
`MITSURO_ALLOW_LIVE_TURN=1` only when a provider-backed turn is intended.

The optional `browser-native` feature links Wry/WebKitGTK. The default build uses the
external-browser bridge only while native embedding remains incomplete.
