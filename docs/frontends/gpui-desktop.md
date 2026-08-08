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

Provider-backed turns require `MITSURO_ALLOW_LIVE_TURN=1`. Merely connecting and
discovering auth never authorizes a paid turn.

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
| Files/processes | Server contract exists; UI adapter incomplete | Live typed paths | Typed fixture |
| Extensions/MCP/skills | Server contract exists; UI adapter incomplete | Partially live | Typed fixture |
| Hive/schedules | Server contract exists; GPUI wiring incomplete | Unsupported | Static/demo UI |
| Pull requests | No product adapter | No product adapter | Static catalog |
| Sites | No product adapter | No product adapter | Static catalog |
| Browser/computer use | No production embed | No production embed | Mock host/catalog |
| Settings writes | Incomplete | Incomplete | Mostly local UI state |

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
- Backend session IDs have a namespaced type (`BackendSessionId`). Persisting the
  namespace in GPUI thread state is still required before mixed-backend lists are
  enabled.

## Resume here

The next implementation slice should be backend-capability completion, not more
visual polish:

1. Replace remaining Codex-shaped `AgentBackend` calls with product-domain session,
   model, turn, file, process, and extension contracts.
2. Persist `BackendSessionId` in GPUI thread state and settings so sessions remain
   attached to their originating backend.
3. Wire Mitsuro files, processes, extensions, skills, Hive, and schedules using the
   existing server/client contracts.
4. Add an authenticated Codex WebSocket adapter only if using the already-running
   app-server is a required deployment mode; managed stdio is working now.
5. Replace or remove PR, Sites, browser/computer, and settings demonstrations one
   surface at a time, with contract and GPUI interaction tests.
6. Split `app.rs` into state/controllers and bounded GPUI views after backend state
   stops moving.

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
