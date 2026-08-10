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
`MITSURO_NO_LIVE_TURN` disables Send; it does not select fixture replay.

## Production data invariant

Fixture records are allowed only when both the UI connection and active backend are
explicitly `fixture`. Mitsuro HTTP, Codex stdio/WebSocket, connecting, and error states
cannot use fixture catalogs or fixture process/file/session operations. Backend errors
clear the affected projection and render a typed loading, empty, unsupported, or error
state. They never substitute sample success.

Desktop preferences, current mode, local browser URL history, and transient composer UI
remain local application state. Server-owned sessions, transcripts, models, files,
processes, extensions, Hive runs/schedules, and account usage come from the selected
backend or are shown as unavailable.

## Truth matrix

| Surface | Mitsuro HTTP | Codex stdio | Fixture/static status |
|---|---|---|---|
| Health/connect | Live | Live | Explicit fixture |
| Sessions list/create/read/rename/delete | Live | Live | Typed fixture |
| Streaming chat | Live SSE | Live JSON-RPC notifications | Sample replay |
| Models | Live | Live | Typed fixture catalog |
| Interrupt | Live session cancel | Live turn interrupt | Typed fixture |
| Tool approval | Live | Live command, file, and exact-profile permission decisions | Sample approval |
| Archive/unarchive | Unsupported, capability-gated | Live | Typed fixture |
| Fork | Unsupported, capability-gated | Live | Typed fixture |
| Files | Live tree/read/fuzzy adapter | Live typed paths | Typed fixture |
| Processes | Read-only server catalog in client; interactive terminal spawn unsupported | Live spawn/stdin/PTY | Typed fixture |
| Extensions/MCP/skills | Live installed extensions, MCP status, and skills | Partially live | Typed fixture |
| Hive/schedules | Live read-only projections; mutations disabled | Unsupported | Typed fixture UI |
| Pull requests | No product adapter; explicit unavailable state | No product adapter; explicit unavailable state | No fake catalog |
| Sites | No product adapter; explicit unavailable state | No product adapter; explicit unavailable state | No fake catalog |
| Browser | System-browser bridge; no page ownership | System-browser bridge; no page ownership | Same local bridge |
| Computer environments/permissions | Unsupported; no invented rows or grants | Live environment APIs and exact requested permission grants | Explicit fixture catalog, labeled fixture |
| Settings writes | Desktop preferences persist locally; server config writes unsupported | Desktop preferences persist locally; server config writes unsupported | Same local persistence boundary |

Unsupported operations must return `NotImplemented` or be disabled through
`BackendCapabilities`. A method name appearing in the Codex inventory does not make it
implemented. The committed `codex-cli 0.147.0` baseline contains 95 stable client
methods, 133 methods with experimental APIs enabled, 70 server notifications, 10 server
requests in the stable contract (11 with experimental APIs), and 18 thread item
variants. The desktop automatically answers the experimental `currentTime/read` server
request. Every generated notification is classified as a core transcript event or a
typed lifecycle event. Every generated server request has an explicit disposition:
native approval/user-input/MCP interaction, a structured unsupported dynamic-tool
result, or an honest JSON-RPC error for unadvertised token/attestation capabilities.
The desktop negotiates experimental APIs because its process, environment, realtime,
and background-terminal surfaces require them. Fixture `call_raw` no longer manufactures
generic success payloads.

## Established recovery baseline

- GPUI and GPUI Component resolve from crates.io; there is no machine-specific
  `/home/.../vendor/gpui` patch.
- Codex notifications use an application-lifetime broadcast hub. Independent
  turn subscribers do not consume each other's events.
- Reopened and live Codex transcripts preserve every current thread item type. Tool,
  search, image, collaboration, review-mode, compaction, hook, and sleep items render as
  real activity rows; unknown future variants remain visible rather than disappearing.
- Server-originated requests are modeled separately from notifications because Codex
  can pause a turn until the client answers. The native interaction strip supports
  request-user-input options/freeform/secrets, standard MCP forms and URL elicitations,
  plus command/file/permission approvals without synthetic response data.
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
- The production-data purity matrix was revalidated on both live transports for Work,
  Scheduled, Computer, Extensions, Settings, and Files. A unit regression matrix forbids
  fixture records for every product backend connection state.
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
scripts/gpui-codex-protocol-check.sh

# Read-only check against a running local Mitsuro server
MITSURO_RUN_SERVER_IT=1 cargo test -p mitsuro-desktop-backend \
  live_server_read_only_contract -- --nocapture
```
