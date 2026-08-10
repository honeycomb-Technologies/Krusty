# GPUI desktop status and continuation

The GPUI desktop client is maintained in `apps/desktop/gpui` on the same Rust
workspace as the Mitsuro server and client contracts. The transport boundary is
`crates/mitsuro-desktop-backend`.

The former standalone prototype directory is a preserved local snapshot. Do not
resume implementation there. Its build target,
gauntlet reports, reverse-engineering dumps, and duplicate comparison captures
are not canonical product evidence.

## Backend selection

`MITSURO_BACKEND` accepts:

- `mitsuro-http` (default): `MITSURO_SERVER_URL`, default `http://127.0.0.1:3000`;
  optional `MITSURO_SERVER_TOKEN` for authenticated remote access.
- `codex-stdio`: managed `codex app-server --stdio`; optional `CODEX_BIN`.
- `fixture`: explicit deterministic UI development.
- `auto`: currently prefers Mitsuro HTTP. Probe/fallback policy still needs a
  product decision.
- `codex-ws`: reserved and rejected honestly until the WebSocket transport is
  implemented.

A Ready, authenticated product backend sends a provider-backed turn by default.
Use `MITSURO_NO_LIVE_TURN=1` for read-only UI review or an explicit fixture backend for
deterministic replay. Merely connecting and discovering auth does not itself send a turn.

## Truth matrix

| Surface | Mitsuro HTTP | Codex stdio | Fixture/static status |
|---|---|---|---|
| Health/connect | Live | Live | Explicit fixture |
| Sessions list/create/read/rename/delete | Live | Live | Typed fixture |
| Streaming chat | Live SSE | Live JSON-RPC notifications | Sample replay |
| Models | Live | Live | Typed fixture catalog |
| Interrupt | Live session cancel | Live turn interrupt | Typed fixture |
| Tool approval | Live | Live | Sample approval |
| Archive/unarchive | Unsupported, capability-gated | Live | Typed fixture |
| Fork | Unsupported, capability-gated | Live | Typed fixture |
| Files | Live tree/read/fuzzy adapter | Live typed paths | Typed fixture |
| Processes | Read-only server catalog in client; interactive terminal spawn unsupported | Live spawn/stdin/PTY | Typed fixture |
| Extensions/MCP/skills | Live installed extensions, MCP status, and skills | Partially live | Typed fixture |
| Hive/schedules | Live read-only projections; mutations disabled | Unsupported | Typed fixture UI |
| Pull requests | No product adapter; explicit unavailable state | No product adapter; explicit unavailable state | No fake catalog |
| Sites | No product adapter; explicit unavailable state | No product adapter; explicit unavailable state | No fake catalog |
| Browser/computer use | System-browser bridge; no page ownership | System-browser bridge; no page ownership | Explicit fixture catalog |
| Settings writes | Desktop preferences persist locally; server config writes unsupported | Desktop preferences persist locally; server config writes unsupported | Same local persistence boundary |

Unsupported operations must return `NotImplemented` or be disabled through
`BackendCapabilities`. A method name appearing in the 127-method Codex inventory
does not make it implemented. Fixture `call_raw` no longer manufactures generic
success payloads.

## Established recovery baseline

- GPUI and GPUI Component resolve from crates.io; there is no machine-specific
  `/home/.../vendor/gpui` patch.
- Codex notifications use an application-lifetime broadcast hub. Independent
  turn subscribers do not consume each other's events.
- Mitsuro uses the canonical `mitsuro-client` HTTP/SSE implementation.
- Thread reads preserve the canonical transcript rather than limiting history to
  eight 280-character bubbles.
- Backend session IDs are namespaced (`BackendSessionId`) and are stored on every
  live GPUI thread. Session/model/turn flows use the transport-neutral
  `ProductBackend` contract, and mutations reject a session whose origin differs
  from the active backend.
- The current UI still shows one selected backend at a time. A future mixed-backend
  list must use `BackendSessionId::qualified()` as its row/selection key instead of
  the raw server ID.

## Release posture

- The fixture/live visual matrix, dual-provider acceptance gauntlet, full workspace
  gates, optimized GPUI build, and isolated runtime provenance check are complete.
- Codex WebSocket stays explicitly unsupported unless attaching to an already-running
  app-server becomes a required deployment mode; managed stdio is the supported path.
- Installation and deployment are separate operator actions. A built GPUI artifact is
  not evidence that the installed Mitsuro CLI/server or any live process changed.
- Splitting `app.rs` into state/controllers and bounded views is a post-release
  maintainability slice and must not change the backend capability boundary.

## Validation

```bash
cargo check -p mitsuro-desktop-backend
cargo test -p mitsuro-client -p mitsuro-desktop-backend
cargo check -p mitsuro-gpui-desktop
cargo test -p mitsuro-gpui-desktop

# Read-only check against a running local Mitsuro server
MITSURO_RUN_SERVER_IT=1 cargo test -p mitsuro-desktop-backend \
  live_server_read_only_contract -- --nocapture
```
