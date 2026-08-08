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

## Build

```bash
cargo check -p mitsuro-desktop-backend
cargo check -p mitsuro-gpui-desktop
cargo test -p mitsuro-desktop-backend
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
